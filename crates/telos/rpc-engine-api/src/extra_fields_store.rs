//! File-based store for Telos extra fields.
//!
//! The consensus client writes extra fields to /tmp/telos-extra-fields/<block_hash>.json
//! before sending engine_newPayloadV1. The executor reads and removes the file.

use crate::structs::TelosEngineAPIExtraFields;
use alloy_primitives::B256;
use std::path::PathBuf;

fn store_dir() -> PathBuf {
    PathBuf::from("/tmp/telos-extra-fields")
}

fn file_path(block_hash: &B256) -> PathBuf {
    store_dir().join(format!("{:?}.json", block_hash))
}

/// Store extra fields for a block hash.
pub fn store_extra_fields(block_hash: B256, fields: TelosEngineAPIExtraFields) {
    let dir = store_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = file_path(&block_hash);
    if let Ok(json) = serde_json::to_string(&fields) {
        let _ = std::fs::write(&path, json);
    }
}

/// Retrieve and remove extra fields for a block hash.
pub fn take_extra_fields(block_hash: &B256) -> Option<TelosEngineAPIExtraFields> {
    let path = file_path(block_hash);
    let data = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    serde_json::from_str(&data).ok()
}
