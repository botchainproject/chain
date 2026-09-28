//! Per address account data: a balance and a nonce.

use serde::{Deserialize, Serialize};

use crate::amount::Amount;
use crate::crypto::Address;

/// The state of one account.
///
/// An account with a zero balance and a zero nonce is indistinguishable from
/// an address that was never used, so the state does not store it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Account {
    /// Spendable base units.
    pub balance: Amount,
    /// The nonce the account's next transfer has to use; starts at 0.
    pub nonce: u64,
}

impl Account {
    /// A fresh account: no balance, nonce 0.
    pub const EMPTY: Account = Account {
        balance: Amount::ZERO,
        nonce: 0,
    };

    /// An account with a balance and a nonce.
    pub const fn new(balance: Amount, nonce: u64) -> Self {
        Account { balance, nonce }
    }

    /// True when the account carries no information and can be forgotten.
    pub const fn is_empty(&self) -> bool {
        self.balance.is_zero() && self.nonce == 0
    }

    /// The account with its address attached, the shape the RPC returns.
    pub fn view(&self, address: Address) -> AccountView {
        AccountView {
            address,
            balance: self.balance,
            nonce: self.nonce,
        }
    }
}

/// An account together with its address: the `getAccount` RPC result.
///
/// ```json
/// {"address":"<base58>","balance":"<decimal>","nonce":0}
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AccountView {
    /// The account's address.
    pub address: Address,
    /// Spendable base units, a decimal string in JSON.
    pub balance: Amount,
    /// The nonce the next transfer has to use.
    pub nonce: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Keypair;

    #[test]
    fn empty_account_is_empty() {
        assert!(Account::EMPTY.is_empty());
        assert!(Account::default().is_empty());
        assert!(!Account::new(Amount::from_u64(1), 0).is_empty());
        // A spent account keeps its nonce, so it is not forgotten.
        assert!(!Account::new(Amount::ZERO, 1).is_empty());
    }

    #[test]
    fn view_json_shape_is_the_public_interface() {
        let address = Keypair::from_secret_bytes([3u8; 32]).address();
        let view = Account::new(Amount::from_u64(1000), 7).view(address);
        let json = serde_json::to_string(&view).expect("serialize");
        assert_eq!(
            json,
            format!(
                r#"{{"address":"{}","balance":"1000","nonce":7}}"#,
                address.to_base58()
            )
        );
    }
}
