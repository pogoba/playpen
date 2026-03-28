use clap::Parser;
use std::process::Command;

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

fn main() {
    let cli = Cli::parse();

    if cli.args.is_empty() {
        eprintln!("No command specified");
        std::process::exit(1);
    }

    let (program, program_args) = cli.args.split_first().unwrap();
    let status = Command::new(program)
        .args(program_args)
        .status()
        .expect("Failed to execute command");
    std::process::exit(status.code().unwrap_or(1));
}
