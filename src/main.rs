mod fd_portal;

use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use fd_portal::FdPortal;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use std::ffi::CString;
use std::io::{self, IsTerminal};
use std::os::fd::IntoRawFd;
use std::os::unix::io::OwnedFd;
use std::ptr;
use std::thread;

#[derive(Parser)]
#[command(name = "playpen")]
#[command(about = "A simple command runner", long_about = None)]
#[command(trailing_var_arg = true)]
#[command(arg_required_else_help = true)]
struct Cli {
    /// Skip confirmation prompt
    #[arg(long)]
    no_confirm: bool,

    /// Command and arguments to execute
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum ConfirmationState {
    Confirmed,
    Aborted,
}

fn show_confirmation_prompt() -> ConfirmationState {
    if !std::io::stdout().is_terminal() {
        eprintln!("Warning: Not running in a terminal, skipping confirmation prompt");
        return ConfirmationState::Confirmed;
    }

    enable_raw_mode().expect("Failed to enable raw mode");
    let mut stdout = std::io::stdout();
    execute!(stdout, EnableMouseCapture).expect("Failed to enable mouse capture");
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).expect("Failed to create terminal");
    execute!(std::io::stdout(), crossterm::terminal::ScrollUp(3))
        .expect("Failed to scroll terminal");

    let mut selected = 0; // 0 = start, 1 = abort
    let state = loop {
        terminal
            .draw(|f| {
                let size = f.area();
                let selector_height = 3;
                let selector_area = ratatui::layout::Rect::new(
                    0,
                    size.height.saturating_sub(selector_height as u16),
                    size.width,
                    selector_height,
                );

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(0)])
                    .split(selector_area);

                let status = Paragraph::new(Line::from(vec![
                    Span::raw("Playpen: "),
                    Span::styled(
                        if selected == 0 { "Start" } else { "Abort" },
                        if selected == 0 {
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                        },
                    ),
                    Span::raw(" (↑/↓ to select, Enter to confirm, Esc to abort)"),
                ]))
                .style(Style::default());
                f.render_widget(status, chunks[0]);

                let options = vec![
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            "► Start command",
                            if selected == 0 {
                                Style::default()
                                    .fg(Color::Green)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ),
                    ]),
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            "  Abort",
                            if selected == 1 {
                                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ),
                    ]),
                ];
                let options_widget = Paragraph::new(options).style(Style::default());
                f.render_widget(options_widget, chunks[1]);
            })
            .expect("Failed to draw terminal");

        if let Event::Key(key) = event::read().expect("Failed to read event") {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        if selected > 0 {
                            selected -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if selected < 1 {
                            selected += 1;
                        }
                    }
                    KeyCode::Enter => {
                        break if selected == 0 {
                            ConfirmationState::Confirmed
                        } else {
                            ConfirmationState::Aborted
                        };
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {
                        break ConfirmationState::Aborted;
                    }
                    _ => {}
                }
            }
        }
    };

    disable_raw_mode().expect("Failed to disable raw mode");
    execute!(terminal.backend_mut(), DisableMouseCapture,)
        .expect("Failed to disable mouse capture");
    terminal.show_cursor().expect("Failed to show cursor");

    state
}

fn install_seccomp_listener(sender: &fd_portal::FdPortalSender) -> Result<(), String> {
    let ctx = unsafe { libseccomp_sys::seccomp_init(libseccomp_sys::SCMP_ACT_ALLOW) };
    if ctx.is_null() {
        return Err("Failed to initialize seccomp filter".to_string());
    }

    let write_rule = unsafe {
        libseccomp_sys::seccomp_rule_add(
            ctx,
            libseccomp_sys::SCMP_ACT_KILL,
            libc::SYS_write as libc::c_int,
            0,
        )
    };
    if write_rule != 0 {
        unsafe { libseccomp_sys::seccomp_release(ctx) };
        return Err(format!(
            "Failed to add write blacklist rule: {}",
            write_rule
        ));
    }

    let mount_rule = unsafe {
        libseccomp_sys::seccomp_rule_add(
            ctx,
            libseccomp_sys::SECCOMP_RET_USER_NOTIF,
            libc::SYS_mount as libc::c_int,
            0,
        )
    };
    if mount_rule != 0 {
        unsafe { libseccomp_sys::seccomp_release(ctx) };
        return Err(format!(
            "Failed to install mount notification rule: {}",
            mount_rule
        ));
    }

    let notify_fd =
        unsafe { libseccomp_sys::seccomp_notify_fd(ctx as libseccomp_sys::const_scmp_filter_ctx) };
    if notify_fd < 0 {
        unsafe { libseccomp_sys::seccomp_release(ctx) };
        return Err("Failed to retrieve seccomp notification fd".to_string());
    }

    let load_result = unsafe { libseccomp_sys::seccomp_load(ctx) };
    if load_result != 0 {
        unsafe {
            libc::close(notify_fd);
            libseccomp_sys::seccomp_release(ctx);
        }
        return Err(format!("Failed to load seccomp filter: {}", load_result));
    }

    unsafe { libseccomp_sys::seccomp_release(ctx) };

    sender
        .send_fd(notify_fd)
        .map_err(|err| format!("Failed to send seccomp listener: {err}"))?;
    unsafe { libc::close(notify_fd) };

    trigger_mount_probe();

    Ok(())
}

