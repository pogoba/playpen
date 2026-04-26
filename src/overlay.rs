//! Persistent named overlayfs sandboxes.
//!
//! Layout under `$XDG_RUNTIME_DIR/playpen/overlays/<NAME>/`:
//!
//!   overlayfs-upper/   writeable upper layer
//!   overlayfs-work/    overlayfs workdir
//!   overlayfs/         mountpoint of the merged view
//!   pids/<pid>         one empty file per active bwrap session
//!   env-<pid>          captured caller env (sourced via BASH_ENV)
//!
//! `enter` mounts the overlay once (idempotent) and bind-mounts it as `/`
//! inside `bwrap`, so multiple sessions can attach to the same NAME and
//! see each other's writes live. `list` enumerates overlays and live
//! pids. `kill` SIGTERMs the participants, unmounts, and removes the
//! NAME directory.

use clap::Args;
use std::error::Error;
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const OVERLAYFS_SUPER_MAGIC: i64 = 0x794c7630;
const PLAYPEN_SUBDIR: &str = "playpen/overlays";

#[derive(Args)]
#[command(trailing_var_arg = true)]
pub struct EnterArgs {
    /// Overlay name (must match `[A-Za-z0-9_-]+`).
    pub name: String,
    /// Command and arguments to run inside the sandbox; defaults to
    /// `$SHELL` when omitted.
    pub cmd: Vec<String>,
}

#[derive(Args)]
pub struct KillArgs {
    /// Overlay name to tear down.
    pub name: String,
}

fn xdg_runtime_dir() -> Result<PathBuf, Box<dyn Error>> {
    let v = std::env::var("XDG_RUNTIME_DIR")
        .map_err(|_| "XDG_RUNTIME_DIR is not set")?;
    if v.is_empty() {
        return Err("XDG_RUNTIME_DIR is empty".into());
    }
    Ok(PathBuf::from(v))
}

fn overlays_root() -> Result<PathBuf, Box<dyn Error>> {
    Ok(xdg_runtime_dir()?.join(PLAYPEN_SUBDIR))
}

fn overlay_dir(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    Ok(overlays_root()?.join(name))
}

fn validate_name(name: &str) -> Result<(), Box<dyn Error>> {
    if name.is_empty() {
        return Err("overlay name must not be empty".into());
    }
    let ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(format!(
            "invalid overlay name {:?}: only A-Z, a-z, 0-9, '-', '_' allowed",
            name
        )
        .into());
    }
    Ok(())
}

fn is_overlay_mounted(path: &Path) -> io::Result<bool> {
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut buf = unsafe { std::mem::zeroed::<libc::statfs>() };
    let r = unsafe { libc::statfs(c.as_ptr(), &mut buf) };
    if r != 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::NotFound {
            return Ok(false);
        }
        return Err(err);
    }
    Ok(buf.f_type as i64 == OVERLAYFS_SUPER_MAGIC)
}

fn ensure_dirs(name: &str) -> Result<(), Box<dyn Error>> {
    let dir = overlay_dir(name)?;
    fs::create_dir_all(dir.join("overlayfs-upper"))?;
    fs::create_dir_all(dir.join("overlayfs-work"))?;
    fs::create_dir_all(dir.join("overlayfs"))?;
    fs::create_dir_all(dir.join("pids"))?;
    Ok(())
}

fn ensure_mounted(name: &str) -> Result<(), Box<dyn Error>> {
    ensure_dirs(name)?;
    let dir = overlay_dir(name)?;
    let mountpoint = dir.join("overlayfs");
    if is_overlay_mounted(&mountpoint)? {
        return Ok(());
    }
    let opts = format!(
        "lowerdir=/,upperdir={},workdir={}",
        dir.join("overlayfs-upper").display(),
        dir.join("overlayfs-work").display(),
    );
    let status = Command::new("sudo")
        .args(["mount", "-t", "overlay", "overlay", "-o"])
        .arg(&opts)
        .arg(&mountpoint)
        .status()?;
    if !status.success() {
        return Err(format!(
            "sudo mount overlayfs at {} failed",
            mountpoint.display()
        )
        .into());
    }
    Ok(())
}

