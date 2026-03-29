## Context

Currently `request_syscall_permission` in `src/main.rs` only pretty-prints `openat` syscalls (showing the path and open flags). All other intercepted syscalls show raw hex args, which is not useful for making security decisions. The existing code has `read_cstring_from_pid` and `format_open_flags` inline in main.rs.

Six syscall classes need formatting: CHOWN, FILE_SYSTEM (partially done via openat), KEYRING, MODULE, MOUNT, SETUID. These are the classes identified in `enable_seccomp` as needing path/arg inspection.

## Goals / Non-Goals

**Goals:**
- Human-readable arg display for all syscalls in CHOWN, FILE_SYSTEM, KEYRING, MODULE, MOUNT, SETUID
- A new `src/fmt_syscall.rs` module that owns all formatting logic
- Clean dispatch from `request_syscall_permission` based on syscall number

**Non-Goals:**
- Formatting for NETWORK_IO, DEBUG, IPC, PROCESS, SIGNAL (future work)
- Resolving uid/gid to usernames (just show numeric values)
- Argument validation or policy decisions (this is display only)
- Tests

## Decisions

### 1. New module `src/fmt_syscall.rs` for all formatters

Move `format_open_flags` and `read_cstring_from_pid` from main.rs into `fmt_syscall.rs`. Add per-class formatting functions there. Main.rs calls a single entry point that dispatches on syscall number.

**Rationale:** main.rs is already 690 lines with TUI code. Formatting is a separate concern. A single module is enough — one file per class would be overkill for what's mostly static flag tables.

### 2. Formatter returns structured data, not rendered strings

Each formatter returns a `Vec<(&str, String)>` of label-value pairs (e.g. `[("path", "/etc/passwd"), ("flags", "O_RDONLY|O_CLOEXEC")]`). The TUI code in main.rs renders these into `Span`s.

**Alternative considered:** Return pre-rendered `Line`/`Span` types. Rejected because it couples the formatter to ratatui and makes the module harder to reuse.

### 3. Dispatch by match on `libc::SYS_*` constants

A single `pub fn format_syscall_args(syscall: i64, args: [u64; 6], pid: pid_t) -> Vec<(&str, String)>` with a match statement. Syscalls sharing the same signature (e.g. `chown`/`lchown` both take path, uid, gid) share the same formatting arm.

**Rationale:** Simpler than a registry/HashMap. The syscall list is static and known at compile time. Groups of syscalls with identical signatures collapse into multi-pattern match arms.

### 4. Only format x86_64 syscalls initially

The `SYS_*` constants from libc are architecture-specific. We only handle x86_64 for now.

**Rationale:** The project already assumes x86_64 (uses `libc::SYS_openat` etc). Multi-arch support is future work.

## Risks / Trade-offs

- **Incomplete coverage** → Some obscure syscalls in FILE_SYSTEM (e.g. `oldstat`, `oldfstat`) may not have `libc::SYS_*` constants. We skip those — they're unused on modern kernels.
- **TOCTOU on path reading** → `read_cstring_from_pid` reads paths from the tracee's memory, which could change. This is inherent to the seccomp-notify model and not something we can fix at the formatting layer.
- **Missing seccomp rules** → Adding formatters without also adding `seccomp_rule_add` calls for the new syscalls means they'll never fire. The implementation must update `enable_seccomp` in lockstep.
