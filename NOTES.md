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

## Decisions
- Verification uses `verify_strict` (rejects small order / malleable keys).
- `Address::from_bytes` does not check the curve point; the check happens in
  `verifying_key()` / `verify()`, so parsing an address never fails on-curve.
- Base58 length is validated after decoding, so any 32/64 byte payload works
  regardless of leading-zero characters.
- Amounts accept leading zeros (`"007"` = 7) but nothing else non-digit.

## Next (milestone 2)
Transactions, accounts and state: the transfer type with the exact signed
message `botchain:transfer:<chain_id>:<from>:<to>:<amount>:<nonce>`, account
state with nonces, and apply/validate rules. Then blocks, storage, the node
binary with the JSON-RPC surface from the spec.
