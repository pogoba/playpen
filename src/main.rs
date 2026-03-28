use clap::Parser;
use std::ffi::CString;

#[derive(Parser)]
#[command(name = "playpen")]
#[command(about = "A simple command runner", long_about = None)]
#[command(trailing_var_arg = true)]
#[command(arg_required_else_help = true)]
struct Cli {
    /// Command and arguments to execute
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,
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
            // Add null terminator for argv array TODO: simplify
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
            unreachable!();
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
