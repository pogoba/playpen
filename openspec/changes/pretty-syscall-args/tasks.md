## 1. Create fmt_syscall module and migrate existing code

- [x] 1.1 Create `src/fmt_syscall.rs` with `format_syscall_args(syscall: i64, args: [u64; 6], pid: pid_t) -> Vec<(&str, String)>` entry point
- [x] 1.2 Move `read_cstring_from_pid` from main.rs into fmt_syscall.rs (make it `pub(crate)`)
- [x] 1.3 Move `format_open_flags` from main.rs into fmt_syscall.rs
- [x] 1.4 Add `mod fmt_syscall` to main.rs, update call sites to use the new module

## 2. Implement per-class formatters

- [x] 2.1 CHOWN formatter: handle `chown`, `fchown`, `fchownat`, `lchown` and 32-bit variants (path/fd + uid + gid)
- [x] 2.2 FILE_SYSTEM formatter: handle `open`, `openat`, `openat2`, `creat`, `mkdir`, `mkdirat`, `unlink`, `unlinkat`, `rename`, `renameat`, `renameat2`, `rmdir`, `symlink`, `symlinkat`, `link`, `linkat`, `chmod`, `fchmod`, `fchmodat`, `truncate`, `stat`, `lstat`, `readlink`, `access`, `faccessat` and similar path-based syscalls
- [x] 2.3 KEYRING formatter: handle `add_key`, `request_key`, `keyctl` (type + description strings)
- [x] 2.4 MODULE formatter: handle `init_module`, `finit_module`, `delete_module` (name/fd + params)
- [x] 2.5 MOUNT formatter: handle `mount`, `umount`, `umount2`, `chroot`, `pivot_root`, `fsopen`, `fsmount`, `move_mount` (source + target + fstype)
- [x] 2.6 SETUID formatter: handle `setuid`, `setgid`, `setreuid`, `setregid`, `setresuid`, `setresgid`, `setgroups` and 32-bit variants (uid/gid values)

## 3. Wire up dispatch and seccomp rules

- [x] 3.1 Implement the match dispatch in `format_syscall_args` mapping `SYS_*` constants to the per-class formatters
- [ ] 3.2 Update `enable_seccomp` to add `seccomp_rule_add` for all newly-formatted syscalls
- [ ] 3.3 Update `request_syscall_permission` to call `format_syscall_args` and render the returned label-value pairs as TUI `Span`s, replacing the current openat-specific code
