//! The `botchain-node` binary.

use std::process::ExitCode;

use botchain_node::cli::{self, Command, USAGE};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match cli::parse(args) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("botchain-node: {error}");
            eprintln!("usage: {USAGE}");
            return ExitCode::from(2);
        }
    };
    match command {
        Command::Help => {
            println!("usage: {USAGE}");
            ExitCode::SUCCESS
        }
        Command::Run(args) => match botchain_node::run(args).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("botchain-node: {error}");
                ExitCode::from(1)
            }
        },
    }
}
