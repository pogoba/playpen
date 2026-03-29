## ADDED Requirements

### Requirement: Format CHOWN syscall arguments
The system SHALL display path, owner (uid), and group (gid) for `chown`, `fchown`, `fchownat`, `lchown` and their 32-bit variants. For `fchown`/`fchown32`, the first argument is an fd (displayed as integer) instead of a path.

#### Scenario: chown with path
- **WHEN** `chown("/etc/hosts", 1000, 1000)` is intercepted
- **THEN** the prompt displays `path: /etc/hosts`, `owner: 1000`, `group: 1000`

#### Scenario: fchownat with dirfd and path
- **WHEN** `fchownat(AT_FDCWD, "file.txt", 0, 0, 0)` is intercepted
- **THEN** the prompt displays `dirfd: AT_FDCWD`, `path: file.txt`, `owner: 0`, `group: 0`

### Requirement: Format FILE_SYSTEM syscall arguments
The system SHALL display relevant arguments for file system syscalls. Syscalls that take a path SHALL show the path. Syscalls that take flags (open, openat, openat2) SHALL show decoded flags. Syscalls that take a mode SHALL show the mode in octal.

#### Scenario: openat with path and flags
- **WHEN** `openat(AT_FDCWD, "/tmp/foo", O_WRONLY|O_CREAT, 0644)` is intercepted
- **THEN** the prompt displays `path: /tmp/foo`, `flags: O_WRONLY|O_CREAT`, `mode: 0644`

#### Scenario: unlinkat with path
- **WHEN** `unlinkat(AT_FDCWD, "/tmp/foo", 0)` is intercepted
- **THEN** the prompt displays `path: /tmp/foo`

#### Scenario: mkdir with path and mode
- **WHEN** `mkdir("/tmp/newdir", 0755)` is intercepted
- **THEN** the prompt displays `path: /tmp/newdir`, `mode: 0755`

#### Scenario: rename with two paths
- **WHEN** `rename("/tmp/old", "/tmp/new")` is intercepted
- **THEN** the prompt displays `from: /tmp/old`, `to: /tmp/new`

### Requirement: Format KEYRING syscall arguments
The system SHALL display the operation and description for `add_key`, `keyctl`, and `request_key`.

#### Scenario: add_key with type and description
- **WHEN** `add_key("user", "my_key", ...)` is intercepted
- **THEN** the prompt displays `type: user`, `description: my_key`

#### Scenario: request_key with type and description
- **WHEN** `request_key("user", "my_key", ...)` is intercepted
- **THEN** the prompt displays `type: user`, `description: my_key`

### Requirement: Format MODULE syscall arguments
The system SHALL display the module path for `init_module` and `finit_module`, and the module name for `delete_module`.

#### Scenario: finit_module with fd
- **WHEN** `finit_module(fd, "", 0)` is intercepted
- **THEN** the prompt displays `fd: <fd number>`, `params: <param string>`

#### Scenario: delete_module with name
- **WHEN** `delete_module("nvidia", 0)` is intercepted
- **THEN** the prompt displays `name: nvidia`

### Requirement: Format MOUNT syscall arguments
The system SHALL display source, target, and filesystem type for `mount`. For `umount`/`umount2`, it SHALL display the target path. For the new mount API (`fsopen`, `fsmount`, `fsconfig`, `move_mount`), it SHALL display relevant string arguments.

#### Scenario: mount with source, target, and fstype
- **WHEN** `mount("/dev/sda1", "/mnt", "ext4", 0, NULL)` is intercepted
- **THEN** the prompt displays `source: /dev/sda1`, `target: /mnt`, `fstype: ext4`

#### Scenario: umount2 with target
- **WHEN** `umount2("/mnt", 0)` is intercepted
- **THEN** the prompt displays `target: /mnt`

#### Scenario: chroot with path
- **WHEN** `chroot("/var/chroot")` is intercepted
- **THEN** the prompt displays `path: /var/chroot`

### Requirement: Format SETUID syscall arguments
The system SHALL display the uid or gid values for `setuid`, `setgid`, `setreuid`, `setregid`, `setresuid`, `setresgid`, `setgroups` and their 32-bit variants.

#### Scenario: setuid with uid
- **WHEN** `setuid(0)` is intercepted
- **THEN** the prompt displays `uid: 0`

#### Scenario: setresuid with real, effective, saved
- **WHEN** `setresuid(1000, 1000, 1000)` is intercepted
- **THEN** the prompt displays `ruid: 1000`, `euid: 1000`, `suid: 1000`

#### Scenario: setgroups with group list
- **WHEN** `setgroups(3, [10, 20, 30])` is intercepted
- **THEN** the prompt displays `ngroups: 3` (reading the group list from tracee memory is not required)

### Requirement: Dispatch formatting based on syscall number
The `fmt_syscall` module SHALL export a single `format_syscall_args(syscall, args, pid)` function that returns a list of label-value pairs. For unknown syscalls, it SHALL return an empty list (falling back to raw hex display in the TUI).

#### Scenario: Known syscall returns formatted pairs
- **WHEN** `format_syscall_args` is called with `SYS_openat`
- **THEN** it returns label-value pairs including path and flags

#### Scenario: Unknown syscall returns empty list
- **WHEN** `format_syscall_args` is called with an unrecognized syscall number
- **THEN** it returns an empty `Vec`
