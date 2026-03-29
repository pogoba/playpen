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

/// Format syscall arguments into human-readable label-value pairs.
/// Returns an empty Vec for unrecognized syscalls.
pub fn format_syscall_args(
    _syscall: i64,
    _args: [u64; 6],
    _pid: libc::pid_t,
) -> Vec<(&'static str, String)> {
    // Dispatch will be filled in by task 3.1
    Vec::new()
}
