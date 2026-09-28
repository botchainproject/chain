//! The transfer transaction: the only transaction botchain has.
//!
//! A transfer moves base units from one account to another. It is signed over
//! a plain UTF-8 text message, so that an ordinary Solana wallet can sign it
//! with its "sign message" function:
//!
//! ```text
//! botchain:transfer:<chain_id>:<from>:<to>:<amount>:<nonce>
//! ```
//!
//! The base58 signature string is the transaction id. There are no fees.

use serde::{Deserialize, Serialize};

use crate::amount::Amount;
use crate::crypto::{Address, CryptoError, Keypair, Signature};

/// The prefix of every signed transfer message.
pub const TRANSFER_PREFIX: &str = "botchain:transfer";

/// Why a transfer is not acceptable.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TxError {
    /// The amount is zero; a transfer must move something.
    #[error("transfer amount must be greater than zero")]
    ZeroAmount,
    /// The sender address is not a usable ed25519 public key.
    #[error("sender address is not a valid ed25519 public key: {0}")]
    BadSender(String),
    /// The signature does not match the message and the sender.
    #[error("signature does not verify for this transfer")]
    BadSignature,
}

impl From<CryptoError> for TxError {
    fn from(error: CryptoError) -> Self {
        match error {
            CryptoError::BadSignature => TxError::BadSignature,
            other => TxError::BadSender(other.to_string()),
        }
    }
}

/// Builds the exact text that a transfer signature covers.
///
/// The fields are joined with `:` in the fixed order sender, receiver,
/// amount, nonce. `chain_id` binds the signature to one chain, so a transfer
/// signed for the devnet cannot be replayed on the mainnet.
pub fn transfer_message(
    chain_id: &str,
    from: &Address,
    to: &Address,
    amount: Amount,
    nonce: u64,
) -> String {
    format!(
        "{TRANSFER_PREFIX}:{chain_id}:{from}:{to}:{amount}:{nonce}",
        from = from.to_base58(),
        to = to.to_base58(),
    )
}

/// A signed transfer of base units between two accounts.
///
/// The JSON form is exactly the one the public interface describes:
///
/// ```json
/// {"from":"<base58>","to":"<base58>","amount":"<decimal>","nonce":0,
///  "signature":"<base58>"}
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Transfer {
    /// The sender, who signs and pays.
    pub from: Address,
    /// The receiver.
    pub to: Address,
    /// How much moves, in base units.
    pub amount: Amount,
    /// The sender's account nonce before this transfer; the first is 0.
    pub nonce: u64,
    /// The ed25519 signature over [`Transfer::signing_message`], also the id.
    pub signature: Signature,
}

impl Transfer {
    /// Signs a new transfer with the sender's key pair.
    pub fn sign(
        chain_id: &str,
        sender: &Keypair,
        to: Address,
        amount: Amount,
        nonce: u64,
    ) -> Transfer {
        let from = sender.address();
        let message = transfer_message(chain_id, &from, &to, amount, nonce);
        let signature = sender.sign(message.as_bytes());
        Transfer {
            from,
            to,
            amount,
            nonce,
            signature,
        }
    }

    /// The exact text this transfer's signature has to cover.
    pub fn signing_message(&self, chain_id: &str) -> String {
        transfer_message(chain_id, &self.from, &self.to, self.amount, self.nonce)
    }

    /// The transaction id: the base58 signature string.
    pub fn id(&self) -> String {
        self.signature.to_base58()
    }

    /// Checks everything that can be checked without any chain state:
    /// a positive amount, a well formed sender key and a valid signature.
    ///
    /// Balance and nonce are checked later, against the account state.
    pub fn verify(&self, chain_id: &str) -> Result<(), TxError> {
        if self.amount.is_zero() {
            return Err(TxError::ZeroAmount);
        }
        // Parsing an address already guarantees 32 bytes; the sender must in
        // addition be a real curve point, or nothing could ever verify.
        self.from
            .verifying_key()
            .map_err(|e| TxError::BadSender(e.to_string()))?;
        let message = self.signing_message(chain_id);
        self.from
            .verify(message.as_bytes(), &self.signature)
            .map_err(TxError::from)
    }
}

/// Where a transfer stands: still waiting, or in a block.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TxStatus {
    /// In the mempool, not in a block yet.
    Pending,
    /// Included in the block at `block_height`.
    Confirmed,
}