fn write_env_file(path: &Path, overlay_name: &str) -> Result<(), Box<dyn Error>> {
    // Mirror nonowrap: capture `export -p` so the inner bash can re-source
    // the caller's env via BASH_ENV. Override `$name` so PS1 reflects the
    // sandbox (most distro prompts read $name when set).
    let out = Command::new("bash")
        .env("name", format!("pp:{}", overlay_name))
        .args(["-c", "export -p"])
        .output()?;
    if !out.status.success() {
        return Err("bash -c 'export -p' failed".into());
    }
    let mut f = File::create(path)?;
    f.write_all(b"# playpen-managed env capture\n")?;
    f.write_all(&out.stdout)?;
    Ok(())
}

fn pid_alive(pid: u32) -> bool {
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if r == 0 {
        return true;
    }
    // EPERM means the process exists but is owned by someone else
    // (here: root, since bwrap is spawned via sudo).
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn live_pids(name: &str) -> Result<Vec<u32>, Box<dyn Error>> {
    let pids_dir = overlay_dir(name)?.join("pids");
    let mut out = Vec::new();
    if !pids_dir.exists() {
        return Ok(out);
    }
    for ent in fs::read_dir(&pids_dir)? {
        let ent = ent?;
        let fname = ent.file_name();
        let s = match fname.to_str() {
            Some(s) => s,
            None => continue,
        };
        let pid: u32 = match s.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        if pid_alive(pid) {
            out.push(pid);
        } else {
            let _ = fs::remove_file(ent.path());
        }
    }
    out.sort_unstable();
    Ok(out)
}

/// Removes the listed paths on drop. Used to scrub the per-session
/// pidfile and env file even on early returns / panics.
struct CleanupGuard {
    paths: Vec<PathBuf>,
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        for p in self.paths.drain(..) {
            let _ = fs::remove_file(p);
        }
    }
}

