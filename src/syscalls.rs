/// A named group of syscalls, ported from systemd's seccomp-util.c
pub struct SyscallFilterSet {
    pub name: &'static str,
    pub help: &'static str,
    /// Syscall names or "@group" references to other sets.
    pub syscalls: &'static [&'static str],
}

pub const DEFAULT: SyscallFilterSet = SyscallFilterSet {
    name: "@default",
    help: "System calls that are always permitted",
    syscalls: &[
        "@sandbox",
        "arch_prctl",
        "brk",
        "cacheflush",
        "clock_getres",
        "clock_getres_time64",
        "clock_gettime",
        "clock_gettime64",
        "clock_nanosleep",
        "clock_nanosleep_time64",
        "execve",
        "exit",
        "exit_group",
        "futex",
        "futex_time64",
        "futex_waitv",
        "get_robust_list",
        "get_thread_area",
        "getegid",
        "getegid32",
        "geteuid",
        "geteuid32",
        "getgid",
        "getgid32",
        "getgroups",
        "getgroups32",
        "getpgid",
        "getpgrp",
        "getpid",
        "getppid",
        "getrandom",
        "getresgid",
        "getresgid32",
        "getresuid",
        "getresuid32",
        "getrlimit",
        "getsid",
        "gettid",
        "gettimeofday",
        "getuid",
        "getuid32",
        "lsm_get_self_attr",
        "lsm_list_modules",
        "membarrier",
        "mmap",
        "mmap2",
        "mprotect",
        "mseal",
        "munmap",
        "nanosleep",
        "pause",
        "prlimit64",
        "restart_syscall",
        "riscv_flush_icache",
        "riscv_hwprobe",
        "rseq",
        "rt_sigreturn",
        "sched_getaffinity",
        "sched_yield",
        "set_robust_list",
        "set_thread_area",
        "set_tid_address",
        "set_tls",
        "sigreturn",
        "time",
        "ugetrlimit",
        "uretprobe",
    ],
};

pub const AIO: SyscallFilterSet = SyscallFilterSet {
    name: "@aio",
    help: "Asynchronous IO",
    syscalls: &[
        "io_cancel",
        "io_destroy",
        "io_getevents",
        "io_pgetevents",
        "io_pgetevents_time64",
        "io_setup",
        "io_submit",
        "io_uring_enter",
        "io_uring_register",
        "io_uring_setup",
    ],
};

pub const BASIC_IO: SyscallFilterSet = SyscallFilterSet {
    name: "@basic-io",
    help: "Basic IO",
    syscalls: &[
        "_llseek",
        "close",
        "close_range",
        "dup",
        "dup2",
        "dup3",
        "llseek",
        "lseek",
        "pread64",
        "preadv",
        "preadv2",
        "pwrite64",
        "pwritev",
        "pwritev2",
        "read",
        "readv",
        "write",
        "writev",
    ],
};

pub const CHOWN: SyscallFilterSet = SyscallFilterSet {
    name: "@chown",
    help: "Change ownership of files and directories",
    syscalls: &[
        "chown",
        "chown32",
        "fchown",
        "fchown32",
        "fchownat",
        "lchown",
        "lchown32",
    ],
};

pub const CLOCK: SyscallFilterSet = SyscallFilterSet {
    name: "@clock",
    help: "Change the system time",
    syscalls: &[
        "adjtimex",
        "clock_adjtime",
        "clock_adjtime64",
        "clock_settime",
        "clock_settime64",
        "settimeofday",
    ],
};

pub const CPU_EMULATION: SyscallFilterSet = SyscallFilterSet {
    name: "@cpu-emulation",
    help: "System calls for CPU emulation functionality",
    syscalls: &[
        "modify_ldt",
        "subpage_prot",
        "switch_endian",
        "vm86",
        "vm86old",
    ],
};

