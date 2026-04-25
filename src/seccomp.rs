use crate::fd_portal;
use std::io::{self, IsTerminal, Write};
use std::os::unix::io::OwnedFd;
use std::os::fd::IntoRawFd;
use std::ptr;

pub fn enable_seccomp(sender: &fd_portal::FdPortalSender, syscall_map: &std::collections::HashMap<i32, &str>) -> Result<(), String> {
    let ctx = unsafe { libseccomp_sys::seccomp_init(libseccomp_sys::SCMP_ACT_ALLOW) };
    if ctx.is_null() {
        return Err("Failed to initialize seccomp filter".to_string());
    }


    // Notes: (leave this comment in)
    //
    // deny by default
    //
    // allow: read only, benign, or guarded by our fd creation guards
    // BASIC_IO, DEFAULT (execve can at best read binaries), CLOCK, TIMER, CPU_EMULATION, IO_EVENT, MEMLOCK, RESOURCES, SYNC, AIO
    //
    // check path or ip args:
    // CHOWN, FILE_SYSTEM, KEYRING, MODULE, MOUNT, SETUID,
    //
    //
    // check or guard through network namespace: NETWORK_IO,
    //
    // check or guard through process namespace: DEBUG, IPC, PROCESS, SIGNAL
    //
    // check or deny: PRIVILEGED (CHOWN, CLOCK, MODULE, RAW_IO, REBOOT, SWAP)
    //
    // deny: OBSOLETE, PKEY, RAW_IO, REBOOT, SANDBOX, SWAP
    //
    // not sure: SYSTEM_SERVICE is a mixed bag of new calls and ones defined elsewhere
    //
    //
    // let write_notif = unsafe {
    //     libseccomp_sys::seccomp_rule_add(
    //         ctx,
    //         libseccomp_sys::SECCOMP_RET_USER_NOTIF,
    //         libc::SYS_write as libc::c_int,
    //         0,
    //     )
    // };
    // if write_notif != 0 {
    //     unsafe { libseccomp_sys::seccomp_release(ctx) };
    //     return Err(format!(
    //         "Failed to install write notification rule: {}",
    //         write_notif
    //     ));
    // }
    //
    // let openat_notif = unsafe {
    //     libseccomp_sys::seccomp_rule_add(
    //         ctx,
    //         libseccomp_sys::SECCOMP_RET_USER_NOTIF,
    //         libc::SYS_openat as libc::c_int,
    //         0,
    //     )
    // };
    // if openat_notif != 0 {
    //     unsafe { libseccomp_sys::seccomp_release(ctx) };
    //     return Err(format!(
    //         "Failed to install openat notification rule: {}",
    //         openat_notif
    //     ));

    for &sys in syscall_map.keys() {
        let ret = unsafe {
            libseccomp_sys::seccomp_rule_add(
                ctx,
                libseccomp_sys::SECCOMP_RET_USER_NOTIF,
                sys as libc::c_int,
                0,
            )
        };
        if ret != 0 {
            unsafe { libseccomp_sys::seccomp_release(ctx) };
            return Err(format!(
                "Failed to install notification rule for syscall {}: {}",
                sys, ret
            ));
        }
    }

    let load_result = unsafe { libseccomp_sys::seccomp_load(ctx) };
    if load_result != 0 {
        unsafe {
            libseccomp_sys::seccomp_release(ctx);
        }
        return Err(format!("Failed to load seccomp filter: {}", load_result));
    }

    let notify_fd =
        unsafe { libseccomp_sys::seccomp_notify_fd(ctx as libseccomp_sys::const_scmp_filter_ctx) };
    if notify_fd < 0 {
        let err = io::Error::last_os_error();
        unsafe { libseccomp_sys::seccomp_release(ctx) };
        return Err(format!(
            "Failed to retrieve seccomp notification fd (is user notification supported?): {}",
            err
        ));
    }

    unsafe { libseccomp_sys::seccomp_release(ctx) };

    sender
        .send_fd(notify_fd)
        .map_err(|err| format!("Failed to send seccomp listener: {err}"))?;
    unsafe { libc::close(notify_fd) };

    Ok(())
}

pub fn handle_seccomp_notifications(listener: OwnedFd, prompt: bool, syscall_map: &std::collections::HashMap<i32, &str>) -> io::Result<()> {
    let fd = listener.into_raw_fd();
    unsafe {
        let mut req = ptr::null_mut();
        let mut resp = ptr::null_mut();
        if libseccomp_sys::seccomp_notify_alloc(&mut req, &mut resp) != 0 {
            libc::close(fd);
            return Err(io::Error::last_os_error());
        }

        let result = loop {
            // we need to clear the buffer because the kernel will refuse to overwrite non-zeros
            (*req).id = 0;
            (*req).pid = 0;
            (*req).flags = 0;
            (*req).data.nr = 0;
            (*req).data.arch = 0;
            (*req).data.instruction_pointer = 0;
            (*req).data.args = [0;6];

            let ret = libseccomp_sys::seccomp_notify_receive(fd, req);
            if ret < 0 {
                let err = io::Error::last_os_error();
                eprintln!("seccomp_notify_receive failed ret {}, err {}", ret, err);
                match err.raw_os_error() {
                    Some(libc::EBADF) | Some(libc::EINVAL) => break Ok(()),
                    _ => break Err(err),
                }
            }

            let args = (*req).data.args;
            let allow = crate::request_syscall_permission(
                (*req).data.nr as i64,
                args,
                (*req).pid as libc::pid_t,
                prompt,
                syscall_map,
            );
            if !allow {
                (*resp).id = (*req).id;
                (*resp).val = 0;
                (*resp).error = -libc::EPERM;
                (*resp).flags = 0;

                if libseccomp_sys::seccomp_notify_respond(fd, resp) < 0 {
                    eprintln!("seccomp_notify_respond (EPERM) failed");
                    break Err(io::Error::last_os_error());
                }
                continue;
            }

            (*resp).id = (*req).id;
            (*resp).val = 0;
            (*resp).error = 0;
            (*resp).flags = libseccomp_sys::SECCOMP_USER_NOTIF_FLAG_CONTINUE;

            if libseccomp_sys::seccomp_notify_respond(fd, resp) < 0 {
                let err = io::Error::last_os_error();
                eprintln!("seccom_notify_respond (CONTINUE) failed");
                break Err(err);
            }
        };

        libseccomp_sys::seccomp_notify_free(req, resp);
        libc::close(fd);
        result
    }
}
