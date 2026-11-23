use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use cloneyard_api::clone::Handle;

mod account;
mod claude;
mod clone;
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
    /// Add an account
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
    /// List the accounts with their providers and directories
    Accounts,
    /// Run, show or stop a clone
    Clone {
        #[command(subcommand)]
        command: CloneCommand,
    },
    /// List the clones with their projects, accounts, modes and states
    Clones,
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
enum AccountCommand {
    /// Log in to Claude Code in a directory of its own and add it as an account
    Add {
        /// The name of the account, which also names its directory
        name: String,
    },
}

#[derive(Debug, Subcommand)]
enum CloneCommand {
    /// Start a headless clone on an account, in a checkout of a project's repo, and print its handle
    Run {
        /// The project whose repo the clone gets a checkout of
        #[arg(long)]
        project: String,
        /// The account the clone runs on
        #[arg(long)]
        account: String,
        /// The first message to the clone
        prompt: String,
    },
    /// Show a clone with its reason, branch, checkout and sessions
    Show {
        /// The clone's handle, such as CT-0042 or ct-42
        handle: Handle,
    },
    /// End a working clone's processes
    Stop {
        /// The clone's handle, such as CT-0042 or ct-42
        handle: Handle,
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
        Command::Account {
            command: AccountCommand::Add { name },
        } => account::add(&name),
        Command::Accounts => account::list(),
        Command::Clone {
            command:
                CloneCommand::Run {
                    project,
                    account,
                    prompt,
                },
        } => clone::run(project, account, prompt),
        Command::Clone {
            command: CloneCommand::Show { handle },
        } => clone::show(handle),
        Command::Clone {
            command: CloneCommand::Stop { handle },
        } => clone::stop(handle),
        Command::Clones => clone::list(),
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
