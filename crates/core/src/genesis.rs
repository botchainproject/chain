//! The genesis file: chain id, timestamp and the initial balances.
//!
//! ```json
//! {"chain_id":"botchain-devnet","timestamp_ms":1790000000000,
//!  "allocations":[{"address":"<base58>","amount":"1000"}]}
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::amount::Amount;
use crate::crypto::Address;

/// One line of the genesis allocation table.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Allocation {
    /// Who gets the units.
    pub address: Address,
    /// How many base units, a decimal string in JSON.
    pub amount: Amount,
}

/// The parsed genesis file.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Genesis {
    /// The chain id every signature is bound to.
    pub chain_id: String,
    /// The timestamp of block 0, in milliseconds since the unix epoch.
    pub timestamp_ms: u64,
    /// The initial balances.
    pub allocations: Vec<Allocation>,
}

/// Why a genesis file is not usable.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GenesisError {
    /// The file could not be read.
    #[error("cannot read genesis file {path}: {message}")]
    Read {
        /// The path we tried to read.
        path: String,
        /// The operating system's complaint.
        message: String,
    },
    /// The file is not the expected JSON.
    #[error("genesis file is not valid JSON: {0}")]
    Json(String),
    /// The chain id is empty.
    #[error("genesis chain_id must not be empty")]
    EmptyChainId,
    /// The same address appears twice in the allocations.
    #[error("genesis allocates to {0} twice")]
    DuplicateAllocation(String),
    /// The allocations add up to more than u64 can hold.
    #[error("genesis allocations overflow the total supply")]
    SupplyOverflow,
}

impl Genesis {
    /// Parses a genesis document and checks it.
    pub fn from_json(json: &str) -> Result<Self, GenesisError> {
        let genesis: Genesis =
            serde_json::from_str(json).map_err(|e| GenesisError::Json(e.to_string()))?;
        genesis.validate()?;
        Ok(genesis)
    }

    /// Reads and checks a genesis file from disk.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, GenesisError> {
        let path = path.as_ref();
        let json = std::fs::read_to_string(path).map_err(|e| GenesisError::Read {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        Genesis::from_json(&json)
    }

    /// Checks the invariants: a chain id, no duplicate address, no overflow.
    pub fn validate(&self) -> Result<(), GenesisError> {
        if self.chain_id.is_empty() {
            return Err(GenesisError::EmptyChainId);
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut total = Amount::ZERO;
        for allocation in &self.allocations {
            if !seen.insert(allocation.address) {
                return Err(GenesisError::DuplicateAllocation(
                    allocation.address.to_base58(),
                ));
            }
            total = total
                .checked_add(allocation.amount)
                .ok_or(GenesisError::SupplyOverflow)?;
        }
        Ok(())
    }

    /// The sum of all allocations; the chain's total supply at block 0.
    pub fn total_supply(&self) -> Option<Amount> {
        self.allocations
            .iter()
            .try_fold(Amount::ZERO, |sum, a| sum.checked_add(a.amount))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Keypair;

    fn address(seed: u8) -> Address {
        Keypair::from_secret_bytes([seed; 32]).address()
    }

    #[test]
    fn parses_the_spec_example() {
        let a = address(1);
        let json = format!(
            r#"{{"chain_id":"botchain-devnet","timestamp_ms":1790000000000,
                 "allocations":[{{"address":"{}","amount":"1000"}}]}}"#,
            a.to_base58()
        );
        let genesis = Genesis::from_json(&json).expect("valid genesis");
        assert_eq!(genesis.chain_id, "botchain-devnet");
        assert_eq!(genesis.timestamp_ms, 1_790_000_000_000);
        assert_eq!(genesis.allocations.len(), 1);
        assert_eq!(genesis.allocations[0].address, a);
        assert_eq!(genesis.allocations[0].amount, Amount::from_u64(1000));
        assert_eq!(genesis.total_supply(), Some(Amount::from_u64(1000)));
    }

    #[test]
    fn roundtrips_through_json() {
        let genesis = Genesis {
            chain_id: "botchain-devnet".to_string(),
            timestamp_ms: 1,
            allocations: vec![Allocation {
                address: address(2),
                amount: Amount::from_u64(7),
            }],
        };
        let json = serde_json::to_string(&genesis).expect("serialize");
        assert!(json.contains(r#""amount":"7""#));
        assert_eq!(Genesis::from_json(&json).expect("reparse"), genesis);
    }

    #[test]
    fn empty_allocations_are_allowed() {
        let genesis = Genesis::from_json(r#"{"chain_id":"x","timestamp_ms":0,"allocations":[]}"#)
            .expect("ok");
        assert_eq!(genesis.total_supply(), Some(Amount::ZERO));
    }

    #[test]
    fn rejects_bad_documents() {
        assert!(matches!(
            Genesis::from_json("not json"),
            Err(GenesisError::Json(_))
        ));
        // amount as a number, not a string
        let bad = format!(
            r#"{{"chain_id":"x","timestamp_ms":0,"allocations":[{{"address":"{}","amount":5}}]}}"#,
            address(3).to_base58()
        );
        assert!(matches!(
            Genesis::from_json(&bad),
            Err(GenesisError::Json(_))
        ));
        // not a base58 address
        assert!(matches!(
            Genesis::from_json(
                r#"{"chain_id":"x","timestamp_ms":0,"allocations":[{"address":"0","amount":"5"}]}"#
            ),
            Err(GenesisError::Json(_))
        ));
        assert_eq!(
            Genesis::from_json(r#"{"chain_id":"","timestamp_ms":0,"allocations":[]}"#),
            Err(GenesisError::EmptyChainId)
        );
    }

    #[test]
    fn rejects_duplicates_and_overflow() {
        let a = address(4).to_base58();
        let duplicate = format!(
            r#"{{"chain_id":"x","timestamp_ms":0,"allocations":[
                 {{"address":"{a}","amount":"1"}},{{"address":"{a}","amount":"2"}}]}}"#
        );
        assert_eq!(
            Genesis::from_json(&duplicate),
            Err(GenesisError::DuplicateAllocation(a))
        );

        let overflow = format!(
            r#"{{"chain_id":"x","timestamp_ms":0,"allocations":[
                 {{"address":"{}","amount":"{}"}},{{"address":"{}","amount":"1"}}]}}"#,
            address(5).to_base58(),
            u64::MAX,
            address(6).to_base58(),
        );
        assert_eq!(
            Genesis::from_json(&overflow),
            Err(GenesisError::SupplyOverflow)
        );
    }

    #[test]
    fn loads_from_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("genesis.json");
        std::fs::write(
            &path,
            r#"{"chain_id":"botchain-devnet","timestamp_ms":5,"allocations":[]}"#,
        )
        .expect("write");
        let genesis = Genesis::load(&path).expect("load");
        assert_eq!(genesis.timestamp_ms, 5);

        let missing = dir.path().join("nope.json");
        assert!(matches!(
            Genesis::load(&missing),
            Err(GenesisError::Read { .. })
        ));
    }
}
