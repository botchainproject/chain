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

### crates/core milestone 3, done — accounts, genesis, state
- `account.rs`: `Account { balance: Amount, nonce: u64 }` (`EMPTY`,
  `is_empty()` = zero balance and zero nonce) and `AccountView
  { address, balance, nonce }`, the exact `getAccount` JSON.
- `genesis.rs`: `Genesis { chain_id, timestamp_ms, allocations }` with
  `Allocation { address, amount }`, `from_json`, `load(path)`, `validate()`,
  `total_supply()`. `GenesisError::{Read, Json, EmptyChainId,
  DuplicateAllocation, SupplyOverflow}`.
- `state.rs`: `State` = `BTreeMap<Address, Account>`. `from_genesis`,
  `account`/`balance`/`nonce`/`account_view`, `accounts()` (ascending
  address), `total_supply`, `credit` (minting), `validate_transfer`
  (read only) and `apply_transfer` (validate first, then debit, bump nonce,
  credit; never a partial write). `StateError::{Tx, WrongNonce,
  InsufficientBalance, BalanceOverflow, NonceOverflow}`, all arithmetic
  checked.
- State root: `sha256("botchain:state:v1" || count_be_u64 || leaves...)`,
  leaf = `sha256("botchain:account:v1" || address || balance_be ||
  nonce_be)`, leaves in ascending address order. Order independent, changes
  on any balance/nonce/account set change, empty state has a fixed non zero
  root.
- 59 core tests green, including every rejection path and root determinism.

### crates/core milestone 4, done — blocks, chain, sequencer
- `block.rs`: `Block { height, hash, parent_hash, state_root, timestamp_ms,
  tx_count, transactions: Vec<Signature> }` (exactly the RPC JSON, in that
  field order). `Block::new` fills `tx_count` and the hash;
  `Block::genesis(&Genesis, state_root)` is height 0 with parent 64 zeros.
  `block_hash` = `sha256("botchain:block:v1" || height_be || parent_hash ||
  state_root || timestamp_be || tx_count_be || each 64 byte signature)`.
  Helpers `compute_hash`, `is_consistent`, `follows(parent)`, `is_genesis`.
  `Checkpoint { height, hash, state_root }` = the `getCheckpoint` JSON.
- `chain.rs`: `Chain` = chain_id + `State` + `Vec<Block>` + signature ->
  `ConfirmedTransfer { transfer, block_height }` index. `Chain::new(genesis)`,
  `height`, `head`, `block(h)`, `block_by_hash`, `latest_blocks(limit)`
  (newest first), `transaction(sig)`, `checkpoint()`, and `seal(timestamp_ms,
  transfers) -> SealedBlock { block, included, rejected }`. Seal applies in
  order, drops invalid ones with a `RejectReason` (`AlreadyConfirmed`,
  `DuplicateInBlock`, `State(..)`) and always produces a block.
- `sequencer.rs`: `Sequencer` = `Chain` + FIFO pending queue (`VecDeque` +
  `BTreeSet` of ids). `submit(tx)` does the order free checks
  (`SubmitError::{Tx, AlreadyPending, AlreadyConfirmed, StaleNonce,
  MempoolFull}`), `tick(timestamp_ms)` drains up to the block limit and seals
  (empty block when nothing is pending). Limits: 10_000 mempool, 1_000 per
  block, overridable with `with_limits`.
- 81 core tests green, including "the chain links by parent hash" and
  "replaying the same transfers from genesis gives the same hashes and roots".

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
- Empty accounts (balance 0, nonce 0) are never stored, so an unknown address
  and a drained-and-never-sent address have the same root. A spent sender has
  nonce >= 1 and stays, which keeps replay protection.
- Genesis allocations of 0 are dropped; duplicate addresses in genesis are an
  error rather than a sum, so the file is unambiguous.
- `credit` exists for genesis style minting and tests; consensus code only
  uses `apply_transfer`.
- State root is a tagged sorted hash, not a merkle tree: no proofs needed
  yet. If proofs are ever needed, swap the root function and bump the tag.
- A block commits to the state root *after* its transfers, so the genesis
  block commits to the allocations.
- Sealing clamps the timestamp to the parent's, so time never runs backwards
  even if the clock does; blocks may share a timestamp.
- Transfers dropped at seal time are not requeued: in this order they can
  never apply, and the client can resend. Kept simple on purpose.
- `submit` only rejects a nonce *below* the account nonce; a future nonce may
  become valid once earlier transfers land.
- `Signature` now derives `Ord` so it can key the confirmed tx index.

## Next (milestone 5)
The node: a `crates/node` binary `botchain-node` with the CLI flags
(`--rpc-port`, `--data-dir`, `--genesis`, `--block-ms`), a tokio tick that
calls `Sequencer::tick`, one log line per event, and the JSON-RPC surface
over axum. Persistence with redb after (or with) that; `Chain` is in memory
today and rebuilt from genesis + blocks.