pub const DEBUG: SyscallFilterSet = SyscallFilterSet {
    name: "@debug",
    help: "Debugging, performance monitoring and tracing functionality",
    syscalls: &[
        "lookup_dcookie",
        "perf_event_open",
        "pidfd_getfd",
        "ptrace",
        "rtas",
        "s390_runtime_instr",
        "sys_debug_setcontext",
    ],
};

pub const FILE_SYSTEM: SyscallFilterSet = SyscallFilterSet {
    name: "@file-system",
    help: "File system operations",
    syscalls: &[
        "access",
        "chdir",
        "chmod",
        "close",
        "creat",
        "faccessat",
        "faccessat2",
        "fallocate",
        "fchdir",
        "fchmod",
        "fchmodat",
        "fchmodat2",
        "fcntl",
        "fcntl64",
        "fgetxattr",
        "file_getattr",
        "file_setattr",
        "flistxattr",
        "fremovexattr",
        "fsetxattr",
        "fstat",
        "fstat64",
        "fstatat",
        "fstatat64",
        "fstatfs",
        "fstatfs64",
        "ftruncate",
        "ftruncate64",
        "futimesat",
        "getcwd",
        "getdents",
        "getdents64",
        "getxattr",
        "getxattrat",
        "inotify_add_watch",
        "inotify_init",
        "inotify_init1",
        "inotify_rm_watch",
        "lgetxattr",
        "link",
        "linkat",
        "listmount",
        "listxattr",
        "listxattrat",
        "llistxattr",
        "lremovexattr",
        "lsetxattr",
        "lstat",
        "lstat64",
        "mkdir",
        "mkdirat",
        "mknod",
        "mknodat",
        "newfstat",
        "newfstatat",
        "oldfstat",
        "oldlstat",
        "oldstat",
        "open",
        "open_tree",
        "openat",
        "openat2",
        "readlink",
        "readlinkat",
        "removexattr",
        "removexattrat",
        "rename",
        "renameat",
        "renameat2",
        "rmdir",
        "setxattr",
        "setxattrat",
        "stat",
        "stat64",
        "statfs",
        "statfs64",
        "statmount",
        "statx",
        "symlink",
        "symlinkat",
        "truncate",
        "truncate64",
        "unlink",
        "unlinkat",
        "utime",
        "utimensat",
        "utimensat_time64",
        "utimes",
    ],
};

pub const IO_EVENT: SyscallFilterSet = SyscallFilterSet {
    name: "@io-event",
    help: "Event loop system calls",
    syscalls: &[
        "_newselect",
        "epoll_create",
        "epoll_create1",
        "epoll_ctl",
        "epoll_ctl_old",
        "epoll_pwait",
        "epoll_pwait2",
        "epoll_wait",
        "epoll_wait_old",
        "eventfd",
        "eventfd2",
        "poll",
        "ppoll",
        "ppoll_time64",
        "pselect6",
        "pselect6_time64",
        "select",
    ],
};

pub const IPC: SyscallFilterSet = SyscallFilterSet {
    name: "@ipc",
    help: "SysV IPC, POSIX Message Queues or other IPC",
    syscalls: &[
        "ipc",
        "memfd_create",
        "mq_getsetattr",
        "mq_notify",
        "mq_open",
        "mq_timedreceive",
        "mq_timedreceive_time64",
        "mq_timedsend",
        "mq_timedsend_time64",
        "mq_unlink",
        "msgctl",
        "msgget",
        "msgrcv",
        "msgsnd",
        "pipe",
        "pipe2",
        "process_madvise",
        "process_vm_readv",
        "process_vm_writev",
        "semctl",
        "semget",
        "semop",
        "semtimedop",
        "semtimedop_time64",
        "shmat",
        "shmctl",
        "shmdt",
        "shmget",
    ],
};

pub const KEYRING: SyscallFilterSet = SyscallFilterSet {
    name: "@keyring",
    help: "Kernel keyring access",
    syscalls: &[
        "add_key",
        "keyctl",
        "request_key",
    ],
};

