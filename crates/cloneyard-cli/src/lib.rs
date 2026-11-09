use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod ping;

// A fixed name, so `cy --version` also prints "cloneyard <version>".
#[derive(Debug, Parser)]
#[command(name = "cloneyard", version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the daemon in the foreground
    Daemon,
    /// Check that the daemon answers, and how fast
    Ping,
}

#[must_use]
pub fn run() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Daemon => cloneyard_daemon::run(),
        Command::Ping => ping::run(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
