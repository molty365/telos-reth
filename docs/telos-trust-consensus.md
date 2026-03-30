# Telos `trust_consensus` — Mid-Chain Start for Non-Genesis EVM Chains

## Background

Telos EVM was deployed onto an already-running Antelope (EOSIO) blockchain. The EVM's "genesis" starts at:
- **Testnet**: Native block ~36M
- **Mainnet**: Native block ~180M

This means Telos EVM blocks don't start at block 0. And when syncing a new reth node, operators typically want to start from a **recent** block (e.g., block 411M), not replay the entire EVM history from the beginning.

Standard reth (like all Ethereum execution clients) assumes a contiguous chain: block 0 → 1 → 2 → ... → head. Every block has a parent, and the chain connects all the way back to genesis. **Telos breaks this assumption.**

## The `--telos.trust_consensus` Flag

When enabled, reth trusts the Telos consensus layer (nodeos + SHIP translator) for:
- Block ordering and validity
- Execution results
- State transitions
- Chain continuity

This means reth doesn't need to independently verify parent chains, execute transactions, or compute state roots — it accepts blocks as authoritative from the consensus client.

## What `trust_consensus` Bypasses

Starting a reth node mid-chain required bypassing **6+ layers** of genesis-chain assumptions in reth's engine tree, persistence, and storage systems:

### 1. State Provider (`engine/tree/mod.rs`, `payload_validator.rs`)
**Problem**: `state_provider_builder()` looks up the parent block's hash in the DB to create a state view. On a fresh start at block 411M, no parent exists.

**Fix**: When `trust_consensus` is on, always return a valid state provider:
1. Try in-memory tree state first
2. Fall back to latest persisted block
3. Fall back to genesis block (block 0, which always exists)

### 2. Parent Header Validation (`payload_validator.rs`)
**Problem**: `validate_header_against_parent()` checks that a block's `parent_hash` matches the actual parent header in the DB. Block 411M's parent hash doesn't exist in reth's DB.

**Fix**: Skip parent header validation entirely when `trust_consensus` is on. The consensus client already validated the chain.

### 3. Disconnected Block Status (`engine/tree/mod.rs`)
**Problem**: Blocks without a known parent in the tree are marked as `BlockStatus::Disconnected` and return `PayloadStatusEnum::Syncing`. The `MakeCanonical` event only fires for `Valid` blocks.

**Fix**: When `trust_consensus` is on, treat `Disconnected` blocks as `Valid`. This triggers the `MakeCanonical` event for every block from the consensus client.

### 4. Chain Walker — `on_new_head()` (`engine/tree/mod.rs`)
**Problem**: `on_new_head()` walks backwards from the new head through `blocks_by_hash` to find a connection to the canonical chain. When intermediate blocks get evicted from the in-memory tree, the walk fails and returns `None`.

**Fix**: When `trust_consensus` can't find a parent during the walk-back, commit the partial chain immediately instead of returning `None`. The consensus client guarantees block ordering.

### 5. Canonical Chain Validation During Persistence (`engine/tree/mod.rs`)
**Problem**: Before persisting blocks, reth walks the canonical chain backwards to verify it connects to the last persisted block. The parent of the first mid-chain block doesn't exist in the Headers table, causing a fatal error.

**Fix**: When `trust_consensus` encounters a missing parent during canonical chain validation, skip the validation and return `Ok(None)`.

### 6. Static File Block Number Gaps (`storage/provider/static_file/writer.rs`)
**Problem**: The static file writer stores headers, receipts, and transactions in contiguous segment files indexed by block number. When it sees block 411M but expects block 1, it either:
- Tries to fill 411M empty entries (takes forever)
- Returns `UnexpectedStaticFileBlockNumber` error

**Fix**: Two changes:
- `check_next_block_number()`: Skip validation when the expected block is ahead of the current position
- `advance_to_block()`: Skip filling the gap when it's > 1000 blocks (mid-chain start indicator)

## Architecture: In-Process SHIP Translator

The `--telos.ship_endpoint` flag enables the **embedded SHIP translator**, eliminating the need for a separate `telos-consensus-client` binary:

```
SHIP WebSocket (nodeos) → In-Process Translator → Local Engine API → reth Pipeline
```

### How it works:
1. The `telos-translator-rs` crate runs directly inside reth's process
2. It connects to nodeos via SHIP WebSocket
3. Deserializes Antelope blocks, extracts EVM transactions and state diffs
4. Sends each block to reth's Engine API via localhost HTTP (with JWT auth)
5. reth processes the block through its engine tree with `trust_consensus` bypasses

### CLI Arguments:
```bash
telos-reth node \
  --chain tevmtestnet \
  --telos.ship_endpoint ws://localhost:29999 \       # SHIP WebSocket URL
  --telos.telos_endpoint http://localhost:8888 \     # Antelope HTTP API
  --telos.chain_id 41 \                              # 40=mainnet, 41=testnet
  --telos.evm_start_block 411424575 \                # Starting EVM block number
  --telos.prev_hash '0x...' \                        # Hash of block before start_block
  --telos.trust_consensus                            # Enable all bypasses
```

## Performance

On Hetzner dedicated server (AMD EPYC, 128GB RAM, NVMe):
- **~2,000+ blocks/second** sync rate (single-block feeding)
- 469K+ blocks synced in 4 minutes
- Zero fatal errors
- Blocks visible via `eth_blockNumber` RPC immediately

## Known Limitations

1. **No historical state**: Starting mid-chain means no state data for blocks before `evm_start_block`. Historical queries for those blocks will fail.
2. **Static file gaps**: The static file segments have a gap from block 0 to `evm_start_block`. Historical block lookups in this range won't work.
3. **Error retry**: Some blocks encounter transient "no state found" errors and need retry. The feeder handles this automatically.
4. **Single-block feeding**: Currently sends blocks one at a time via Engine API. Batching would improve throughput but requires handling the state provider timing between batches.

## Files Modified

Key files with `trust_consensus` modifications (search for `trust_consensus`):
- `crates/engine/tree/src/tree/mod.rs` — Engine tree: chain walker, block status, persistence validation
- `crates/engine/tree/src/tree/payload_validator.rs` — Payload validator: state provider, header validation, execution bypass
- `crates/storage/provider/src/providers/static_file/writer.rs` — Static file: block number gaps
- `crates/stages/stages/src/stages/merkle.rs` — Merkle stage: state root bypass
- `crates/telos/ship-sync/` — In-process SHIP translator
- `crates/telos/bin/src/main.rs` — CLI integration
- `crates/telos/node/src/args.rs` — CLI arguments
