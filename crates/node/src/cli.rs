//! The command line of `botchain-node`.
//!
//! ```text
//! botchain-node --rpc-port <u16> --data-dir <path> --genesis <file>
//!               [--block-ms <u64, default 1000>]
//! ```

use std::path::PathBuf;

/// How long a block takes when `--block-ms` is not given.
pub const DEFAULT_BLOCK_MS: u64 = 1000;

/// The usage text printed for `--help` and for bad arguments.
pub const USAGE: &str = "botchain-node --rpc-port <u16> --data-dir <path> --genesis <file> [--block-ms <u64, default 1000>]";

/// What the command line asked for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Command {
    /// Run the node with these arguments.
    Run(Args),
    /// Print the usage text and exit successfully.
    Help,
}

/// The parsed arguments of a node run.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Args {
    /// TCP port the JSON-RPC server listens on, on 127.0.0.1.
    pub rpc_port: u16,
    /// Directory the node keeps its data in.
    pub data_dir: PathBuf,
    /// Path of the genesis JSON file.
    pub genesis: PathBuf,
    /// Milliseconds between two sealed blocks.
    pub block_ms: u64,
}

/// Why a command line could not be used.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CliError {
    /// A required flag was not given.
    #[error("missing required argument {0}")]
    Missing(&'static str),
    /// A flag was given without its value.
    #[error("{0} needs a value")]
    MissingValue(String),
    /// A flag was given twice.
    #[error("{0} was given more than once")]
    Repeated(String),
    /// A value could not be read as the flag's type.
    #[error("invalid value for {flag}: {value}")]
    BadValue {
        /// The flag the value belongs to.
        flag: String,
        /// The value as it was given.
        value: String,
    },
    /// Something that is not one of our flags.
    #[error("unknown argument {0}")]
    Unknown(String),
}

/// Parses the arguments, without the program name.
pub fn parse<I, S>(args: I) -> Result<Command, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut rpc_port: Option<u16> = None;
    let mut data_dir: Option<PathBuf> = None;
    let mut genesis: Option<PathBuf> = None;
    let mut block_ms: Option<u64> = None;

    let mut args = args.into_iter().map(Into::into);
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| -> Result<String, CliError> {
            args.next()
                .ok_or_else(|| CliError::MissingValue(flag.to_string()))
        };
        match arg.as_str() {
            "--help" | "-h" => return Ok(Command::Help),
            "--rpc-port" => {
                let raw = value("--rpc-port")?;
                let port = raw.parse::<u16>().map_err(|_| CliError::BadValue {
                    flag: "--rpc-port".to_string(),
                    value: raw,
                })?;
                set(&mut rpc_port, port, "--rpc-port")?;
            }
            "--data-dir" => {
                let raw = value("--data-dir")?;
                set(&mut data_dir, PathBuf::from(raw), "--data-dir")?;
            }
            "--genesis" => {
                let raw = value("--genesis")?;
                set(&mut genesis, PathBuf::from(raw), "--genesis")?;
            }
            "--block-ms" => {
                let raw = value("--block-ms")?;
                let ms = raw
                    .parse::<u64>()
                    .ok()
                    .filter(|ms| *ms > 0)
                    .ok_or_else(|| CliError::BadValue {
                        flag: "--block-ms".to_string(),
                        value: raw,
                    })?;
                set(&mut block_ms, ms, "--block-ms")?;
            }
            other => return Err(CliError::Unknown(other.to_string())),
        }
    }

    Ok(Command::Run(Args {
        rpc_port: rpc_port.ok_or(CliError::Missing("--rpc-port"))?,
        data_dir: data_dir.ok_or(CliError::Missing("--data-dir"))?,
        genesis: genesis.ok_or(CliError::Missing("--genesis"))?,
        block_ms: block_ms.unwrap_or(DEFAULT_BLOCK_MS),
    }))
}

fn set<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), CliError> {
    if slot.is_some() {
        return Err(CliError::Repeated(flag.to_string()));
    }
    *slot = Some(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Args, CliError> {
        match parse(args.iter().copied()) {
            Ok(Command::Run(args)) => Ok(args),
            Ok(Command::Help) => panic!("expected a run, got help"),
            Err(error) => Err(error),
        }
    }

    #[test]
    fn full_command_line_parses() {
        let args = run(&[
            "--rpc-port",
            "8899",
            "--data-dir",
            "/tmp/bot",
            "--genesis",
            "g.json",
            "--block-ms",
            "250",
        ])
        .expect("parse");
        assert_eq!(args.rpc_port, 8899);
        assert_eq!(args.data_dir, PathBuf::from("/tmp/bot"));
        assert_eq!(args.genesis, PathBuf::from("g.json"));
        assert_eq!(args.block_ms, 250);
    }

    #[test]
    fn block_ms_defaults_to_one_second_and_order_does_not_matter() {
        let args = run(&[
            "--genesis",
            "g.json",
            "--data-dir",
            "d",
            "--rpc-port",
            "1234",
        ])
        .expect("parse");
        assert_eq!(args.block_ms, DEFAULT_BLOCK_MS);
    }

    #[test]
    fn missing_flags_are_reported() {
        assert_eq!(
            run(&["--data-dir", "d", "--genesis", "g"]),
            Err(CliError::Missing("--rpc-port"))
        );
        assert_eq!(
            run(&["--rpc-port", "1", "--genesis", "g"]),
            Err(CliError::Missing("--data-dir"))
        );
        assert_eq!(
            run(&["--rpc-port", "1", "--data-dir", "d"]),
            Err(CliError::Missing("--genesis"))
        );
    }

    #[test]
    fn bad_values_are_reported() {
        assert!(matches!(
            run(&["--rpc-port", "70000", "--data-dir", "d", "--genesis", "g"]),
            Err(CliError::BadValue { .. })
        ));
        assert!(matches!(
            run(&[
                "--rpc-port",
                "1",
                "--data-dir",
                "d",
                "--genesis",
                "g",
                "--block-ms",
                "0"
            ]),
            Err(CliError::BadValue { .. })
        ));
        assert_eq!(
            run(&["--rpc-port"]),
            Err(CliError::MissingValue("--rpc-port".to_string()))
        );
        assert_eq!(
            run(&["--port", "1"]),
            Err(CliError::Unknown("--port".to_string()))
        );
        assert_eq!(
            run(&["--rpc-port", "1", "--rpc-port", "2"]),
            Err(CliError::Repeated("--rpc-port".to_string()))
        );
    }

    #[test]
    fn help_is_a_command_of_its_own() {
        assert_eq!(parse(["--help"]), Ok(Command::Help));
        assert_eq!(parse(["--rpc-port", "1", "-h"]), Ok(Command::Help));
    }
}
