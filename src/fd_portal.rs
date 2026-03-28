//! Safe descriptor portal inspired by the `user-trap.c` sample.
//!
//! This module exposes a pair of endpoints backed by a UNIX domain
//! `SOCK_SEQPACKET` socket pair that can exchange file descriptors via
//! `SCM_RIGHTS`. Such portals are useful when one thread creates a kernel
//! handle (for example a seccomp listener) and needs to hand it to another
//! thread without leaking the descriptor or racing on close.
//!
//! The APIs mirror the C helper routines (`send_fd`/`recv_fd`) while keeping
//! ownership explicit with an `OwnedFd` result.
use std::io::{self, IoSlice, IoSliceMut};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixDatagram;

use nix::sys::socket::{
    self, AddressFamily, ControlMessage, ControlMessageOwned, MsgFlags, SockFlag, SockType,
};

/// A two-ended portal that can send and receive descriptors between threads.
pub struct FdPortal {
    sender: FdPortalSender,
    receiver: FdPortalReceiver,
}

impl FdPortal {
    /// Create a new portal backed by `socketpair(AF_UNIX, SOCK_SEQPACKET)`.
    pub fn new() -> io::Result<Self> {
        let (tx, rx) = socket::socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .map_err(io::Error::from)?;

        let sender = unsafe { UnixDatagram::from_raw_fd(tx.into_raw_fd()) };
        let receiver = unsafe { UnixDatagram::from_raw_fd(rx.into_raw_fd()) };

        Ok(FdPortal {
            sender: FdPortalSender { socket: sender },
            receiver: FdPortalReceiver { socket: receiver },
        })
    }

    /// Consume the portal and return the sender/receiver endpoints.
    pub fn split(self) -> (FdPortalSender, FdPortalReceiver) {
        (self.sender, self.receiver)
    }

    /// Clone the sender endpoint so it can be moved into another thread.
    #[allow(dead_code)]
    pub fn sender(&self) -> io::Result<FdPortalSender> {
        self.sender.try_clone()
    }

    /// Clone the receiver endpoint so it can be moved into another thread.
    #[allow(dead_code)]
    pub fn receiver(&self) -> io::Result<FdPortalReceiver> {
        self.receiver.try_clone()
    }
}

/// Sender half of an `FdPortal`.
pub struct FdPortalSender {
    socket: UnixDatagram,
}

impl FdPortalSender {
    /// Clone the sender. Useful when the same portal needs to fan out to multiple
    /// worker threads.
    #[allow(dead_code)]
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            socket: self.socket.try_clone()?,
        })
    }

    /// Send a file descriptor together with a single-byte payload.
    pub fn send_fd(&self, fd: RawFd) -> io::Result<()> {
        let payload = IoSlice::new(b"x");
        let cmsg = ControlMessage::ScmRights(&[fd]);

        socket::sendmsg::<()>(
            self.socket.as_raw_fd(),
            &[payload],
            &[cmsg],
            MsgFlags::empty(),
            None::<&()>,
        )
        .map(|_| ())
        .map_err(io::Error::from)
    }
}

impl AsRawFd for FdPortalSender {
    fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

impl IntoRawFd for FdPortalSender {
    fn into_raw_fd(self) -> RawFd {
        self.socket.into_raw_fd()
    }
}

impl FromRawFd for FdPortalSender {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        FdPortalSender {
            socket: unsafe { UnixDatagram::from_raw_fd(fd) },
        }
    }
}

/// Receiver half of an `FdPortal`.
pub struct FdPortalReceiver {
    socket: UnixDatagram,
}

impl FdPortalReceiver {
    /// Clone the receiver.
    #[allow(dead_code)]
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            socket: self.socket.try_clone()?,
        })
    }

    /// Receive a file descriptor that was sent via `send_fd`.
    pub fn recv_fd(&self) -> io::Result<OwnedFd> {
        let mut buf = [0u8; 1];
        let mut cmsg_space = vec![0u8; 128];
        let mut iov = [IoSliceMut::new(&mut buf)];

        let (bytes, cmsgs) = {
            let msg = socket::recvmsg::<()>(
                self.socket.as_raw_fd(),
                &mut iov,
                Some(&mut cmsg_space),
                MsgFlags::empty(),
            )
            .map_err(io::Error::from)?;

            let bytes = msg.bytes;
            let cmsgs: Vec<_> = msg.cmsgs().map_err(io::Error::from)?.collect();
            (bytes, cmsgs)
        };

        if bytes != 1 || buf[0] != b'x' {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid payload from fd portal",
            ));
        }

        for cmsg in cmsgs {
            if let ControlMessageOwned::ScmRights(fds) = cmsg {
                if let Some(&fd) = fds.first() {
                    return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "descriptor not found in control message",
        ))
    }
}

impl AsRawFd for FdPortalReceiver {
    fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

impl IntoRawFd for FdPortalReceiver {
    fn into_raw_fd(self) -> RawFd {
        self.socket.into_raw_fd()
    }
}

impl FromRawFd for FdPortalReceiver {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        FdPortalReceiver {
            socket: unsafe { UnixDatagram::from_raw_fd(fd) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libc;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::unix::io::{FromRawFd, IntoRawFd};
    use std::process;
    use std::thread;

    #[test]
    fn threads_can_exchange_fd() -> io::Result<()> {
        let portal = FdPortal::new()?;
        let (sender, receiver) = portal.split();

        let temp_path = std::env::temp_dir().join("df_portal_thread_test");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)?;
        writeln!(file, "thread safe transfer")?;
        file.flush()?;
        let src_fd = file.into_raw_fd();

        let handle = thread::spawn(move || {
            sender.send_fd(src_fd)?;
            Ok::<(), io::Error>(())
        });

        let received_fd = receiver.recv_fd()?;
        let mut received_file = unsafe { File::from_raw_fd(received_fd.into_raw_fd()) };

        let mut contents = String::new();
        received_file.seek(SeekFrom::Start(0))?;
        received_file.read_to_string(&mut contents)?;
        assert_eq!(contents, "thread safe transfer\n");

        handle.join().expect("sender thread panicked")?;
        std::fs::remove_file(&temp_path)?;
        Ok(())
    }

    #[test]
    fn processes_can_exchange_fd() -> io::Result<()> {
        let portal = FdPortal::new()?;
        let (sender, receiver) = portal.split();

        let temp_path = std::env::temp_dir().join("df_portal_process_test");
        match unsafe { libc::fork() } {
            -1 => Err(io::Error::last_os_error()),
            0 => {
                drop(receiver);
                let mut file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&temp_path)?;
                writeln!(file, "cross-process transfer")?;
                file.flush()?;
                let src_fd = file.into_raw_fd();
                sender.send_fd(src_fd)?;
                unsafe { libc::close(src_fd) };
                process::exit(0);
            }
            pid => {
                drop(sender);
                let received_fd = receiver.recv_fd()?;
                let mut received_file = unsafe { File::from_raw_fd(received_fd.into_raw_fd()) };
                received_file.seek(SeekFrom::Start(0))?;
                let mut contents = String::new();
                received_file.read_to_string(&mut contents)?;
                assert_eq!(contents, "cross-process transfer\n");
                let mut status = 0;
                if unsafe { libc::waitpid(pid, &mut status, 0) } < 0 {
                    return Err(io::Error::last_os_error());
                }
                std::fs::remove_file(&temp_path)?;
                Ok(())
            }
        }
    }
}