pub const MEMLOCK: SyscallFilterSet = SyscallFilterSet {
    name: "@memlock",
    help: "Memory locking control",
    syscalls: &[
        "mlock",
        "mlock2",
        "mlockall",
        "munlock",
        "munlockall",
    ],
};

pub const MODULE: SyscallFilterSet = SyscallFilterSet {
    name: "@module",
    help: "Loading and unloading of kernel modules",
    syscalls: &[
        "delete_module",
        "finit_module",
        "init_module",
    ],
};

pub const MOUNT: SyscallFilterSet = SyscallFilterSet {
    name: "@mount",
    help: "Mounting and unmounting of file systems",
    syscalls: &[
        "chroot",
        "fsconfig",
        "fsmount",
        "fsopen",
        "fspick",
        "mount",
        "mount_setattr",
        "move_mount",
        "open_tree_attr",
        "pivot_root",
        "umount",
        "umount2",
    ],
};

pub const NETWORK_IO: SyscallFilterSet = SyscallFilterSet {
    name: "@network-io",
    help: "Network or Unix socket IO, should not be needed if not network facing",
    syscalls: &[
        "accept",
        "accept4",
        "bind",
        "connect",
        "getpeername",
        "getsockname",
        "getsockopt",
        "listen",
        "recv",
        "recvfrom",
        "recvmmsg",
        "recvmmsg_time64",
        "recvmsg",
        "send",
        "sendmmsg",
        "sendmsg",
        "sendto",
        "setsockopt",
        "shutdown",
        "socket",
        "socketcall",
        "socketpair",
    ],
};

pub const OBSOLETE: SyscallFilterSet = SyscallFilterSet {
    name: "@obsolete",
    help: "Unusual, obsolete or unimplemented system calls",
    syscalls: &[
        "_sysctl",
        "afs_syscall",
        "bdflush",
        "break",
        "create_module",
        "ftime",
        "get_kernel_syms",
        "getpmsg",
        "gtty",
        "idle",
        "lock",
        "mpx",
        "prof",
        "profil",
        "putpmsg",
        "query_module",
        "security",
        "sgetmask",
        "ssetmask",
        "stime",
        "stty",
        "sysfs",
        "tuxcall",
        "ulimit",
        "uselib",
        "ustat",
        "vserver",
    ],
};

pub const PKEY: SyscallFilterSet = SyscallFilterSet {
    name: "@pkey",
    help: "System calls used for memory protection keys",
    syscalls: &[
        "pkey_alloc",
        "pkey_free",
        "pkey_mprotect",
    ],
};

pub const PRIVILEGED: SyscallFilterSet = SyscallFilterSet {
    name: "@privileged",
    help: "All system calls which need super-user capabilities",
    syscalls: &[
        "@chown",
        "@clock",
        "@module",
        "@raw-io",
        "@reboot",
        "@swap",
        "_sysctl",
        "acct",
        "bpf",
        "capset",
        "chroot",
        "fanotify_init",
        "fanotify_mark",
        "nfsservctl",
        "open_by_handle_at",
        "pivot_root",
        "quotactl",
        "quotactl_fd",
        "setdomainname",
        "setfsuid",
        "setfsuid32",
        "setgroups",
        "setgroups32",
        "sethostname",
        "setresuid",
        "setresuid32",
        "setreuid",
        "setreuid32",
        "setuid",
        "setuid32",
        "vhangup",
    ],
};

pub const PROCESS: SyscallFilterSet = SyscallFilterSet {
    name: "@process",
    help: "Process control, execution, namespacing operations",
    syscalls: &[
        "capget",
        "clone",
        "clone3",
        "execveat",
        "fork",
        "getrusage",
        "kill",
        "pidfd_open",
        "pidfd_send_signal",
        "prctl",
        "rt_sigqueueinfo",
        "rt_tgsigqueueinfo",
        "setns",
        "swapcontext",
        "tgkill",
        "times",
        "tkill",
        "unshare",
        "vfork",
        "wait4",
        "waitid",
        "waitpid",
    ],
};

