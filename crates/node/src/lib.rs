//! The botchain node: a sequencer that seals blocks on a clock, behind a
//! JSON-RPC 2.0 server.

pub mod cli;
pub mod jsonrpc;
pub mod methods;
pub mod node;
pub mod rpc;

use std::path::Path;

use botchain_core::Genesis;

pub use cli::{Args, CliError, Command};
pub use node::{Node, SharedNode};

/// Loads the genesis file, builds the node and runs the block clock and the
/// RPC server until the process is asked to stop.
pub async fn run(args: Args) -> anyhow::Result<()> {
    let genesis = load_genesis(&args.genesis)?;
    genesis.validate()?;
    prepare_data_dir(&args.data_dir)?;

    let node = Node::from_genesis(&genesis, args.block_ms)?;
    println!(
        "node starting chain_id={} genesis={} data_dir={} block_ms={}",
        genesis.chain_id,
        args.genesis.display(),
        args.data_dir.display(),
        args.block_ms
    );
    println!(
        "genesis block 0 hash={} accounts={} supply={}",
        node.checkpoint().await.hash,
        genesis.allocations.len(),
        genesis.total_supply().unwrap_or_default()
    );

    let clock = tokio::spawn(node::run_block_clock(node.clone()));
    let result = tokio::select! {
        served = rpc::serve(node.clone(), args.rpc_port) => served,
        _ = tokio::signal::ctrl_c() => {
            println!("node stopping");
            Ok(())
        }
    };
    clock.abort();
    result
}

/// Reads a genesis document from disk, with the path in the error message.
fn load_genesis(path: &Path) -> anyhow::Result<Genesis> {
    Genesis::load(path)
        .map_err(|error| anyhow::anyhow!("could not load genesis file {}: {error}", path.display()))
}

/// Makes sure the data directory exists and is a directory.
fn prepare_data_dir(path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)
        .map_err(|error| anyhow::anyhow!("could not use data dir {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_genesis_file_says_which_one() {
        let error = load_genesis(Path::new("/nope/genesis.json")).expect_err("missing");
        assert!(error.to_string().contains("/nope/genesis.json"));
    }

    #[test]
    fn the_data_dir_is_created() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a/b");
        prepare_data_dir(&nested).expect("create");
        assert!(nested.is_dir());
        // Running again on an existing directory is fine.
        prepare_data_dir(&nested).expect("again");
    }
}