pub fn enter(args: EnterArgs) -> Result<(), Box<dyn Error>> {
    validate_name(&args.name)?;

    let dir = overlay_dir(&args.name)?;
    let own_pid = std::process::id();
    let xdg = xdg_runtime_dir()?;
    let uid = unsafe { libc::getuid() }.to_string();
    let gid = unsafe { libc::getgid() }.to_string();
    let mountpoint = dir.join("overlayfs");

    // create overlayfs if needed
    ensure_mounted(&args.name)?;

    // record user environment to restore inside the sandbox
    let env_file_host = dir.join(format!("env-{}", own_pid));
    write_env_file(&env_file_host, &args.name)?;
    // The env file lives under $XDG_RUNTIME_DIR, which is different inside the
    // sandbox — we will bind a copy at a fixed path so BASH_ENV can find it.
    let env_file_sandbox = format!("/tmp/playpen-env-{}", own_pid);

    // determine what to run inside the sandbox
    let target_cmd: Vec<String> = if args.cmd.is_empty() {
        let shell = std::env::var("SHELL")
            .map_err(|_| "$SHELL is not set; pass a command after the overlay name")?;
        vec![shell]
    } else {
        args.cmd.clone()
    };

    // start command in sandbox
    let mut cmd = Command::new("sudo");
    cmd.arg("bwrap")
        .arg("--bind")
        .arg(&mountpoint)
        .arg("/")
        .arg("--ro-bind")
        .arg("/run/current-system")
        .arg("/run/current-system")
        .arg("--bind")
        .arg("/nix/var/nix/daemon-socket")
        .arg("/nix/var/nix/daemon-socket")
        .arg("--dir")
        .arg("/run/systemd/resolve")
        .arg("--ro-bind")
        .arg("/run/systemd/resolve/stub-resolv.conf")
        .arg("/run/systemd/resolve/stub-resolv.conf")
        .arg("--ro-bind")
        .arg("/run/nscd")
        .arg("/run/nscd")
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--tmpfs")
        .arg(&xdg)
        .arg("--ro-bind")
        .arg(&env_file_host)
        .arg(&env_file_sandbox)
        .arg("--setenv")
        .arg("BASH_ENV")
        .arg(&env_file_sandbox)
        .arg("--die-with-parent")
        .arg("--")
        .arg("setpriv")
        .arg(format!("--reuid={}", uid))
        .arg(format!("--regid={}", gid))
        .arg("--init-groups")
        .arg("--inh-caps=-all")
        .arg("--")
        .arg("bash")
        .arg("-c")
        .arg(r#"unset BASH_ENV; exec "$@""#)
        .arg("_");
    for arg in &target_cmd {
        cmd.arg(arg);
    }

    let mut child = cmd.spawn().map_err(|e| {
        format!("failed to spawn `sudo bwrap …`: {} (is bwrap installed?)", e)
    })?;
    let child_pid = child.id();
    let pidfile = dir.join("pids").join(child_pid.to_string());
    File::create(&pidfile)?;

    let _guard = CleanupGuard {
        paths: vec![pidfile, env_file_host],
    };

    let status = child.wait()?;
    if let Some(code) = status.code() {
        std::process::exit(code);
    }
    if let Some(sig) = status.signal() {
        std::process::exit(128 + sig);
    }
    Ok(())
}

pub fn list() -> Result<(), Box<dyn Error>> {
    let root = overlays_root()?;
    if !root.exists() {
        println!("(no overlays)");
        return Ok(());
    }
    let mut names: Vec<String> = fs::read_dir(&root)?
        .filter_map(|r| r.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    if names.is_empty() {
        println!("(no overlays)");
        return Ok(());
    }
    let width = names.iter().map(|n| n.len()).max().unwrap_or(4).max(4);
    println!("{:<width$}  PIDS", "NAME", width = width);
    for name in &names {
        let pids = live_pids(name)?;
        let pidlist = if pids.is_empty() {
            "(none)".to_string()
        } else {
            pids.iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!("{:<width$}  {}", name, pidlist, width = width);
    }
    Ok(())
}

pub fn kill(args: KillArgs) -> Result<(), Box<dyn Error>> {
    validate_name(&args.name)?;
    let dir = overlay_dir(&args.name)?;
    if !dir.exists() {
        return Err(format!("overlay {:?} does not exist", args.name).into());
    }
    let mountpoint = dir.join("overlayfs");

    let pids = live_pids(&args.name)?;
    if !pids.is_empty() {
        let pid_args: Vec<String> = pids.iter().map(|p| p.to_string()).collect();
        let _ = Command::new("sudo")
            .arg("kill")
            .arg("-TERM")
            .args(&pid_args)
            .status();

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let remaining = live_pids(&args.name)?;
            if remaining.is_empty() {
                break;
            }
            if Instant::now() >= deadline {
                let stragglers: Vec<String> =
                    remaining.iter().map(|p| p.to_string()).collect();
                let _ = Command::new("sudo")
                    .arg("kill")
                    .arg("-KILL")
                    .args(&stragglers)
                    .status();
                break;
            }
        }
    }

    if is_overlay_mounted(&mountpoint).unwrap_or(false) {
        let st = Command::new("sudo")
            .arg("umount")
            .arg(&mountpoint)
            .status()?;
        if !st.success() {
            return Err(
                format!("sudo umount {} failed", mountpoint.display()).into(),
            );
        }
    }

    let st = Command::new("sudo")
        .arg("rm")
        .arg("-rf")
        .arg(&dir)
        .status()?;
    if !st.success() {
        return Err(format!("sudo rm -rf {} failed", dir.display()).into());
    }

    println!("killed overlay {:?}", args.name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation_accepts_simple_names() {
        for n in ["foo", "Foo-1", "abc_def", "x", "0", "a-b_c-1"] {
            assert!(validate_name(n).is_ok(), "rejected {n:?}");
        }
    }

    #[test]
    fn name_validation_rejects_bad_names() {
        for n in ["", "foo/bar", "..", ".", "foo bar", "foo.bar", "a/b/c"] {
            assert!(validate_name(n).is_err(), "accepted {n:?}");
        }
    }

    #[test]
    fn live_pids_skips_stale_entries() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("playpen-test-{}-{}", std::process::id(), nanos));
        let xdg = tmp.join("xdg");
        let pids = xdg.join(PLAYPEN_SUBDIR).join("foo").join("pids");
        fs::create_dir_all(&pids).unwrap();
        // Live: our own pid. Stale: pid 999999 (extremely unlikely to exist).
        let own = std::process::id();
        File::create(pids.join(own.to_string())).unwrap();
        File::create(pids.join("999999")).unwrap();
        File::create(pids.join("not-a-pid")).unwrap();

        // SAFETY: tests run single-threaded for env mutation here.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &xdg) };
        let result = live_pids("foo").unwrap();

        assert_eq!(result, vec![own]);
        assert!(!pids.join("999999").exists(), "stale pid 999999 not reaped");
        // Bogus filename is left alone (treated as not-a-pid, ignored).
        assert!(pids.join("not-a-pid").exists());

        let _ = fs::remove_dir_all(&tmp);
    }
}