/// A transfer as the `getTransaction` RPC returns it.
///
/// ```json
/// {"signature":"<base58>","from":"<base58>","to":"<base58>",
///  "amount":"<decimal>","nonce":0,"block_height":null,"status":"pending"}
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct TxView {
    /// The transaction id.
    pub signature: Signature,
    /// The sender.
    pub from: Address,
    /// The receiver.
    pub to: Address,
    /// How much moves, in base units.
    pub amount: Amount,
    /// The sender's nonce this transfer used.
    pub nonce: u64,
    /// The block that confirmed it, or `null` while it is pending.
    pub block_height: Option<u64>,
    /// `pending` or `confirmed`.
    pub status: TxStatus,
}

impl TxView {
    /// The view of a transfer that is still in the mempool.
    pub fn pending(transfer: &Transfer) -> TxView {
        TxView {
            signature: transfer.signature,
            from: transfer.from,
            to: transfer.to,
            amount: transfer.amount,
            nonce: transfer.nonce,
            block_height: None,
            status: TxStatus::Pending,
        }
    }

    /// The view of a transfer that landed in a block.
    pub fn confirmed(transfer: &Transfer, block_height: u64) -> TxView {
        TxView {
            block_height: Some(block_height),
            status: TxStatus::Confirmed,
            ..TxView::pending(transfer)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAIN: &str = "botchain-devnet";

    /// RFC 8032 section 7.1, test 1: a fixed key, so the tests are stable.
    fn sender() -> Keypair {
        let mut seed = [0u8; 32];
        let bytes = hex::decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
            .expect("hex");
        seed.copy_from_slice(&bytes);
        Keypair::from_secret_bytes(seed)
    }

    fn receiver() -> Keypair {
        Keypair::from_secret_bytes([7u8; 32])
    }

    #[test]
    fn message_text_is_exactly_the_spec() {
        let from = sender().address();
        let to = receiver().address();
        let text = transfer_message(CHAIN, &from, &to, Amount::from_u64(1000), 3);
        assert_eq!(
            text,
            format!(
                "botchain:transfer:botchain-devnet:{}:{}:1000:3",
                from.to_base58(),
                to.to_base58()
            )
        );
        assert_eq!(text.split(':').count(), 7);
    }

    #[test]
    fn signed_transfer_verifies() {
        let kp = sender();
        let tx = Transfer::sign(CHAIN, &kp, receiver().address(), Amount::from_u64(42), 0);
        assert_eq!(tx.verify(CHAIN), Ok(()));
        assert_eq!(tx.from, kp.address());
        assert_eq!(tx.id(), tx.signature.to_base58());
    }

    #[test]
    fn changed_amount_or_nonce_breaks_the_signature() {
        let kp = sender();
        let tx = Transfer::sign(CHAIN, &kp, receiver().address(), Amount::from_u64(42), 5);

        let mut tampered = tx;
        tampered.amount = Amount::from_u64(43);
        assert_eq!(tampered.verify(CHAIN), Err(TxError::BadSignature));

        let mut tampered = tx;
        tampered.nonce = 6;
        assert_eq!(tampered.verify(CHAIN), Err(TxError::BadSignature));

        let mut tampered = tx;
        tampered.to = Keypair::from_secret_bytes([9u8; 32]).address();
        assert_eq!(tampered.verify(CHAIN), Err(TxError::BadSignature));

        let mut tampered = tx;
        tampered.from = Keypair::from_secret_bytes([9u8; 32]).address();
        assert_eq!(tampered.verify(CHAIN), Err(TxError::BadSignature));

        // The untouched transfer still verifies.
        assert_eq!(tx.verify(CHAIN), Ok(()));
    }

    #[test]
    fn other_chain_id_does_not_verify() {
        let tx = Transfer::sign(
            CHAIN,
            &sender(),
            receiver().address(),
            Amount::from_u64(1),
            0,
        );
        assert_eq!(tx.verify("botchain-mainnet"), Err(TxError::BadSignature));
    }

    #[test]
    fn zero_amount_is_rejected_before_the_signature() {
        let tx = Transfer::sign(CHAIN, &sender(), receiver().address(), Amount::ZERO, 0);
        assert_eq!(tx.verify(CHAIN), Err(TxError::ZeroAmount));
    }

    #[test]
    fn sender_that_is_not_a_curve_point_is_rejected() {
        // Roughly half of all 32 byte strings are not ed25519 curve points;
        // find one instead of hardcoding a guess.
        let bad = (1u8..=255)
            .map(|b| Address::from_bytes([b; 32]))
            .find(|a| a.verifying_key().is_err())
            .expect("some byte pattern is not a curve point");
        let tx = Transfer {
            from: bad,
            to: receiver().address(),
            amount: Amount::from_u64(1),
            nonce: 0,
            signature: Signature::from_bytes([0u8; 64]),
        };
        assert!(matches!(tx.verify(CHAIN), Err(TxError::BadSender(_))));
    }

    #[test]
    fn json_shape_is_the_public_interface() {
        let tx = Transfer::sign(
            CHAIN,
            &sender(),
            receiver().address(),
            Amount::from_u64(1000),
            7,
        );
        let json = serde_json::to_string(&tx).expect("serialize");
        assert_eq!(
            json,
            format!(
                r#"{{"from":"{}","to":"{}","amount":"1000","nonce":7,"signature":"{}"}}"#,
                tx.from.to_base58(),
                tx.to.to_base58(),
                tx.signature.to_base58()
            )
        );
        let back: Transfer = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, tx);
        assert_eq!(back.verify(CHAIN), Ok(()));
    }

    #[test]
    fn tx_view_json_shape_is_the_public_interface() {
        let tx = Transfer::sign(
            CHAIN,
            &sender(),
            receiver().address(),
            Amount::from_u64(5),
            2,
        );
        let pending = serde_json::to_string(&TxView::pending(&tx)).expect("serialize");
        assert_eq!(
            pending,
            format!(
                r#"{{"signature":"{}","from":"{}","to":"{}","amount":"5","nonce":2,"block_height":null,"status":"pending"}}"#,
                tx.signature.to_base58(),
                tx.from.to_base58(),
                tx.to.to_base58()
            )
        );
        let confirmed = serde_json::to_string(&TxView::confirmed(&tx, 9)).expect("serialize");
        assert!(confirmed.contains(r#""block_height":9,"status":"confirmed""#));
    }

    #[test]
    fn bad_json_is_rejected() {
        // Amount as a number instead of a string.
        assert!(serde_json::from_str::<Transfer>(
            r#"{"from":"11111111111111111111111111111111","to":"11111111111111111111111111111111","amount":1,"nonce":0,"signature":"x"}"#
        )
        .is_err());
        // Missing field.
        assert!(serde_json::from_str::<Transfer>(
            r#"{"from":"11111111111111111111111111111111","to":"11111111111111111111111111111111","amount":"1","nonce":0}"#
        )
        .is_err());
    }

    /// A transfer whose signature was produced once by our own signer and
    /// then hardcoded: the canonical message and the id must never drift.
    #[test]
    fn hardcoded_transfer_json_verifies() {
        const JSON: &str = r#"{
            "from": "FVen3X669xLzsi6N2V91DoiyzHzg1uAgqiT8jZ9nS96Z",
            "to": "GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB",
            "amount": "1000",
            "nonce": 0,
            "signature": "3265Rh8cb5kuSynH4KiYgv6AVBC3HzNYPH7ynP8Q2NV1AjEjvVBMkYa49CH1kmFUcPvz9g3K8tYQiPbi6AoKdV4m"
        }"#;
        let tx: Transfer = serde_json::from_str(JSON).expect("deserialize");
        assert_eq!(tx.verify(CHAIN), Ok(()));
        assert_eq!(
            tx.signing_message(CHAIN),
            "botchain:transfer:botchain-devnet:\
             FVen3X669xLzsi6N2V91DoiyzHzg1uAgqiT8jZ9nS96Z:\
             GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB:1000:0"
        );

        // The same inputs signed again give exactly the same id.
        let again = Transfer::sign(CHAIN, &sender(), tx.to, tx.amount, tx.nonce);
        assert_eq!(again.id(), tx.id());
        assert_eq!(again.from, tx.from);

        // One flipped byte in that hardcoded signature must not verify.
        let mut broken = tx;
        let mut raw = tx.signature.to_bytes();
        raw[10] ^= 0x01;
        broken.signature = Signature::from_bytes(raw);
        assert_eq!(broken.verify(CHAIN), Err(TxError::BadSignature));
    }
}
