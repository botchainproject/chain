//! The account state: a balance and a nonce per address, plus the rules that
//! move units between them and the state root that commits to all of it.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::account::{Account, AccountView};
use crate::amount::Amount;
use crate::crypto::Address;
use crate::genesis::{Genesis, GenesisError};
use crate::hash::Hash;
use crate::tx::{Transfer, TxError};

/// Domain tag of an account leaf in the state root.
const ACCOUNT_TAG: &[u8] = b"botchain:account:v1";
/// Domain tag of the state root itself.
const STATE_TAG: &[u8] = b"botchain:state:v1";

/// Why a transfer cannot be applied to the state.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StateError {
    /// The transfer itself is not valid (amount, sender key, signature).
    #[error(transparent)]
    Tx(#[from] TxError),
    /// The nonce is not the sender's next nonce: replay or gap.
    #[error("wrong nonce: account {address} expects {expected}, transfer has {got}")]
    WrongNonce {
        /// The sender.
        address: String,
        /// The nonce the account expects next.
        expected: u64,
        /// The nonce the transfer carries.
        got: u64,
    },
    /// The sender cannot cover the amount.
    #[error("insufficient balance: account {address} has {balance}, transfer needs {amount}")]
    InsufficientBalance {
        /// The sender.
        address: String,
        /// What the sender holds.
        balance: Amount,
        /// What the transfer wants to move.
        amount: Amount,
    },
    /// Crediting the receiver would exceed u64.
    #[error("balance of {address} would overflow")]
    BalanceOverflow {
        /// The account that would overflow.
        address: String,
    },
    /// The sender's nonce is at u64::MAX and cannot be bumped.
    #[error("nonce of {address} would overflow")]
    NonceOverflow {
        /// The account that would overflow.
        address: String,
    },
}

/// The whole account state, keyed by address.
///
/// Accounts are held in a `BTreeMap`, so iteration is always in ascending
/// address order and the state root does not depend on insertion order.
/// Accounts that hold nothing and have never sent anything are not stored.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct State {
    accounts: BTreeMap<Address, Account>,
}

impl State {
    /// An empty state: no accounts at all.
    pub fn new() -> Self {
        State::default()
    }

    /// Builds the state of block 0 from a genesis document.
    pub fn from_genesis(genesis: &Genesis) -> Result<Self, GenesisError> {
        genesis.validate()?;
        let mut state = State::new();
        for allocation in &genesis.allocations {
            if allocation.amount.is_zero() {
                // Nothing to store; an empty account is the same as no account.
                continue;
            }
            // validate() already ruled out duplicates and a supply overflow.
            state
                .accounts
                .insert(allocation.address, Account::new(allocation.amount, 0));
        }
        Ok(state)
    }

    /// The account at `address`, empty when the address is unknown.
    pub fn account(&self, address: &Address) -> Account {
        self.accounts
            .get(address)
            .copied()
            .unwrap_or(Account::EMPTY)
    }

    /// The balance at `address`, zero when the address is unknown.
    pub fn balance(&self, address: &Address) -> Amount {
        self.account(address).balance
    }

    /// The nonce the next transfer from `address` has to use.
    pub fn nonce(&self, address: &Address) -> u64 {
        self.account(address).nonce
    }

    /// The account with its address attached, the shape the RPC returns.
    pub fn account_view(&self, address: &Address) -> AccountView {
        self.account(address).view(*address)
    }

    /// How many accounts the state stores.
    pub fn len(&self) -> usize {
        self.accounts.len()
    }

    /// True when no account is stored.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }

    /// All stored accounts in ascending address order.
    pub fn accounts(&self) -> impl Iterator<Item = (&Address, &Account)> {
        self.accounts.iter()
    }

    /// The sum of every balance, `None` if it somehow overflows.
    pub fn total_supply(&self) -> Option<Amount> {
        self.accounts
            .values()
            .try_fold(Amount::ZERO, |sum, account| {
                sum.checked_add(account.balance)
            })
    }

