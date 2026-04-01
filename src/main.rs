mod fd_portal;
mod fmt_syscall;
mod syscalls;
mod seccomp;

use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use fd_portal::FdPortal;
use nix::errno::Errno;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use std::ffi::CString;
use std::io::{self, IsTerminal, Write};
use std::os::fd::IntoRawFd;
use std::os::unix::io::OwnedFd;
use std::ptr;

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


fn is_immutable(flags: u64) -> bool {
    // catch unspecified cases
    if (flags as libc::c_int & (libc::O_APPEND | libc::O_CREAT | libc::O_TRUNC)) != 0 {
        return false;
    }
    // whitelist
    match flags as libc::c_int & libc::O_ACCMODE {
        libc::O_RDONLY => return true,
        _ => {},
    };
    // deny by default
    return false;
}


fn request_syscall_permission(syscall: i64, args: [u64; 6], pid: libc::pid_t, prompt: bool, syscall_map: &std::collections::HashMap<i32, &str>) -> bool {
    if syscall == libc::SYS_openat as i64 && is_immutable(args[2]) {
        return true;
    }

    if !std::io::stdout().is_terminal() {
        eprintln!(
            "Intercepted syscall {}, allowing by default because terminal is not available.",
            syscall
        );
        return true;
    }

    if !prompt {
        return true;
    }

    let formatted_args = fmt_syscall::format_syscall_args(syscall, args, pid);
    let name = syscall_map.get(&(syscall as i32)).copied().unwrap_or("unknown");
    let syscall_label = format!("{} ({})", name, syscall);

    println!();
    enable_raw_mode().expect("Failed to enable raw mode");
    let mut stdout = std::io::stdout();
    execute!(stdout, EnableMouseCapture).expect("Failed to enable mouse capture");
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).expect("Failed to create terminal");
    execute!(std::io::stdout(), crossterm::terminal::ScrollUp(3))
        .expect("Failed to scroll terminal");

    // 1 line for syscall header + 1 line per formatted arg + 2 lines for options
    let detail_lines_len = 1 + formatted_args.len();
    let mut selected = 0;
    let state = loop {
        terminal
            .draw(|f| {
                let size = f.area();
                let selector_height = detail_lines_len + 3;
                let selector_area = ratatui::layout::Rect::new(
                    0,
                    size.height.saturating_sub(selector_height as u16),
                    size.width,
                    selector_height as u16,
                );

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(detail_lines_len as u16), Constraint::Min(0)])
                    .split(selector_area);

                let mut detail_lines = vec![Line::from(vec![
                    Span::raw("Intercepted syscall: "),
                    Span::styled(
                        syscall_label.clone(),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])];

                for (label, value) in &formatted_args {
                    detail_lines.push(Line::from(vec![
                        Span::raw(format!("{}: ", label)),
                        Span::styled(
                            value.clone(),
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                        ),
                    ]));
                }

                let detail = Paragraph::new(detail_lines).style(Style::default());
                f.render_widget(detail, chunks[0]);

                let options = vec![
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            "► Allow",
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
                            "  Deny",
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
                        break selected == 0;
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {
                        break false;
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
    println!();
    println!();
    println!();
    std::io::stdout().flush().ok();

    state
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

    // Build syscall number → name map from syscalls.rs filter sets
    let syscall_map = syscalls::resolve_syscall_map(&[
        &syscalls::CHOWN,
        &syscalls::FILE_SYSTEM,
        &syscalls::KEYRING,
        &syscalls::MODULE,
        &syscalls::MOUNT,
        &syscalls::SETUID,
    ]);

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
            if let Err(err) = seccomp::enable_seccomp(&sender, &syscall_map) {
                eprintln!("Failed to enable seccomp: {}", err);
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
            let ret = unsafe { libc::execvp(program_ptr, argv_ptr) };
            if ret == -1 {
                eprintln!("Failed to execve child"); // errno will be set by execve
                std::process::exit(Errno::last_raw());
            }
            unreachable!();
        }
        pid => {
            drop(sender);
            println!("Child PID: {}", pid);

            match receiver.recv_fd() {
                Ok(listener) => {
                    if let Err(err) = seccomp::handle_seccomp_notifications(listener, !cli.no_confirm, &syscall_map) {
                        eprintln!("seccomp notifier failed: {}", err);
                    }
                }
                Err(err) => eprintln!(
                    "Failed to receive seccomp listener (child may have exited before sharing it): {}",
                    err
                ),
            }

            let mut status: libc::c_int = 0;
            unsafe { libc::waitpid(pid, &mut status, 0) };

            // println!("fzz {}", status);
            if libc::WIFEXITED(status) {
                std::process::exit(libc::WEXITSTATUS(status));
                // std::process::exit(status & 0xFF) // this is what WEXITSTATUS should do
            } else if libc::WIFSIGNALED(status) {
                eprintln!("Child was terminated by signal {}", libc::WTERMSIG(status));
                std::process::exit(1);
            } else {
                std::process::exit(123);
            }
        }
    }
}
