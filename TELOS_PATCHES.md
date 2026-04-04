# Telos EVM Patches for reth v1.11.3

## Overview

Telos EVM is a high-performance EVM running on the Telos DPoS blockchain. It uses a
**consensus client** (`telos-consensus-client`) to bridge between the native Telos
blockchain (nodeos) and reth's Engine API. The consensus client translates EOSIO blocks
into Ethereum ExecutionPayloads and feeds them to reth via `newPayload` / `forkChoiceUpdated`.

## Architecture

```
nodeos (DPoS consensus) → SHIP (state history) → telos-consensus-client → Engine API → telos-reth
```

- **nodeos**: The source of truth. Runs the eosio.evm smart contract.
- **consensus client**: Reads SHIP data, translates native blocks to EVM ExecutionPayloads.
- **telos-reth**: Stores EVM state and serves Ethereum-compatible RPC.
- **trust_consensus mode**: Reth trusts the consensus client's execution results instead of re-executing, since nodeos already validated everything via DPoS.

## Patches

### Patch 1: Trust Consensus Mode (`--telos.trust_consensus`)

**Files:** Multiple (engine tree, consensus, stages, evm)

Telos's EVM is a smart contract inside nodeos — DPoS consensus already validated every
transaction. Re-executing in reth is redundant and produces different results due to
differences in gas metering, native actions (deposits/withdrawals), and state diff handling.

When `trust_consensus=true` (default):
- Skip EVM re-execution — accept state from consensus client
- Skip state root validation — Telos uses `EMPTY_ROOT_HASH`
- Skip receipt root validation — receipts come from nodeos console output
- Skip gas validation for system transactions
- Make every valid block canonical immediately (no fork choice games)
- Allow blocks from consensus client on fresh start (best_block=0)

### Patch 2: Block Hash Consistency

**File:** `crates/telos/node/src/engine.rs` (`TelosEngineValidator`)

The consensus client (alloy 0.3.x) computes block hashes with `base_fee_per_gas=None`
but sends a non-zero `base_fee_per_gas` in the ExecutionPayload. Standard reth would
recompute the hash using the payload's base_fee_per_gas and get a mismatch.

**Fix:** Trust the `block_hash` from the consensus client. Strip `base_fee_per_gas` from
the header before storing (sets to `None`) to match the legacy representation.

### Patch 3: Parent Block Number Validation

**File:** `crates/consensus/common/src/validation.rs`

The Telos EVM chain starts at block 1 (first SHIP block), not immediately after genesis
block 0. When syncing from genesis, there's a gap: block 0 → block 1. Standard Ethereum
consensus requires `child.number == parent.number + 1`, which fails here.

**Fix:** Skip parent number validation when `trust_consensus` is enabled.

### Patch 4: Timestamp Validation

**File:** `crates/ethereum/consensus/src/lib.rs`

Telos produces blocks every 0.5 seconds with integer-second timestamps. Many consecutive
blocks share the same timestamp. Ethereum requires strictly increasing timestamps.

**Fix:** Skip `validate_against_parent_timestamp` check.

## Syncing from Genesis

### Prerequisites

1. **nodeos** with SHIP (state history plugin) synced from genesis
2. **telos-consensus-client** built from `telosnetwork/telos-consensus-client`
3. **telos-reth** built from this repo

### Testnet (Chain ID 41)

```bash
# Start reth
telos-reth node --chain tevmtestnet \
  --datadir /data/reth-testnet \
  --http --http.addr 0.0.0.0 --http.port 8545 \
  --http.api eth,net,web3,debug,trace \
  --authrpc.addr 127.0.0.1 --authrpc.port 8551 \
  --authrpc.jwtsecret /path/to/jwt.hex \
  --telos.telos_endpoint http://127.0.0.1:8888 \
  --telos.signer_account rpc.evm \
  --telos.signer_permission rpc \
  --telos.signer_key <SIGNER_KEY>
```

Consensus client config (`config.toml`):
```toml
chain_id = 41
evm_start_block = 1
evm_deploy_block = 136393755
prev_hash = "0xb25034033c9ca7a40e879ddcc29cf69071a22df06688b5fe8cc2d68b4e0528f9"
batch_size = 500
ship_endpoint = "ws://127.0.0.1:18999"
chain_endpoint = "http://127.0.0.1:8888"
execution_endpoint = "http://127.0.0.1:8551"
jwt_secret = "<same as reth authrpc>"
log_level = "info"
data_path = "/data/consensus-client/db"
block_checkpoint_interval = 1000
maximum_sync_range = 500000
latest_blocks_in_db_num = 1800
```

**Key config values explained:**
- `evm_start_block = 1`: First EVM block number. SHIP starts at native block `1 + 57 = 58`.
  This produces blocks 1, 2, 3... matching the production chain.
- `evm_deploy_block = 136393755`: Native block where eosio.evm was deployed. Blocks before
  this are empty (no EVM transactions). The deploy state is injected at this block.
- `prev_hash`: Must be the genesis block hash (`0xb25034...` for testnet). This ensures
  block 1's `parentHash` matches production, producing identical block hashes.
- `block_delta`: Hardcoded per chain (57 for testnet, 36 for mainnet). Maps native blocks
  to EVM blocks: `EVM_block = native_block - block_delta`.

### Mainnet (Chain ID 40)

```toml
chain_id = 40
evm_start_block = 1
evm_deploy_block = <mainnet deploy block>
prev_hash = "<mainnet genesis hash>"
```

Use `--chain tevmmainnet` for reth. Mainnet `block_delta` = 36.

### Block Hash Determinism

**CRITICAL:** For all nodes to produce identical block hashes, they MUST use the same
`evm_start_block` and `prev_hash`. The block hash depends on:

1. `parentHash` — chains from genesis through every block
2. `timestamp` — derived from native block timestamp (deterministic per native block)
3. `extraData` — native block hash (deterministic per native block)
4. `base_fee_per_gas` — must be `None` (our `TelosEngineValidator` handles this)

If you start from a snapshot/backup instead of genesis, use the backup's finalized block
as `evm_start_block` and its hash as `prev_hash`. The resulting chain will be internally
consistent but hashes will NOT match other nodes that started from genesis.

### Sync Performance

- Empty blocks (before EVM deploy): ~3,800 blocks/sec
- Blocks with transactions: ~250-1,100 blocks/sec depending on tx volume
- Testnet full sync ETA: ~10-15 hours (136M empty + 280M active blocks)

### Continuing from Backup

The official `telos-evm-installer` downloads a reth backup and continues from there:
```toml
evm_start_block = <backup's finalized block>
prev_hash = "<backup's finalized block parent hash>"
```
This is faster but produces a different hash chain than genesis sync.

## Testing Status

- ✅ Syncs from block 1 with zero errors
- ✅ `forkChoiceUpdated` returns `Valid` status
- ✅ Blocks committed to canonical chain at ~3,800 blocks/sec
- ✅ Block hashes match TCD production (v1.0.8) exactly
- ✅ Dual reth instances run in parallel (working + genesis sync)
- ✅ RPC queries work correctly
