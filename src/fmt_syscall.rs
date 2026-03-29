use std::io;

pub(crate) fn read_cstring_from_pid(pid: libc::pid_t, addr: u64) -> io::Result<String> {
    if addr == 0 {
        return Ok("<null>".to_string());
    }

    let mut buf = Vec::new();
    let mut offset = 0usize;
    let mut chunk = [0u8; 256];

    while buf.len() < 0x10000 {
        let local = libc::iovec {
            iov_base: chunk.as_mut_ptr() as *mut libc::c_void,
            iov_len: chunk.len(),
        };
        let remote = libc::iovec {
            iov_base: (addr as usize + offset) as *mut libc::c_void,
            iov_len: chunk.len(),
        };

        let read = unsafe {
            libc::process_vm_readv(
                pid,
                &local as *const libc::iovec,
                1,
                &remote as *const libc::iovec,
                1,
                0,
            )
        };
        if read <= 0 {
            return Err(io::Error::last_os_error());
        }

        let read = read as usize;
        if let Some(pos) = chunk[..read].iter().position(|b| *b == 0) {
            buf.extend_from_slice(&chunk[..pos]);
            break;
        }

        buf.extend_from_slice(&chunk[..read]);
        offset += read;
    }

    Ok(String::from_utf8_lossy(&buf).to_string())
}

pub(crate) fn format_open_flags(flags: u64) -> String {
    let mut parts: Vec<&'static str> = Vec::new();
    let accmode = flags as libc::c_int & libc::O_ACCMODE;
    match accmode {
        libc::O_RDONLY => parts.push("O_RDONLY"),
        libc::O_WRONLY => parts.push("O_WRONLY"),
        libc::O_RDWR => parts.push("O_RDWR"),
        _ => parts.push("O_ACCMODE?"),
    }

    let flag_map: &[(libc::c_int, &str)] = &[
        (libc::O_APPEND, "O_APPEND"),
        (libc::O_CLOEXEC, "O_CLOEXEC"),
        (libc::O_CREAT, "O_CREAT"),
        (libc::O_DIRECTORY, "O_DIRECTORY"),
        (libc::O_EXCL, "O_EXCL"),
        (libc::O_NOFOLLOW, "O_NOFOLLOW"),
        (libc::O_TRUNC, "O_TRUNC"),
    ];

    for (flag, name) in flag_map {
        if (flags as libc::c_int & *flag) != 0 {
            parts.push(*name);
        }
    }

    #[cfg(target_os = "linux")]
    {
        if (flags as libc::c_int & libc::O_TMPFILE) != 0 {
            parts.push("O_TMPFILE");
        }
    }

    parts.join("|")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read_path(pid: libc::pid_t, addr: u64) -> String {
    match read_cstring_from_pid(pid, addr) {
        Ok(s) => s,
        Err(_) => "<unreadable>".to_string(),
    }
}

fn format_dirfd(fd: u64) -> String {
    if fd as libc::c_int == libc::AT_FDCWD {
        "AT_FDCWD".to_string()
    } else {
        format!("{}", fd)
    }
}

fn format_mode(mode: u64) -> String {
    format!("{:#05o}", mode & 0o7777)
}

// ---------------------------------------------------------------------------
// CHOWN class
// ---------------------------------------------------------------------------

// chown(path, owner, group)
fn fmt_chown_path(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("path", read_path(pid, args[0])),
        ("owner", format!("{}", args[1] as u32)),
        ("group", format!("{}", args[2] as u32)),
    ]
}

// fchown(fd, owner, group)
fn fmt_fchown(args: [u64; 6], _pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("fd", format!("{}", args[0])),
        ("owner", format!("{}", args[1] as u32)),
        ("group", format!("{}", args[2] as u32)),
    ]
}

// fchownat(dirfd, path, owner, group, flags)
fn fmt_fchownat(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("dirfd", format_dirfd(args[0])),
        ("path", read_path(pid, args[1])),
        ("owner", format!("{}", args[2] as u32)),
        ("group", format!("{}", args[3] as u32)),
    ]
}

// ---------------------------------------------------------------------------
// FILE_SYSTEM class
// ---------------------------------------------------------------------------

// Syscalls taking a single path: stat, lstat, chdir, rmdir, unlink, readlink, access
fn fmt_path_only(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![("path", read_path(pid, args[0]))]
}

// open(path, flags, mode)
fn fmt_open(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    let mut v = vec![
        ("path", read_path(pid, args[0])),
        ("flags", format_open_flags(args[1])),
    ];
    if (args[1] as libc::c_int & libc::O_CREAT) != 0 {
        v.push(("mode", format_mode(args[2])));
    }
    v
}

