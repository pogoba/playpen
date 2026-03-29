## Why

When the seccomp notifier intercepts a syscall, the user currently sees raw hex arguments for everything except `openat`. This makes it hard to make informed allow/deny decisions for security-sensitive syscalls like `chown`, `mount`, or `unlink`. Pretty-printing args (paths, uids, flags) for the remaining dangerous syscall classes gives the user the context they need.

## What Changes

- Add human-readable argument formatting for syscalls in these classes: CHOWN, FILE_SYSTEM, KEYRING, MODULE, MOUNT, SETUID
- Extend `request_syscall_permission` to dispatch to per-class formatters instead of only handling `openat`
- Create a new `src/fmt_syscall.rs` module with shared formatting helpers and per-class formatters
- Move existing formatting logic (`format_open_flags`, path reading) into the new module
- Register the new syscall classes for seccomp user notification interception in `enable_seccomp`

## Capabilities

### New Capabilities
- `syscall-arg-formatting`: Per-class pretty-printing of syscall arguments for display in the TUI permission prompt

### Modified Capabilities

(none — no existing specs)

## Impact

- New file: `src/fmt_syscall.rs` — formatting helpers and per-class arg formatters
- `src/main.rs`: `request_syscall_permission` gains dispatch to formatters; `enable_seccomp` gains rules for new syscalls; `format_open_flags` and `read_cstring_from_pid` move to new module
- `src/syscalls.rs`: Used as reference for which syscalls belong to each class
- No new dependencies — `libc` constants cover mount flags, open flags, etc.
