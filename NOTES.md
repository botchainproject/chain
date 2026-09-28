# botchain notes

Memory between sessions. Keep short and current.

## What exists

Cargo workspace, members = `crates/*`. Only the crates in
`[workspace.dependencies]` are available (offline registry). `crates/warm` is
not part of the chain: never touch it.

### crates/core (`botchain-core`) — milestone 1, done
- `crypto.rs`: `Address` (32 byte ed25519 public key, base58 like Solana),
  `Signature` (64 bytes, base58, this string is the tx id), `Keypair`
  (generate, `from_secret_bytes`/`secret_bytes` seed, `sign`), free `verify`.
  `CryptoError` covers empty / bad base58 / wrong length / bad key / bad
  signature with clear messages. Serde for `Address` and `Signature` is the
  base58 string. `Debug` for `Keypair` prints only the address.
- `hash.rs`: `Hash` (32 bytes, Display/serde = 64 lowercase hex, `Hash::ZERO`
  is the genesis parent), `sha256`, `sha256_parts`.
- `amount.rs`: `Amount(u64)`, serde is always a decimal string, checked
  add/sub, `parse_decimal` rejects signs, dots, spaces and overflow.
- 24 unit tests, all green (RFC 8032 test 1 vector for keys and signature,
  flipped signature byte and flipped message byte both fail, sha256 vectors,
  JSON shapes).

### crates/core `tx.rs` (`botchain-core`) — milestone 2, done
- `Transfer { from, to, amount, nonce, signature }`, serde gives exactly the
  interface JSON (`amount` decimal string, `nonce` integer, rest base58).
- `transfer_message(chain_id, from, to, amount, nonce)` builds the signed
  text `botchain:transfer:<chain_id>:<from>:<to>:<amount>:<nonce>`;
  `Transfer::signing_message`, `Transfer::sign(chain_id, keypair, ...)`,
  `Transfer::id()` = base58 signature string.
- `Transfer::verify(chain_id)` -> `TxError::{ZeroAmount, BadSender,
  BadSignature}`: positive amount, sender is a real curve point, signature
  over the canonical message. Balance and nonce are state checks, later.
- 9 tests: signed transfer verifies, tampered amount/nonce/to/from and a
  wrong chain_id all fail, zero amount rejected, JSON shape pinned, and a
  hardcoded transfer JSON (signature produced once by our signer) verifies.

## Decisions
- Verification uses `verify_strict` (rejects small order / malleable keys).
- `Address::from_bytes` does not check the curve point; the check happens in
  `verifying_key()` / `verify()`, so parsing an address never fails on-curve.
- Base58 length is validated after decoding, so any 32/64 byte payload works
  regardless of leading-zero characters.
- Amounts accept leading zeros (`"007"` = 7) but nothing else non-digit.
- Transfer JSON is lenient about unknown fields (no `deny_unknown_fields`) so
  clients can add metadata; only the five fields are signed.
- `to` is not required to be a curve point (Solana allows off-curve PDAs);
  `from` must be, or nothing could verify.
- Self transfers are not rejected here; they are a no-op for state.

## Next (milestone 3)
Accounts and state: balances plus nonces, apply/validate rules on top of
`Transfer::verify` (sufficient balance, nonce == account nonce), a state root.
Then blocks, storage, the node binary with the JSON-RPC surface from the spec.
