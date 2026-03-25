# Telos EVM Patches for reth v1.11.3

## Overview

Telos EVM is a high-performance EVM running on the Telos DPoS blockchain. It uses a
**consensus client** (`telos-consensus-client`) to bridge between the native Telos
blockchain (nodeos) and reth's Engine API. The consensus client translates EOSIO blocks
into Ethereum ExecutionPayloads and feeds them to reth via `newPayload` / `forkChoiceUpdated`.

This requires three patches to upstream reth because Telos EVM has fundamental
architectural differences from Ethereum:

## Patch 1: State Root Validation Bypass

**File:** `crates/engine/tree/src/tree/payload_validator.rs`

**Problem:** Telos EVM does not maintain Ethereum-style state tries. The consensus client
sends blocks with an empty state root (`0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421`).
When reth executes the block, it computes a different state root from its own state trie.
Standard reth rejects the block as invalid.

**Fix:** Log the mismatch but accept the block. The empty state root is intentional and
matches what the legacy telos-reth binary stores.

## Patch 2: Block Hash Consistency

**File:** `crates/ethereum/payload/src/validator.rs`

**Problem:** The standard `ensure_well_formed_payload` function calls `seal_slow()` which
recomputes the block hash from the header. Since the header contains the empty state root
(not reth's computed root), this hash differs from what `forkChoiceUpdated` will reference.
If we used `seal_slow()`, reth would store blocks under a hash that the consensus client
doesn't know about, causing permanent `SYNCING` state.

**Fix:** Use `SealedBlock::new_unchecked()` with the consensus client's provided hash
instead of recomputing via `seal_slow()`. This preserves hash consistency with:
- The consensus client's `forkChoiceUpdated` calls
- The legacy telos-reth binary (which also uses empty state roots)
- Block explorers (Blockscout/Teloscan) querying the node

## Patch 3: Timestamp Validation

**File:** `crates/ethereum/consensus/src/lib.rs`

**Problem:** Telos produces blocks every 0.5 seconds, but block timestamps use integer
seconds. This means many consecutive blocks share the same timestamp. Ethereum's consensus
rules require strictly increasing timestamps (`child.timestamp > parent.timestamp`), which
fails for Telos blocks.

**Fix:** Skip the `validate_against_parent_timestamp` check.

## Sync Process

With these patches, reth v1.11.3 can sync Telos testnet (chain ID 41) from genesis:

```bash
# Start reth
telos-reth node --chain tevmtestnet --datadir /data/reth-testnet \
  --http --http.addr 0.0.0.0 --http.port 8557 \
  --http.api eth,net,web3,debug,trace \
  --authrpc.addr 127.0.0.1 --authrpc.port 8559 \
  --authrpc.jwtsecret /path/to/jwt.hex

# Start consensus client (config: evm_start_block = 1, evm_deploy_block = 137430500)
telos-consensus-client --config /path/to/config.toml
```

Blocks before `evm_deploy_block` (137430500) are empty EVM blocks (no transactions).
Real EVM activity starts at the deploy block.

## Hash Consistency

These patches ensure that block hashes returned by reth v1.11.3 match those returned
by the legacy telos-reth binary. Both store blocks with the empty state root in the
header, producing identical block hashes for the same block numbers. This is critical
for:

- Load-balanced RPC endpoints (`rpc.telos.net`)
- Block explorers consistency
- Cross-chain protocols (LayerZero) that verify block hashes
- Wallet transaction confirmation by block hash

## Testing Status

- ✅ Syncs from block 1 with zero errors
- ✅ `forkChoiceUpdated` returns `Valid` status
- ✅ Blocks committed to canonical chain
- ✅ RPC `eth_blockNumber` advances correctly
- ✅ Block hashes match legacy telos-reth output