// creat(path, mode)
fn fmt_creat(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("path", read_path(pid, args[0])),
        ("mode", format_mode(args[1])),
    ]
}

// openat(dirfd, path, flags, mode)
fn fmt_openat(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    let mut v = vec![
        ("dirfd", format_dirfd(args[0])),
        ("path", read_path(pid, args[1])),
        ("flags", format_open_flags(args[2])),
    ];
    if (args[2] as libc::c_int & libc::O_CREAT) != 0 {
        v.push(("mode", format_mode(args[3])));
    }
    v
}

// openat2(dirfd, path, how, size) — how is a pointer to struct open_how, just show dirfd+path
fn fmt_openat2(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("dirfd", format_dirfd(args[0])),
        ("path", read_path(pid, args[1])),
    ]
}

// *at(dirfd, path) — unlinkat, readlinkat, faccessat, faccessat2, fchdir uses fd only
fn fmt_dirfd_path(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("dirfd", format_dirfd(args[0])),
        ("path", read_path(pid, args[1])),
    ]
}

// fstat/fchdir/ftruncate(fd, ...)
fn fmt_fd_only(args: [u64; 6], _pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![("fd", format!("{}", args[0]))]
}

// mkdir(path, mode)
fn fmt_mkdir(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("path", read_path(pid, args[0])),
        ("mode", format_mode(args[1])),
    ]
}

// mkdirat(dirfd, path, mode)
fn fmt_mkdirat(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("dirfd", format_dirfd(args[0])),
        ("path", read_path(pid, args[1])),
        ("mode", format_mode(args[2])),
    ]
}

// mknod(path, mode, dev)
fn fmt_mknod(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("path", read_path(pid, args[0])),
        ("mode", format_mode(args[1])),
    ]
}

// mknodat(dirfd, path, mode, dev)
fn fmt_mknodat(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("dirfd", format_dirfd(args[0])),
        ("path", read_path(pid, args[1])),
        ("mode", format_mode(args[2])),
    ]
}

// rename(old, new)
fn fmt_rename(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("from", read_path(pid, args[0])),
        ("to", read_path(pid, args[1])),
    ]
}

// renameat(olddirfd, old, newdirfd, new)
fn fmt_renameat(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("old_dirfd", format_dirfd(args[0])),
        ("from", read_path(pid, args[1])),
        ("new_dirfd", format_dirfd(args[2])),
        ("to", read_path(pid, args[3])),
    ]
}

// symlink(target, linkpath)
fn fmt_symlink(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("target", read_path(pid, args[0])),
        ("linkpath", read_path(pid, args[1])),
    ]
}

// symlinkat(target, newdirfd, linkpath)
fn fmt_symlinkat(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("target", read_path(pid, args[0])),
        ("dirfd", format_dirfd(args[1])),
        ("linkpath", read_path(pid, args[2])),
    ]
}

// link(oldpath, newpath)
fn fmt_link(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("oldpath", read_path(pid, args[0])),
        ("newpath", read_path(pid, args[1])),
    ]
}

// linkat(olddirfd, oldpath, newdirfd, newpath, flags)
fn fmt_linkat(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("old_dirfd", format_dirfd(args[0])),
        ("oldpath", read_path(pid, args[1])),
        ("new_dirfd", format_dirfd(args[2])),
        ("newpath", read_path(pid, args[3])),
    ]
}

// chmod(path, mode)
fn fmt_chmod(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("path", read_path(pid, args[0])),
        ("mode", format_mode(args[1])),
    ]
}

// fchmod(fd, mode)
fn fmt_fchmod(args: [u64; 6], _pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("fd", format!("{}", args[0])),
        ("mode", format_mode(args[1])),
    ]
}

// fchmodat(dirfd, path, mode, flags)
fn fmt_fchmodat(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("dirfd", format_dirfd(args[0])),
        ("path", read_path(pid, args[1])),
        ("mode", format_mode(args[2])),
    ]
}

// truncate(path, length)
fn fmt_truncate(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("path", read_path(pid, args[0])),
        ("length", format!("{}", args[1])),
    ]
}

// readlinkat(dirfd, path, buf, bufsiz)
fn fmt_readlinkat(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("dirfd", format_dirfd(args[0])),
        ("path", read_path(pid, args[1])),
    ]
}

// ---------------------------------------------------------------------------
// KEYRING class
// ---------------------------------------------------------------------------

// add_key(type, description, payload, plen, keyring)
fn fmt_add_key(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("type", read_path(pid, args[0])),
        ("description", read_path(pid, args[1])),
    ]
}

// request_key(type, description, callout_info, dest_keyring)
fn fmt_request_key(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("type", read_path(pid, args[0])),
        ("description", read_path(pid, args[1])),
    ]
}

