use clap::Parser;

// A fixed name, so `cy --version` also prints "cloneyard <version>".
#[derive(Debug, Parser)]
#[command(name = "cloneyard", version, about)]
struct Cli {}

pub fn run() {
    Cli::parse();
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
