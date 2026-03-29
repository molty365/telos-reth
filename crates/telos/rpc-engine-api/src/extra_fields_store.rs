//! Global store for Telos extra fields.
//!
//! This is a temporary bridge to pass TelosEngineAPIExtraFields from the RPC handler
//! to the block executor without modifying the entire execution pipeline.

use crate::structs::TelosEngineAPIExtraFields;
use alloy_primitives::B256;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// Global store mapping block hash → extra fields
static EXTRA_FIELDS: LazyLock<Mutex<HashMap<B256, TelosEngineAPIExtraFields>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Store extra fields for a block hash.
pub fn store_extra_fields(block_hash: B256, fields: TelosEngineAPIExtraFields) {
    if let Ok(mut map) = EXTRA_FIELDS.lock() {
        // Keep only last 1000 entries to prevent unbounded growth
        if map.len() > 1000 {
            let keys: Vec<B256> = map.keys().take(500).cloned().collect();
            for key in keys {
                map.remove(&key);
            }
        }
        map.insert(block_hash, fields);
    }
}

/// Retrieve and remove extra fields for a block hash.
pub fn take_extra_fields(block_hash: &B256) -> Option<TelosEngineAPIExtraFields> {
    EXTRA_FIELDS.lock().ok()?.remove(block_hash)
}

/// Retrieve extra fields without removing (for retry scenarios).
pub fn get_extra_fields(block_hash: &B256) -> Option<TelosEngineAPIExtraFields> {
    EXTRA_FIELDS.lock().ok()?.get(block_hash).cloned()
}