// keyctl(operation, ...) — just show the operation number
fn fmt_keyctl(args: [u64; 6], _pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![("operation", format!("{}", args[0]))]
}

// ---------------------------------------------------------------------------
// MODULE class
// ---------------------------------------------------------------------------

// init_module(module_image, len, param_values)
fn fmt_init_module(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("image_addr", format!("{:#x}", args[0])),
        ("len", format!("{}", args[1])),
        ("params", read_path(pid, args[2])),
    ]
}

// finit_module(fd, param_values, flags)
fn fmt_finit_module(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("fd", format!("{}", args[0])),
        ("params", read_path(pid, args[1])),
    ]
}

// delete_module(name, flags)
fn fmt_delete_module(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![("name", read_path(pid, args[0]))]
}

// ---------------------------------------------------------------------------
// MOUNT class
// ---------------------------------------------------------------------------

// mount(source, target, filesystemtype, mountflags, data)
fn fmt_mount(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("source", read_path(pid, args[0])),
        ("target", read_path(pid, args[1])),
        ("fstype", read_path(pid, args[2])),
        ("flags", format_mount_flags(args[3])),
    ]
}

fn format_mount_flags(flags: u64) -> String {
    let flag_map: &[(libc::c_ulong, &str)] = &[
        (libc::MS_RDONLY, "MS_RDONLY"),
        (libc::MS_NOSUID, "MS_NOSUID"),
        (libc::MS_NODEV, "MS_NODEV"),
        (libc::MS_NOEXEC, "MS_NOEXEC"),
        (libc::MS_REMOUNT, "MS_REMOUNT"),
        (libc::MS_BIND, "MS_BIND"),
        (libc::MS_MOVE, "MS_MOVE"),
        (libc::MS_NOATIME, "MS_NOATIME"),
    ];
    let mut parts = Vec::new();
    for (flag, name) in flag_map {
        if (flags as libc::c_ulong & *flag) != 0 {
            parts.push(*name);
        }
    }
    if parts.is_empty() {
        "0".to_string()
    } else {
        parts.join("|")
    }
}

// umount2(target, flags)
fn fmt_umount2(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![("target", read_path(pid, args[0]))]
}

// chroot(path)
fn fmt_chroot(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![("path", read_path(pid, args[0]))]
}

// pivot_root(new_root, put_old)
fn fmt_pivot_root(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("new_root", read_path(pid, args[0])),
        ("put_old", read_path(pid, args[1])),
    ]
}

// fsopen(fs_name, flags)
fn fmt_fsopen(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![("fstype", read_path(pid, args[0]))]
}

// fsmount(fs_fd, flags, attr_flags)
fn fmt_fsmount(args: [u64; 6], _pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![("fs_fd", format!("{}", args[0]))]
}

// move_mount(from_dirfd, from_path, to_dirfd, to_path, flags)
fn fmt_move_mount(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("from_dirfd", format_dirfd(args[0])),
        ("from_path", read_path(pid, args[1])),
        ("to_dirfd", format_dirfd(args[2])),
        ("to_path", read_path(pid, args[3])),
    ]
}

// fsconfig(fd, cmd, key, value, aux)
fn fmt_fsconfig(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    let mut v = vec![
        ("fd", format!("{}", args[0])),
        ("cmd", format!("{}", args[1])),
    ];
    if args[2] != 0 {
        v.push(("key", read_path(pid, args[2])));
    }
    v
}

// fspick(dirfd, path, flags)
fn fmt_fspick(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("dirfd", format_dirfd(args[0])),
        ("path", read_path(pid, args[1])),
    ]
}

// mount_setattr(dirfd, path, flags, uattr, usize)
fn fmt_mount_setattr(args: [u64; 6], pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![
        ("dirfd", format_dirfd(args[0])),
        ("path", read_path(pid, args[1])),
    ]
}

// ---------------------------------------------------------------------------
// SETUID class
// ---------------------------------------------------------------------------

// setuid(uid) / setgid(gid)
fn fmt_set_id(label: &'static str, args: [u64; 6]) -> Vec<(&'static str, String)> {
    vec![(label, format!("{}", args[0] as u32))]
}

// setreuid(ruid, euid) / setregid(rgid, egid)
fn fmt_set_re_id(
    real_label: &'static str,
    eff_label: &'static str,
    args: [u64; 6],
) -> Vec<(&'static str, String)> {
    vec![
        (real_label, format!("{}", args[0] as u32)),
        (eff_label, format!("{}", args[1] as u32)),
    ]
}

