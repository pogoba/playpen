//! Integration tests for `playpen merge`.
//!
//! Tests fall into two groups:
//!   1. Pure CLI flows (help, bad paths, empty upper) — driven via
//!      `Command::new(BIN)` directly, no terminal.
//!   2. TUI flows — driven via tmux: a per-test tmux server (unique `-L`
//!      socket) hosts a session that runs the binary; we send keystrokes
//!      with `tmux send-keys` and inspect output via `tmux capture-pane`.
//!
//! No external dev-deps. tempdirs follow the manual `std::env::temp_dir()`
//! pattern used in `src/fd_portal.rs`.

#![allow(dead_code)]

use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BIN: &str = env!("CARGO_BIN_EXE_playpen");

// -----------------------------------------------------------------------------
// Fixtures: per-test tempdir with upper/ and lower/.
// -----------------------------------------------------------------------------

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!(
            "playpen-test-{}-{}-{}-{}",
            name, pid, nanos, n
        ));
        fs::create_dir_all(root.join("upper")).expect("create upper");
        fs::create_dir_all(root.join("lower")).expect("create lower");
        Self { root }
    }
    fn upper(&self) -> PathBuf {
        self.root.join("upper")
    }
    fn lower(&self) -> PathBuf {
        self.root.join("lower")
    }
    fn write(&self, side: &str, rel: &str, content: &[u8]) {
        let full = self.root.join(side).join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }
    fn write_lower(&self, rel: &str, content: &[u8]) {
        self.write("lower", rel, content);
    }
    fn write_upper(&self, rel: &str, content: &[u8]) {
        self.write("upper", rel, content);
    }
    fn lower_exists(&self, rel: &str) -> bool {
        self.lower().join(rel).exists()
    }
    fn upper_exists(&self, rel: &str) -> bool {
        self.upper().join(rel).exists()
    }
    fn read_lower(&self, rel: &str) -> Vec<u8> {
        fs::read(self.lower().join(rel)).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// -----------------------------------------------------------------------------
// tmux harness.
// -----------------------------------------------------------------------------

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Helper macro: skip a test (with a printed reason) if tmux is missing.
macro_rules! require_tmux {
    () => {
        if !tmux_available() {
            eprintln!("skipping: tmux not on PATH");
            return;
        }
    };
}

struct Tui {
    socket: String,
    session: String,
    fixture: Fixture,
    cols: u16,
    rows: u16,
}

impl Tui {
    fn start(fixture: Fixture, cols: u16, rows: u16) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let socket = format!("playpen-tests-{}-{}-{}", std::process::id(), nanos, n);
        let session = "main".to_string();
        let tui = Tui {
            socket,
            session,
            fixture,
            cols,
            rows,
        };

        // Launch the binary inside a short sh wrapper. The trailing
        // `; sleep 30` keeps the pane process alive after the binary
        // exits, which both (a) gives stdout buffers time to flush so
        // tmux sees the binary's final println output, and (b) lets
        // capture-pane keep working until the test drops the Tui.
        let cmd = format!(
            "{} merge --upper {} --lower {}; printf '\\nplaypen_test_done\\n'; sleep 30",
            BIN,
            tui.fixture.upper().display(),
            tui.fixture.lower().display(),
        );
        let status = Command::new("tmux")
            .args([
                "-L",
                &tui.socket,
                "new-session",
                "-d",
                "-s",
                &tui.session,
                "-x",
                &cols.to_string(),
                "-y",
                &rows.to_string(),
                "sh",
                "-c",
                &cmd,
            ])
            .status()
            .expect("spawn tmux");
        assert!(status.success(), "tmux new-session failed");

        tui
    }

    fn send_raw(&self, literal: &str) {
        let status = Command::new("tmux")
            .args([
                "-L",
                &self.socket,
                "send-keys",
                "-t",
                &self.session,
                "-l",
                literal,
            ])
            .status()
            .expect("send-keys -l");
        assert!(status.success(), "send-keys -l failed");
    }

    fn send_key(&self, key: &str) {
        let status = Command::new("tmux")
            .args([
                "-L",
                &self.socket,
                "send-keys",
                "-t",
                &self.session,
                key,
            ])
            .status()
            .expect("send-keys");
        assert!(status.success(), "send-keys {} failed", key);
    }

    fn capture(&self) -> String {
        let out = Command::new("tmux")
            .args([
                "-L",
                &self.socket,
                "capture-pane",
                "-t",
                &self.session,
                "-p",
            ])
            .output()
            .expect("capture-pane");
        assert!(out.status.success(), "capture-pane failed");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn wait_for(&self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let pane = self.capture();
            if pane.contains(needle) {
                return;
            }
            if Instant::now() >= deadline {
                panic!(
                    "timeout waiting for {needle:?}; last pane:\n{}\n",
                    pane
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Wait until the binary has exited (sh wrapper printed
    /// `playpen_test_done`) AND the pane contains `needle`.
    fn wait_for_exit(&self, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let pane = self.capture();
            if pane.contains("playpen_test_done") && pane.contains(needle) {
                return pane;
            }
            if Instant::now() >= deadline {
                panic!(
                    "timeout waiting for {needle:?}; last pane:\n{}\n",
                    pane
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn pane_dead(&self) -> bool {
        let out = Command::new("tmux")
            .args([
                "-L",
                &self.socket,
                "list-panes",
                "-t",
                &self.session,
                "-F",
                "#{pane_dead}",
            ])
            .output()
            .expect("list-panes");
        let s = String::from_utf8_lossy(&out.stdout);
        s.trim().lines().any(|l| l.trim() == "1")
    }

    fn fixture(&self) -> &Fixture {
        &self.fixture
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .status();
        // tmux usually removes its socket on kill-server, but not always.
        if let Ok(uid_out) = Command::new("id").arg("-u").output() {
            if let Ok(uid) = std::str::from_utf8(&uid_out.stdout) {
                let socket_path = std::env::temp_dir()
                    .join(format!("tmux-{}", uid.trim()))
                    .join(&self.socket);
                let _ = fs::remove_file(socket_path);
            }
        }
    }
}

fn merge_cmd(fixture: &Fixture) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.args(["merge", "--upper"])
        .arg(fixture.upper())
        .arg("--lower")
        .arg(fixture.lower());
    cmd
}

fn run(mut cmd: Command) -> Output {
    cmd.output().expect("run binary")
}

// =============================================================================
// CLI-only tests.
// =============================================================================

#[test]
fn cli_help_lists_upper_and_lower() {
    let mut cmd = Command::new(BIN);
    cmd.args(["merge", "--help"]);
    let out = cmd.output().expect("run");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--upper"), "help missing --upper:\n{stdout}");
    assert!(stdout.contains("--lower"), "help missing --lower:\n{stdout}");
}

#[test]
fn cli_bad_upper_errors() {
    let mut cmd = Command::new(BIN);
    cmd.args([
        "merge",
        "--upper",
        "/nonexistent/playpen/path/should/not/exist",
        "--lower",
        "/tmp",
    ]);
    let out = cmd.output().expect("run");
    assert!(!out.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("is not a directory"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn cli_empty_upper_reports_no_changes() {
    let f = Fixture::new("empty-upper");
    let out = merge_cmd(&f).output().expect("run");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("No changes in upper layer."),
        "stdout: {stdout:?}"
    );
}

// =============================================================================
// Helpers used by TUI tests.
// =============================================================================

fn make_tree_fixture(name: &str) -> Fixture {
    let f = Fixture::new(name);
    // src/a.rs modified; src/sub/b.rs modified
    f.write_lower("src/a.rs", b"old a\n");
    f.write_lower("src/sub/b.rs", b"old b\n");
    f.write_upper("src/a.rs", b"new a\n");
    f.write_upper("src/sub/b.rs", b"new b\n");
    // docs/intro.md added; README.md added
    f.write_upper("docs/intro.md", b"intro\n");
    f.write_upper("README.md", b"top\n");
    f
}

// =============================================================================
// TUI tests.
// =============================================================================

#[test]
fn tui_tree_renders() {
    require_tmux!();
    let f = make_tree_fixture("tree-renders");
    let tui = Tui::start(f, 180, 30);
    tui.wait_for("Changes (");
    let pane = tui.capture();
    assert!(pane.contains("(0 selected / 5 total)"), "title:\n{pane}");
    assert!(pane.contains("▾"), "expected expanded marker:\n{pane}");
    assert!(pane.contains("docs/"), "missing docs/:\n{pane}");
    assert!(pane.contains("README.md"), "missing README.md:\n{pane}");
    assert!(pane.contains("src/"), "missing src/:\n{pane}");
}

#[test]
fn tui_cursor_moves_with_j() {
    require_tmux!();
    let f = make_tree_fixture("nav");
    let tui = Tui::start(f, 180, 30);
    tui.wait_for("Changes (");
    let before = tui.capture();
    let cursor_line = |pane: &str| -> Option<String> {
        pane.lines()
            .find(|l| l.contains("│> "))
            .map(|s| s.to_string())
    };
    let initial = cursor_line(&before).expect("initial cursor line");

    tui.send_key("j");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let now = tui.capture();
        if let Some(after) = cursor_line(&now) {
            if after != initial {
                break;
            }
        }
        if Instant::now() >= deadline {
            panic!(
                "cursor never moved.\nbefore:\n{}\nafter:\n{}",
                before, now
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn tui_recursive_selection_on_implicit_dir() {
    require_tmux!();
    // src/ is implicit (no entry of its own); selecting it should select
    // both files under it (a.rs and sub/b.rs == 2 entries).
    let f = make_tree_fixture("recursive");
    let tui = Tui::start(f, 180, 30);
    tui.wait_for("(0 selected / 5 total)");

    // Move cursor to the src/ row. With sorted-dirs-first, the first row
    // is docs/; press j twice (past docs/intro.md) to land on src/.
    tui.send_key("j"); // -> docs/intro.md
    tui.send_key("j"); // -> src/
    // Toggle selection.
    tui.send_key("Space");
    tui.wait_for("(2 selected / 5 total)");

    let pane = tui.capture();
    // src/ row should be [x]
    let src_row = pane
        .lines()
        .find(|l| l.contains("src/"))
        .expect("src/ row");
    assert!(src_row.contains("[x]"), "src row: {src_row:?}");
}

#[test]
fn tui_partial_state_when_one_child_unselected() {
    require_tmux!();
    let f = make_tree_fixture("partial");
    let tui = Tui::start(f, 180, 30);
    tui.wait_for("(0 selected / 5 total)");

    // Cursor → src/, recursive select, then toggle off one of the
    // descendants and assert src/ shows [~].
    tui.send_key("j");
    tui.send_key("j"); // src/
    tui.send_key("Space");
    tui.wait_for("(2 selected / 5 total)");

    // Children of src/: sub/ (dir, comes first), then b.rs under sub/,
    // then a.rs at src/ top level. Move cursor down to src/sub/b.rs.
    tui.send_key("j"); // → src/sub/
    tui.send_key("j"); // → src/sub/b.rs
    tui.send_key("Space");
    tui.wait_for("(1 selected / 5 total)");

    let pane = tui.capture();
    let src_row = pane
        .lines()
        .find(|l| l.contains(" src/"))
        .expect("src/ row");
    assert!(
        src_row.contains("[~]"),
        "expected partial marker [~] on src/, got: {src_row:?}\nfull pane:\n{pane}"
    );
}

#[test]
fn tui_fold_and_unfold() {
    require_tmux!();
    let f = make_tree_fixture("fold");
    let tui = Tui::start(f, 180, 30);
    tui.wait_for("(0 selected / 5 total)");

    // Step to src/ and collapse with h. The list pane renders leaves as
    // "[ ] M b.rs" — the checkbox prefix uniquely scopes our match to
    // the left pane (the right diff pane shows full paths without `[ ]`).
    tui.send_key("j");
    tui.send_key("j"); // src/
    let before = tui.capture();
    assert!(
        before.contains("[ ] M b.rs"),
        "expected b.rs in list before fold:\n{before}"
    );
    tui.send_key("h");

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let now = tui.capture();
        if !now.contains("[ ] M b.rs") && !now.contains("[ ] M a.rs") {
            assert!(now.contains("▸"), "expected collapsed marker:\n{now}");
            break;
        }
        if Instant::now() >= deadline {
            panic!("fold never hid children:\n{}", now);
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    tui.send_key("l");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let now = tui.capture();
        if now.contains("[ ] M b.rs") || now.contains("[ ] M a.rs") {
            break;
        }
        if Instant::now() >= deadline {
            panic!("unfold never restored children:\n{}", now);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn tui_apply_writes_lower_and_leaves_upper_intact() {
    require_tmux!();
    // Fixture tailored to the apply assertions.
    let f = Fixture::new("apply");
    f.write_lower("equal.txt", b"identical\n");
    f.write_upper("equal.txt", b"identical\n");
    f.write_lower("changed.txt", b"before\n");
    f.write_upper("changed.txt", b"after\n");
    f.write_upper("added.txt", b"new\n");

    let tui = Tui::start(f, 180, 30);
    tui.wait_for("(0 selected / 2 total)");
    // Sanity: equal.txt is hidden by content-equality.
    let pane = tui.capture();
    assert!(!pane.contains("equal.txt"), "equal.txt should be hidden:\n{pane}");

    tui.send_key("a"); // select all
    tui.wait_for("(2 selected / 2 total)");
    tui.send_key("A"); // apply

    let final_pane = tui.wait_for_exit("Applied 2 entries");

    let f = tui.fixture();
    assert_eq!(f.read_lower("changed.txt"), b"after\n");
    assert_eq!(f.read_lower("added.txt"), b"new\n");
    // Upper is preserved.
    assert!(f.upper_exists("changed.txt"), "upper changed.txt should survive");
    assert!(f.upper_exists("added.txt"), "upper added.txt should survive");
    assert!(f.upper_exists("equal.txt"), "upper equal.txt should survive");
    // equal.txt unchanged in lower.
    assert_eq!(f.read_lower("equal.txt"), b"identical\n");

    // Suppress unused variable warning.
    let _ = final_pane;
}

#[test]
fn tui_rerun_after_apply_reports_no_changes() {
    require_tmux!();
    let f = Fixture::new("rerun");
    f.write_lower("file.txt", b"before\n");
    f.write_upper("file.txt", b"after\n");

    {
        let tui = Tui::start(f, 180, 30);
        tui.wait_for("(0 selected / 1 total)");
        tui.send_key("a");
        tui.wait_for("(1 selected / 1 total)");
        tui.send_key("A");
        tui.wait_for_exit("Applied 1 entry");

        // Sanity: lower received the apply, upper retained.
        let f = tui.fixture();
        assert_eq!(f.read_lower("file.txt"), b"after\n");
        assert!(f.upper_exists("file.txt"));

        // Re-run as a plain CLI invocation. The Tui's tempdir is owned by
        // its Fixture; clone the paths first so we can run after Tui drop.
        let upper = f.upper();
        let lower = f.lower();
        let out = Command::new(BIN)
            .args([OsStr::new("merge"), OsStr::new("--upper")])
            .arg(upper)
            .args([OsStr::new("--lower")])
            .arg(lower)
            .output()
            .unwrap();
        assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("No changes in upper layer."),
            "stdout: {stdout:?}"
        );
    }
}

#[test]
fn tui_quit_leaves_lower_unchanged() {
    require_tmux!();
    let f = Fixture::new("quit");
    f.write_lower("file.txt", b"before\n");
    f.write_upper("file.txt", b"after\n");
    let lower_before = f.read_lower("file.txt");

    let tui = Tui::start(f, 180, 30);
    tui.wait_for("(0 selected / 1 total)");
    tui.send_key("q");
    tui.wait_for_exit("Aborted.");
    let f = tui.fixture();
    assert_eq!(f.read_lower("file.txt"), lower_before);
    assert!(f.upper_exists("file.txt"));
}

#[test]
fn tui_diff_pane_handles_tabs_and_escapes() {
    require_tmux!();
    let f = Fixture::new("sanitize");
    // Upper file embeds a CSI escape and a tab; the diff pane must not
    // emit raw escape codes (which would corrupt the screen) and must
    // expand tabs to keep ratatui's buffer aligned.
    f.write_lower("note.txt", b"bye\n");
    f.write_upper("note.txt", b"\x1b[31mhi\x1b[0m\there\n");

    let tui = Tui::start(f, 180, 30);
    tui.wait_for("(0 selected / 1 total)");
    let pane = tui.capture();

    // No raw `[` immediately following an unprintable; substring "31mhi"
    // would only appear if escape stripping failed.
    assert!(
        !pane.contains("31mhi"),
        "found unstripped CSI body in diff:\n{pane}"
    );
    // Tab should have been expanded; "+hi" should be followed by spaces
    // and then "here", not by a literal tab.
    let line = pane
        .lines()
        .find(|l| l.contains("+hi") && l.contains("here"))
        .expect("expected diff line containing +hi…here");
    let between = &line[line.find("+hi").unwrap() + 3..line.find("here").unwrap()];
    assert!(
        !between.contains('\t') && between.chars().all(|c| c == ' '),
        "expected only spaces between +hi and here, got: {between:?}"
    );
}