pub const RAW_IO: SyscallFilterSet = SyscallFilterSet {
    name: "@raw-io",
    help: "Raw I/O port access",
    syscalls: &[
        "ioperm",
        "iopl",
        "pciconfig_iobase",
        "pciconfig_read",
        "pciconfig_write",
        "s390_pci_mmio_read",
        "s390_pci_mmio_write",
    ],
};

pub const REBOOT: SyscallFilterSet = SyscallFilterSet {
    name: "@reboot",
    help: "Reboot and reboot preparation/kexec",
    syscalls: &[
        "kexec_file_load",
        "kexec_load",
        "reboot",
    ],
};

pub const RESOURCES: SyscallFilterSet = SyscallFilterSet {
    name: "@resources",
    help: "Alter resource settings",
    syscalls: &[
        "ioprio_set",
        "mbind",
        "migrate_pages",
        "move_pages",
        "nice",
        "sched_setaffinity",
        "sched_setattr",
        "sched_setparam",
        "sched_setscheduler",
        "set_mempolicy",
        "set_mempolicy_home_node",
        "setpriority",
        "setrlimit",
    ],
};

pub const SANDBOX: SyscallFilterSet = SyscallFilterSet {
    name: "@sandbox",
    help: "Sandbox functionality",
    syscalls: &[
        "landlock_add_rule",
        "landlock_create_ruleset",
        "landlock_restrict_self",
        "seccomp",
    ],
};

pub const SETUID: SyscallFilterSet = SyscallFilterSet {
    name: "@setuid",
    help: "Operations for changing user/group credentials",
    syscalls: &[
        "setgid",
        "setgid32",
        "setgroups",
        "setgroups32",
        "setregid",
        "setregid32",
        "setresgid",
        "setresgid32",
        "setresuid",
        "setresuid32",
        "setreuid",
        "setreuid32",
        "setuid",
        "setuid32",
    ],
};

pub const SIGNAL: SyscallFilterSet = SyscallFilterSet {
    name: "@signal",
    help: "Process signal handling",
    syscalls: &[
        "rt_sigaction",
        "rt_sigpending",
        "rt_sigprocmask",
        "rt_sigsuspend",
        "rt_sigtimedwait",
        "rt_sigtimedwait_time64",
        "sigaction",
        "sigaltstack",
        "signal",
        "signalfd",
        "signalfd4",
        "sigpending",
        "sigprocmask",
        "sigsuspend",
    ],
};

pub const SWAP: SyscallFilterSet = SyscallFilterSet {
    name: "@swap",
    help: "Enable/disable swap devices",
    syscalls: &[
        "swapoff",
        "swapon",
    ],
};

pub const SYNC: SyscallFilterSet = SyscallFilterSet {
    name: "@sync",
    help: "Synchronize files and memory to storage",
    syscalls: &[
        "fdatasync",
        "fsync",
        "msync",
        "sync",
        "sync_file_range",
        "sync_file_range2",
        "syncfs",
    ],
};