    /// Checks a transfer against this state without changing anything.
    ///
    /// Runs the stateless checks of [`Transfer::verify`] first, then the nonce
    /// and the balance. Every arithmetic step is checked.
    pub fn validate_transfer(&self, chain_id: &str, tx: &Transfer) -> Result<(), StateError> {
        tx.verify(chain_id)?;
        let sender = self.account(&tx.from);
        if tx.nonce != sender.nonce {
            return Err(StateError::WrongNonce {
                address: tx.from.to_base58(),
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        let remaining = sender.balance.checked_sub(tx.amount).ok_or_else(|| {
            StateError::InsufficientBalance {
                address: tx.from.to_base58(),
                balance: sender.balance,
                amount: tx.amount,
            }
        })?;
        if sender.nonce == u64::MAX {
            return Err(StateError::NonceOverflow {
                address: tx.from.to_base58(),
            });
        }
        // A self transfer credits the balance the debit just left behind.
        let credited = if tx.from == tx.to {
            remaining
        } else {
            self.balance(&tx.to)
        };
        credited
            .checked_add(tx.amount)
            .ok_or_else(|| StateError::BalanceOverflow {
                address: tx.to.to_base58(),
            })?;
        Ok(())
    }

    /// Applies a transfer: debit, credit and bump the sender's nonce.
    ///
    /// Either the whole transfer applies or the state is untouched: every
    /// check runs before the first write.
    pub fn apply_transfer(&mut self, chain_id: &str, tx: &Transfer) -> Result<(), StateError> {
        self.validate_transfer(chain_id, tx)?;

        let mut sender = self.account(&tx.from);
        // Checked above, but stay panic free on any future path.
        sender.balance = sender.balance.checked_sub(tx.amount).ok_or_else(|| {
            StateError::InsufficientBalance {
                address: tx.from.to_base58(),
                balance: sender.balance,
                amount: tx.amount,
            }
        })?;
        sender.nonce = sender
            .nonce
            .checked_add(1)
            .ok_or_else(|| StateError::NonceOverflow {
                address: tx.from.to_base58(),
            })?;
        self.write(tx.from, sender);

        let mut receiver = self.account(&tx.to);
        receiver.balance =
            receiver
                .balance
                .checked_add(tx.amount)
                .ok_or_else(|| StateError::BalanceOverflow {
                    address: tx.to.to_base58(),
                })?;
        self.write(tx.to, receiver);
        Ok(())
    }

    /// Adds units to an account, for genesis style minting.
    pub fn credit(&mut self, address: Address, amount: Amount) -> Result<(), StateError> {
        let mut account = self.account(&address);
        account.balance =
            account
                .balance
                .checked_add(amount)
                .ok_or_else(|| StateError::BalanceOverflow {
                    address: address.to_base58(),
                })?;
        self.write(address, account);
        Ok(())
    }

    /// Stores an account, forgetting it when it holds nothing.
    fn write(&mut self, address: Address, account: Account) {
        if account.is_empty() {
            self.accounts.remove(&address);
        } else {
            self.accounts.insert(address, account);
        }
    }

    /// The state root: sha256 over every account, in address order.
    ///
    /// Each account contributes a tagged leaf hash over its address, balance
    /// and nonce in big endian; the root hashes the leaf count and then the
    /// leaves. Any change to any account changes the root, and the root never
    /// depends on the order in which accounts were touched.
    pub fn state_root(&self) -> Hash {
        let mut hasher = Sha256::new();
        hasher.update(STATE_TAG);
        hasher.update((self.accounts.len() as u64).to_be_bytes());
        for (address, account) in &self.accounts {
            hasher.update(account_leaf(address, account).as_bytes());
        }
        let digest = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Hash::from_bytes(out)
    }
}

/// The leaf hash of one account inside the state root.
fn account_leaf(address: &Address, account: &Account) -> Hash {
    crate::hash::sha256_parts(&[
        ACCOUNT_TAG,
        address.as_bytes(),
        &account.balance.as_u64().to_be_bytes(),
        &account.nonce.to_be_bytes(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{Keypair, Signature};
    use crate::genesis::Allocation;

    const CHAIN: &str = "botchain-devnet";

    fn keypair(seed: u8) -> Keypair {
        Keypair::from_secret_bytes([seed; 32])
    }

    fn genesis(allocations: Vec<(Address, u64)>) -> Genesis {
        Genesis {
            chain_id: CHAIN.to_string(),
            timestamp_ms: 1_790_000_000_000,
            allocations: allocations
                .into_iter()
                .map(|(address, amount)| Allocation {
                    address,
                    amount: Amount::from_u64(amount),
                })
                .collect(),
        }
    }

    fn funded_state(alice: &Keypair, amount: u64) -> State {
        State::from_genesis(&genesis(vec![(alice.address(), amount)])).expect("genesis")
    }

    #[test]
    fn genesis_allocations_become_balances() {
        let alice = keypair(1);
        let bob = keypair(2);
        let state =
            State::from_genesis(&genesis(vec![(alice.address(), 1000), (bob.address(), 5)]))
                .expect("genesis");
        assert_eq!(state.balance(&alice.address()), Amount::from_u64(1000));
        assert_eq!(state.nonce(&alice.address()), 0);
        assert_eq!(state.balance(&bob.address()), Amount::from_u64(5));
        assert_eq!(state.len(), 2);
        assert_eq!(state.total_supply(), Some(Amount::from_u64(1005)));
    }

    #[test]
    fn unknown_address_is_zero() {
        let state = State::new();
        let stranger = keypair(9).address();
        assert_eq!(state.balance(&stranger), Amount::ZERO);
        assert_eq!(state.nonce(&stranger), 0);
        assert_eq!(state.account(&stranger), Account::EMPTY);
        assert!(state.is_empty());
    }

    #[test]
    fn zero_allocations_are_not_stored() {
        let state =
            State::from_genesis(&genesis(vec![(keypair(1).address(), 0)])).expect("genesis");
        assert!(state.is_empty());
        assert_eq!(state.state_root(), State::new().state_root());
    }

    #[test]
    fn duplicate_genesis_allocation_is_rejected() {
        let a = keypair(1).address();
        let bad = genesis(vec![(a, 1), (a, 2)]);
        assert_eq!(
            State::from_genesis(&bad),
            Err(GenesisError::DuplicateAllocation(a.to_base58()))
        );
    }

    #[test]
    fn transfer_moves_units_and_bumps_the_nonce() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut state = funded_state(&alice, 1000);

        let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(400), 0);
        assert_eq!(state.apply_transfer(CHAIN, &tx), Ok(()));
        assert_eq!(state.balance(&alice.address()), Amount::from_u64(600));
        assert_eq!(state.nonce(&alice.address()), 1);
        assert_eq!(state.balance(&bob.address()), Amount::from_u64(400));
        assert_eq!(state.nonce(&bob.address()), 0);
        assert_eq!(state.total_supply(), Some(Amount::from_u64(1000)));

        let tx2 = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(600), 1);
        assert_eq!(state.apply_transfer(CHAIN, &tx2), Ok(()));
        // Alice is spent out but keeps her nonce, so she stays in the state.
        assert_eq!(state.balance(&alice.address()), Amount::ZERO);
        assert_eq!(state.nonce(&alice.address()), 2);
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn replayed_or_skipped_nonce_is_rejected() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut state = funded_state(&alice, 100);
        let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(10), 0);
        assert_eq!(state.apply_transfer(CHAIN, &tx), Ok(()));

        // The exact same transfer again: nonce 0 is used up.
        assert_eq!(
            state.apply_transfer(CHAIN, &tx),
            Err(StateError::WrongNonce {
                address: alice.address().to_base58(),
                expected: 1,
                got: 0,
            })
        );
        // A gap in the future is rejected too.
        let ahead = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(10), 7);
        assert_eq!(
            state.apply_transfer(CHAIN, &ahead),
            Err(StateError::WrongNonce {
                address: alice.address().to_base58(),
                expected: 1,
                got: 7,
            })
        );
        // Nothing moved.
        assert_eq!(state.balance(&alice.address()), Amount::from_u64(90));
        assert_eq!(state.balance(&bob.address()), Amount::from_u64(10));
    }

    #[test]
    fn insufficient_balance_is_rejected_and_changes_nothing() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut state = funded_state(&alice, 100);
        let root_before = state.state_root();

        let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(101), 0);
        assert_eq!(
            state.apply_transfer(CHAIN, &tx),
            Err(StateError::InsufficientBalance {
                address: alice.address().to_base58(),
                balance: Amount::from_u64(100),
                amount: Amount::from_u64(101),
            })
        );
        assert_eq!(state.state_root(), root_before);
        assert_eq!(state.nonce(&alice.address()), 0);
    }

    #[test]
    fn transfer_from_an_unknown_account_is_rejected() {
        let stranger = keypair(8);
        let bob = keypair(2);
        let mut state = State::new();
        let tx = Transfer::sign(CHAIN, &stranger, bob.address(), Amount::from_u64(1), 0);
        assert_eq!(
            state.apply_transfer(CHAIN, &tx),
            Err(StateError::InsufficientBalance {
                address: stranger.address().to_base58(),
                balance: Amount::ZERO,
                amount: Amount::from_u64(1),
            })
        );
        assert!(state.is_empty());
    }

    #[test]
    fn bad_signature_zero_amount_and_wrong_chain_are_rejected() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut state = funded_state(&alice, 100);

        let mut tampered = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(10), 0);
        tampered.amount = Amount::from_u64(11);
        assert_eq!(
            state.apply_transfer(CHAIN, &tampered),
            Err(StateError::Tx(TxError::BadSignature))
        );

        let zero = Transfer::sign(CHAIN, &alice, bob.address(), Amount::ZERO, 0);
        assert_eq!(
            state.apply_transfer(CHAIN, &zero),
            Err(StateError::Tx(TxError::ZeroAmount))
        );

        let other_chain = Transfer::sign(
            "botchain-mainnet",
            &alice,
            bob.address(),
            Amount::from_u64(10),
            0,
        );
        assert_eq!(
            state.apply_transfer(CHAIN, &other_chain),
            Err(StateError::Tx(TxError::BadSignature))
        );

        let off_curve = (1u8..=255)
            .map(|b| Address::from_bytes([b; 32]))
            .find(|a| a.verifying_key().is_err())
            .expect("some byte pattern is off curve");
        let bad_sender = Transfer {
            from: off_curve,
            to: bob.address(),
            amount: Amount::from_u64(1),
            nonce: 0,
            signature: Signature::from_bytes([0u8; 64]),
        };
        assert!(matches!(
            state.apply_transfer(CHAIN, &bad_sender),
            Err(StateError::Tx(TxError::BadSender(_)))
        ));

        assert_eq!(state.balance(&alice.address()), Amount::from_u64(100));
        assert_eq!(state.nonce(&alice.address()), 0);
    }

    #[test]
    fn receiver_overflow_is_rejected() {
        let alice = keypair(1);
        let bob = keypair(2);
        // Total supply beyond u64 cannot come from genesis, so mint it here.
        let mut state = State::new();
        state
            .credit(alice.address(), Amount::from_u64(10))
            .expect("credit");
        state
            .credit(bob.address(), Amount::from_u64(u64::MAX - 5))
            .expect("credit");

        let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(10), 0);
        assert_eq!(
            state.apply_transfer(CHAIN, &tx),
            Err(StateError::BalanceOverflow {
                address: bob.address().to_base58(),
            })
        );
        // Six units still fit.
        let small = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(5), 0);
        assert_eq!(state.apply_transfer(CHAIN, &small), Ok(()));
        assert_eq!(state.balance(&bob.address()), Amount::MAX);
        assert_eq!(state.balance(&alice.address()), Amount::from_u64(5));
    }

    #[test]
    fn nonce_overflow_is_rejected() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut state = State::new();
        state
            .credit(alice.address(), Amount::from_u64(10))
            .expect("credit");
        // Force the sender to the last possible nonce.
        let mut account = state.account(&alice.address());
        account.nonce = u64::MAX;
        state.write(alice.address(), account);

        let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(1), u64::MAX);
        assert_eq!(
            state.apply_transfer(CHAIN, &tx),
            Err(StateError::NonceOverflow {
                address: alice.address().to_base58(),
            })
        );
    }

    #[test]
    fn self_transfer_only_bumps_the_nonce() {
        let alice = keypair(1);
        let mut state = funded_state(&alice, 100);
        let tx = Transfer::sign(CHAIN, &alice, alice.address(), Amount::from_u64(40), 0);
        assert_eq!(state.apply_transfer(CHAIN, &tx), Ok(()));
        assert_eq!(state.balance(&alice.address()), Amount::from_u64(100));
        assert_eq!(state.nonce(&alice.address()), 1);

        // Sending more than the balance to yourself is still rejected.
        let too_much = Transfer::sign(CHAIN, &alice, alice.address(), Amount::from_u64(101), 1);
        assert!(matches!(
            state.apply_transfer(CHAIN, &too_much),
            Err(StateError::InsufficientBalance { .. })
        ));
    }

    #[test]
    fn credit_to_yourself_near_the_max_overflows() {
        let alice = keypair(1);
        let mut state = State::new();
        state.credit(alice.address(), Amount::MAX).expect("credit");
        assert_eq!(
            state.credit(alice.address(), Amount::from_u64(1)),
            Err(StateError::BalanceOverflow {
                address: alice.address().to_base58(),
            })
        );
    }

    #[test]
    fn validate_does_not_mutate() {
        let alice = keypair(1);
        let bob = keypair(2);
        let state = funded_state(&alice, 100);
        let root = state.state_root();
        let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(1), 0);
        assert_eq!(state.validate_transfer(CHAIN, &tx), Ok(()));
        assert_eq!(state.state_root(), root);
        assert_eq!(state.balance(&bob.address()), Amount::ZERO);
    }

    #[test]
    fn state_root_is_order_independent_and_deterministic() {
        let a = keypair(1).address();
        let b = keypair(2).address();
        let c = keypair(3).address();

        let one = State::from_genesis(&genesis(vec![(a, 1), (b, 2), (c, 3)])).expect("genesis");
        let other = State::from_genesis(&genesis(vec![(c, 3), (a, 1), (b, 2)])).expect("genesis");
        assert_eq!(one.state_root(), other.state_root());
        // Stable across calls, and 64 hex characters.
        assert_eq!(one.state_root(), one.state_root());
        assert_eq!(one.state_root().to_hex().len(), 64);

        // A different set of balances gives a different root.
        let changed = State::from_genesis(&genesis(vec![(a, 1), (b, 2), (c, 4)])).expect("genesis");
        assert_ne!(one.state_root(), changed.state_root());
    }

    #[test]
    fn state_root_changes_with_every_account_change() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut state = funded_state(&alice, 100);
        let mut roots = vec![state.state_root()];

        let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(30), 0);
        state.apply_transfer(CHAIN, &tx).expect("apply");
        roots.push(state.state_root());

        // Only a nonce change, the balances stay the same.
        let mut nonce_only = state.clone();
        let mut account = nonce_only.account(&bob.address());
        account.nonce += 1;
        nonce_only.write(bob.address(), account);
        roots.push(nonce_only.state_root());

        // Only a balance change.
        let mut balance_only = state.clone();
        balance_only
            .credit(bob.address(), Amount::from_u64(1))
            .expect("credit");
        roots.push(balance_only.state_root());

        // One more account.
        let mut extra = state.clone();
        extra
            .credit(keypair(4).address(), Amount::from_u64(1))
            .expect("credit");
        roots.push(extra.state_root());

        for (i, left) in roots.iter().enumerate() {
            for right in roots.iter().skip(i + 1) {
                assert_ne!(left, right, "roots must all differ");
            }
        }
    }

    #[test]
    fn same_history_gives_the_same_root() {
        let alice = keypair(1);
        let bob = keypair(2);
        let carol = keypair(3);

        let build = || {
            let mut state = funded_state(&alice, 1000);
            let t1 = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(100), 0);
            let t2 = Transfer::sign(CHAIN, &bob, carol.address(), Amount::from_u64(40), 0);
            let t3 = Transfer::sign(CHAIN, &alice, carol.address(), Amount::from_u64(10), 1);
            for tx in [t1, t2, t3] {
                state.apply_transfer(CHAIN, &tx).expect("apply");
            }
            state
        };
        let first = build();
        let second = build();
        assert_eq!(first, second);
        assert_eq!(first.state_root(), second.state_root());
        assert_eq!(first.balance(&carol.address()), Amount::from_u64(50));
        assert_eq!(first.total_supply(), Some(Amount::from_u64(1000)));

        // An empty state has its own fixed, non zero root.
        let empty = State::new().state_root();
        assert_ne!(empty, Hash::ZERO);
        assert_ne!(empty, first.state_root());
        assert_eq!(empty, State::new().state_root());
    }

    #[test]
    fn account_leaves_are_domain_separated() {
        // A leaf is not simply the hash of the concatenated fields, so a
        // balance cannot be confused with an address prefix.
        let address = keypair(1).address();
        let account = Account::new(Amount::from_u64(7), 1);
        let leaf = account_leaf(&address, &account);
        let naive = crate::hash::sha256_parts(&[
            address.as_bytes(),
            &7u64.to_be_bytes(),
            &1u64.to_be_bytes(),
        ]);
        assert_ne!(leaf, naive);
    }
}
