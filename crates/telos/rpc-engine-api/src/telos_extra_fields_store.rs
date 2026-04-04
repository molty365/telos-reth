//! Thread-safe store for Telos extra fields passed alongside engine_newPayloadV1.
//!
//! The Telos consensus client sends `engine_newPayloadV1(payload, telos_extra_fields)` 
//! where the extra fields contain native Telos state diffs. Since the standard reth
//! engine pipeline only passes ExecutionData through the BeaconEngineMessage channel,
//! we use this shared store as a side-channel to make the extra fields available 
//! during block execution.
//!
//! Flow:
//! 1. RPC handler receives extra_fields and stores them keyed by block_hash
//! 2. Block execution retrieves extra_fields by block_hash
//! 3. compare_state_diffs is called with the extra fields after EVM execution
//! 4. Entry is removed after consumption

use alloy_primitives::B256;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use crate::structs::TelosEngineAPIExtraFields;

/// Global store for Telos extra fields, keyed by block hash.
static TELOS_EXTRA_FIELDS: LazyLock<RwLock<HashMap<B256, TelosEngineAPIExtraFields>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Store extra fields for a given block hash.
pub fn store_extra_fields(block_hash: B256, extra_fields: TelosEngineAPIExtraFields) {
    let mut store = TELOS_EXTRA_FIELDS.write().expect("telos extra fields lock poisoned");
    store.insert(block_hash, extra_fields);
}

/// Retrieve and remove extra fields for a given block hash.
/// Returns None if no extra fields were stored for this block.
pub fn take_extra_fields(block_hash: &B256) -> Option<TelosEngineAPIExtraFields> {
    let mut store = TELOS_EXTRA_FIELDS.write().expect("telos extra fields lock poisoned");
    store.remove(block_hash)
}

/// Check if extra fields exist for a given block hash without removing them.
pub fn has_extra_fields(block_hash: &B256) -> bool {
    let store = TELOS_EXTRA_FIELDS.read().expect("telos extra fields lock poisoned");
    store.contains_key(block_hash)
}

/// Get the number of pending extra field entries (for diagnostics).
pub fn pending_count() -> usize {
    let store = TELOS_EXTRA_FIELDS.read().expect("telos extra fields lock poisoned");
    store.len()
}