pub const SYSTEM_SERVICE: SyscallFilterSet = SyscallFilterSet {
    name: "@system-service",
    help: "General system service operations",
    syscalls: &[
        "@aio",
        "@basic-io",
        "@chown",
        "@default",
        "@file-system",
        "@io-event",
        "@ipc",
        "@keyring",
        "@memlock",
        "@network-io",
        "@process",
        "@resources",
        "@setuid",
        "@signal",
        "@sync",
        "@timer",
        "arm_fadvise64_64",
        "capget",
        "capset",
        "copy_file_range",
        "fadvise64",
        "fadvise64_64",
        "flock",
        "get_mempolicy",
        "getcpu",
        "getpriority",
        "ioctl",
        "ioprio_get",
        "kcmp",
        "madvise",
        "mremap",
        "name_to_handle_at",
        "oldolduname",
        "olduname",
        "personality",
        "readahead",
        "readdir",
        "remap_file_pages",
        "sched_get_priority_max",
        "sched_get_priority_min",
        "sched_getattr",
        "sched_getparam",
        "sched_getscheduler",
        "sched_rr_get_interval",
        "sched_rr_get_interval_time64",
        "sched_yield",
        "sendfile",
        "sendfile64",
        "setfsgid",
        "setfsgid32",
        "setfsuid",
        "setfsuid32",
        "setpgid",
        "setsid",
        "splice",
        "sysinfo",
        "tee",
        "umask",
        "uname",
        "userfaultfd",
        "vmsplice",
    ],
};

pub const TIMER: SyscallFilterSet = SyscallFilterSet {
    name: "@timer",
    help: "Schedule operations by time",
    syscalls: &[
        "alarm",
        "getitimer",
        "setitimer",
        "timer_create",
        "timer_delete",
        "timer_getoverrun",
        "timer_gettime",
        "timer_gettime64",
        "timer_settime",
        "timer_settime64",
        "timerfd_create",
        "timerfd_gettime",
        "timerfd_gettime64",
        "timerfd_settime",
        "timerfd_settime64",
        "times",
    ],
};

/// All filter sets, for lookup by name.
pub const ALL_SETS: &[&SyscallFilterSet] = &[
    &DEFAULT,
    &AIO,
    &BASIC_IO,
    &CHOWN,
    &CLOCK,
    &CPU_EMULATION,
    &DEBUG,
    &FILE_SYSTEM,
    &IO_EVENT,
    &IPC,
    &KEYRING,
    &MEMLOCK,
    &MODULE,
    &MOUNT,
    &NETWORK_IO,
    &OBSOLETE,
    &PKEY,
    &PRIVILEGED,
    &PROCESS,
    &RAW_IO,
    &REBOOT,
    &RESOURCES,
    &SANDBOX,
    &SETUID,
    &SIGNAL,
    &SWAP,
    &SYNC,
    &SYSTEM_SERVICE,
    &TIMER,
];

/// Look up a filter set by its "@name".
pub fn find_set(name: &str) -> Option<&'static SyscallFilterSet> {
    ALL_SETS.iter().find(|s| s.name == name).copied()
}

use std::collections::HashMap;
use std::ffi::CString;

/// Build a map from syscall number → name for the given filter sets.
///
/// Recursively expands `@group` references. Uses libseccomp's
/// `seccomp_syscall_resolve_name()` to resolve each name string to its
/// architecture-specific syscall number. Unknown names (returning -1) are
/// silently skipped.
pub fn resolve_syscall_map(sets: &[&SyscallFilterSet]) -> HashMap<i32, &'static str> {
    let mut list = vec![];
    for set in sets {
        collect_into(&mut list, set);
    }
    // resolve syscall id
    for syscall in list {
        if let Ok(cname) = CString::new(syscall) {
            let nr = unsafe {
                libseccomp_sys::seccomp_syscall_resolve_name(cname.as_ptr())
            };
            if nr >= 0 {
                map.insert(nr, entry);
            }
        }
    }

    map
}

fn collect_into(list: &mut Vec<&'static str>, set: &SyscallFilterSet) {
    for &entry in set.syscalls {
        if let Some(group_name) = entry.strip_prefix('@') {
            // Expand @group reference
            let full_name = format!("@{}", group_name);
            if let Some(sub) = find_set(&full_name) {
                collect_into(list, sub);
            }
        } else {
            list.push(entry);
        }
    }
}

pub fn get_syscalls_of(sets: &[&SyscallFilterSet]) -> Vec<(i32, &'static str)> {
    let mut syscalls = vec![];

    for set in sets {

    }

    syscalls
}
