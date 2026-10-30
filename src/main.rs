use clap::Parser;
use std::process::ExitCode;
use todo_unearther::{app, cli::Cli};

fn main() -> ExitCode {
    match app::run(Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(2)
        }
    }
}
