use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod connection;
mod ping;
mod project;
mod rpc;

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
    /// Add a project
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// List the projects with the paths of their repos
    Projects,
    /// Look at the daemon's API
    Rpc {
        #[command(subcommand)]
        command: RpcCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    /// Add the git repo at a path as a project
    Add {
        /// A path in the repo's working tree
        path: PathBuf,
        /// The name of the project, instead of the name of the repo's directory
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum RpcCommand {
    /// Print the OpenRPC document of the API
    Schema,
}

pub fn run() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Daemon => cloneyard_daemon::run(),
        Command::Ping => ping::run(),
        Command::Project {
            command: ProjectCommand::Add { path, name },
        } => project::add(&path, name),
        Command::Projects => project::list(),
        Command::Rpc {
            command: RpcCommand::Schema,
        } => rpc::schema(),
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