// setresuid(ruid, euid, suid) / setresgid(rgid, egid, sgid)
fn fmt_set_res_id(
    real_label: &'static str,
    eff_label: &'static str,
    saved_label: &'static str,
    args: [u64; 6],
) -> Vec<(&'static str, String)> {
    vec![
        (real_label, format!("{}", args[0] as u32)),
        (eff_label, format!("{}", args[1] as u32)),
        (saved_label, format!("{}", args[2] as u32)),
    ]
}

// setgroups(size, list)
fn fmt_setgroups(args: [u64; 6], _pid: libc::pid_t) -> Vec<(&'static str, String)> {
    vec![("ngroups", format!("{}", args[0]))]
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Format syscall arguments into human-readable label-value pairs.
/// Returns an empty Vec for unrecognized syscalls.
pub fn format_syscall_args(
    syscall: i64,
    args: [u64; 6],
    pid: libc::pid_t,
) -> Vec<(&'static str, String)> {
    match syscall {
        // CHOWN
        libc::SYS_chown | libc::SYS_lchown => fmt_chown_path(args, pid),
        libc::SYS_fchown => fmt_fchown(args, pid),
        libc::SYS_fchownat => fmt_fchownat(args, pid),

        // FILE_SYSTEM
        libc::SYS_open => fmt_open(args, pid),
        libc::SYS_openat => fmt_openat(args, pid),
        libc::SYS_openat2 => fmt_openat2(args, pid),
        libc::SYS_creat => fmt_creat(args, pid),

        libc::SYS_stat | libc::SYS_lstat | libc::SYS_access
        | libc::SYS_chdir | libc::SYS_rmdir | libc::SYS_unlink
        | libc::SYS_readlink => fmt_path_only(args, pid),

        libc::SYS_fstat | libc::SYS_fchdir | libc::SYS_ftruncate => fmt_fd_only(args, pid),

        libc::SYS_unlinkat | libc::SYS_faccessat | libc::SYS_faccessat2 => {
            fmt_dirfd_path(args, pid)
        }

        libc::SYS_mkdir => fmt_mkdir(args, pid),
        libc::SYS_mkdirat => fmt_mkdirat(args, pid),
        libc::SYS_mknod => fmt_mknod(args, pid),
        libc::SYS_mknodat => fmt_mknodat(args, pid),

        libc::SYS_rename => fmt_rename(args, pid),
        libc::SYS_renameat | libc::SYS_renameat2 => fmt_renameat(args, pid),

        libc::SYS_symlink => fmt_symlink(args, pid),
        libc::SYS_symlinkat => fmt_symlinkat(args, pid),
        libc::SYS_link => fmt_link(args, pid),
        libc::SYS_linkat => fmt_linkat(args, pid),

        libc::SYS_chmod => fmt_chmod(args, pid),
        libc::SYS_fchmod => fmt_fchmod(args, pid),
        libc::SYS_fchmodat => fmt_fchmodat(args, pid),

        libc::SYS_truncate => fmt_truncate(args, pid),
        libc::SYS_readlinkat => fmt_readlinkat(args, pid),

        // KEYRING
        libc::SYS_add_key => fmt_add_key(args, pid),
        libc::SYS_request_key => fmt_request_key(args, pid),
        libc::SYS_keyctl => fmt_keyctl(args, pid),

        // MODULE
        libc::SYS_init_module => fmt_init_module(args, pid),
        libc::SYS_finit_module => fmt_finit_module(args, pid),
        libc::SYS_delete_module => fmt_delete_module(args, pid),

        // MOUNT
        libc::SYS_mount => fmt_mount(args, pid),
        libc::SYS_umount2 => fmt_umount2(args, pid),
        libc::SYS_chroot => fmt_chroot(args, pid),
        libc::SYS_pivot_root => fmt_pivot_root(args, pid),
        libc::SYS_fsopen => fmt_fsopen(args, pid),
        libc::SYS_fsmount => fmt_fsmount(args, pid),
        libc::SYS_move_mount => fmt_move_mount(args, pid),
        libc::SYS_fsconfig => fmt_fsconfig(args, pid),
        libc::SYS_fspick => fmt_fspick(args, pid),
        libc::SYS_mount_setattr => fmt_mount_setattr(args, pid),

        // SETUID
        libc::SYS_setuid => fmt_set_id("uid", args),
        libc::SYS_setgid => fmt_set_id("gid", args),
        libc::SYS_setreuid => fmt_set_re_id("ruid", "euid", args),
        libc::SYS_setregid => fmt_set_re_id("rgid", "egid", args),
        libc::SYS_setresuid => fmt_set_res_id("ruid", "euid", "suid", args),
        libc::SYS_setresgid => fmt_set_res_id("rgid", "egid", "sgid", args),
        libc::SYS_setgroups => fmt_setgroups(args, pid),

        _ => Vec::new(),
    }
}