fn trigger_mount_probe() {
    const TARGET: &str = "/tmp/playpen_mount_probe";
    let _ = std::fs::create_dir_all(TARGET);
    let source_c = CString::new("/dev/sda").unwrap();
    let target_c = CString::new(TARGET).unwrap();

    unsafe {
        libc::mount(
            source_c.as_ptr(),
            target_c.as_ptr(),
            ptr::null(),
            0,
            ptr::null(),
        );
    }

    let _ = std::fs::remove_dir_all(TARGET);
}

fn handle_seccomp_notifications(listener: OwnedFd) -> io::Result<()> {
    let fd = listener.into_raw_fd();
    unsafe {
        let mut req = ptr::null_mut();
        let mut resp = ptr::null_mut();
        if libseccomp_sys::seccomp_notify_alloc(&mut req, &mut resp) != 0 {
            libc::close(fd);
            return Err(io::Error::last_os_error());
        }

        let result = loop {
            if libseccomp_sys::seccomp_notify_receive(fd, req) < 0 {
                break Err(io::Error::last_os_error());
            }

            eprintln!(
                "[playpen] intercepted syscall {} from pid {} with args {:?}",
                (*req).data.nr,
                (*req).pid,
                (*req).data.args
            );

            (*resp).id = (*req).id;
            (*resp).val = 0;
            (*resp).error = -libc::EPERM;
            (*resp).flags = 0;

            if libseccomp_sys::seccomp_notify_respond(fd, resp) < 0 {
                break Err(io::Error::last_os_error());
            }

            break Ok(());
        };

        libseccomp_sys::seccomp_notify_free(req, resp);
        libc::close(fd);
        result
    }
}

fn main() {
    let cli = Cli::parse();

    if cli.args.is_empty() {
        eprintln!("No command specified");
        std::process::exit(1);
    }

    if !cli.no_confirm {
        match show_confirmation_prompt() {
            ConfirmationState::Aborted => {
                eprintln!();
                eprintln!("Aborted by user");
                std::process::exit(1);
            }
            ConfirmationState::Confirmed => {
                eprintln!();
            }
        }
    }

    let portal = match FdPortal::new() {
        Ok(portal) => portal,
        Err(err) => {
            eprintln!("Failed to create descriptor portal: {}", err);
            std::process::exit(1);
        }
    };

    let (sender, receiver) = portal.split();
    let (program, program_args) = cli.args.split_first().unwrap();

    match unsafe { libc::fork() } {
        -1 => {
            eprintln!("Failed to fork");
            std::process::exit(1);
        }
        0 => {
            drop(receiver);
            if let Err(err) = install_seccomp_listener(&sender) {
                eprintln!("Failed to set up seccomp listener: {}", err);
                std::process::exit(1);
            }

            let mut c_args = Vec::<CString>::new();
            c_args.push(CString::new(program.as_str()).unwrap());
            for arg in program_args {
                c_args.push(CString::new(arg.as_str()).unwrap());
            }
            let c_args_ptrs: Vec<*const libc::c_char> = c_args
                .iter()
                .map(|s| s.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();

            let program_ptr = c_args_ptrs[0];
            let argv_ptr = c_args_ptrs.as_ptr() as *const *const libc::c_char;
            unsafe {
                libc::execvp(program_ptr, argv_ptr);
            }
            std::process::exit(1);
        }
        pid => {
            drop(sender);
            println!("Child PID: {}", pid);

            let notifier = match receiver.recv_fd() {
                Ok(listener) => Some(thread::spawn(move || {
                    if let Err(err) = handle_seccomp_notifications(listener) {
                        eprintln!("seccomp notifier failed: {}", err);
                    }
                })),
                Err(err) => {
                    eprintln!("Failed to receive seccomp listener: {}", err);
                    None
                }
            };

            let mut status: libc::c_int = 0;
            unsafe { libc::waitpid(pid, &mut status, 0) };

            if let Some(handle) = notifier {
                let _ = handle.join();
            }

            std::process::exit(if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                1
            });
        }
    }
}
