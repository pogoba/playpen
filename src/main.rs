use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::ffi::CString;
use std::io::{IsTerminal, Write};

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
    Pending,
    Confirmed,
    Aborted,
}

fn show_confirmation_prompt() -> ConfirmationState {
    // Check if stdout is a TTY
    if !std::io::stdout().is_terminal() {
        eprintln!("Warning: Not running in a terminal, skipping confirmation prompt");
        return ConfirmationState::Confirmed;
    }

    enable_raw_mode().expect("Failed to enable raw mode");
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).expect("Failed to enter alternate screen");
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).expect("Failed to create terminal");

    let mut state = ConfirmationState::Pending;
    let mut selected = 0; // 0 = start, 1 = abort

    loop {
        terminal.draw(|f| {
            let size = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(0),
                    Constraint::Length(3),
                ])
                .split(size);

            // Title
            let title = Paragraph::new(Line::from(vec![
                Span::styled("Playpen", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" - Command Runner"),
            ]))
            .block(Block::default().borders(Borders::ALL).title(" Confirmation "))
            .style(Style::default());
            f.render_widget(title, chunks[0]);

            // Instructions
            let instructions = Paragraph::new(vec![
                Line::from(""),
                Line::from(vec![
                    Span::raw("What would you like to do? (use ↑/↓ to navigate, Enter to confirm)"),
                ]),
                Line::from(""),
            ])
            .block(Block::default().borders(Borders::ALL).title(" Instructions "))
            .style(Style::default());
            f.render_widget(instructions, chunks[1]);

            // Options
            let options = vec![
                Line::from(vec![
                    Span::styled(
                        "► Start command",
                        if selected == 0 {
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        },
                    ),
                ]),
                Line::from(vec![
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
            let options_widget = Paragraph::new(options)
                .block(Block::default().borders(Borders::ALL).title(" Options "))
                .style(Style::default());
            f.render_widget(options_widget, chunks[2]);
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
                        state = if selected == 0 {
                            ConfirmationState::Confirmed
                        } else {
                            ConfirmationState::Aborted
                        };
                        break;
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {
                        state = ConfirmationState::Aborted;
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode().expect("Failed to disable raw mode");
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .expect("Failed to leave alternate screen");
    terminal.show_cursor().expect("Failed to show cursor");

    state
}

fn apply_seccomp_filter() -> Result<(), String> {
    // Initialize seccomp filter with SCMP_ACT_ALLOW
    let filter = unsafe { libseccomp_sys::seccomp_init(libseccomp_sys::SCMP_ACT_ALLOW) };
    if filter.is_null() {
        return Err("Failed to initialize seccomp filter".to_string());
    }

    // Apply the filter to the current process
    let ret = unsafe { libseccomp_sys::seccomp_load(filter) };
    if ret != 0 {
        return Err(format!("Failed to load seccomp filter: {}", ret));
    }

    Ok(())
}

fn main() {
    let cli = Cli::parse();

    if cli.args.is_empty() {
        eprintln!("No command specified");
        std::process::exit(1);
    }

    // Show confirmation prompt (unless --no-confirm is used)
    if !cli.no_confirm {
        match show_confirmation_prompt() {
            ConfirmationState::Aborted => {
                eprintln!("Aborted by user");
                std::process::exit(1);
            }
            ConfirmationState::Confirmed => {}
            ConfirmationState::Pending => {
                eprintln!("No confirmation received");
                std::process::exit(1);
            }
        }
    }

    let (program, program_args) = cli.args.split_first().unwrap();

    // Fork a child process
    match unsafe { libc::fork() } {
        -1 => {
            eprintln!("Failed to fork");
            std::process::exit(1);
        }
        0 => {
            // Child process: apply seccomp filter then exec
            if let Err(e) = apply_seccomp_filter() {
                eprintln!("Failed to apply seccomp filter: {}", e);
                std::process::exit(1);
            }

            // Convert program and arguments to C strings
            let mut c_args: Vec<CString> = Vec::new();
            c_args.push(CString::new(program.as_str()).unwrap());
            for arg in program_args {
                c_args.push(CString::new(arg.as_str()).unwrap());
            }
            // Add null terminator for argv array
            let c_args_ptrs: Vec<*const libc::c_char> = c_args
                .iter()
                .map(|s| s.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();

            // Exec the command with seccomp applied
            // Note: execvp does not return on success
            let program_ptr = c_args_ptrs[0];
            let argv_ptr = c_args_ptrs.as_ptr() as *const *const libc::c_char;
            unsafe {
                libc::execvp(program_ptr, argv_ptr);
            }
            // If we get here, execvp failed
            std::process::exit(1);
        }
        pid => {
            // Parent process: wait for child and return its exit code
            let mut status: libc::c_int = 0;
            unsafe { libc::waitpid(pid, &mut status, 0) };
            std::process::exit(if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                1
            });
        }
    }
}
