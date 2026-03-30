#![allow(missing_docs)]

//! Embedded SHIP sync for Telos reth.
//!
//! Manages the telos-consensus-client as a subprocess that feeds translated
//! blocks to reth's Engine API via local authenticated HTTP calls.
//!
//! ## Architecture
//!
//! The telos-consensus-client (translator + client) runs as a managed child
//! process. It connects to a SHIP WebSocket endpoint, translates Antelope
//! blocks into EVM blocks, and sends them to reth via `engine_newPayloadV1`
//! and `engine_forkchoiceUpdatedV1` over the local auth'd Engine API.
//!
//! ## Why subprocess instead of in-process?
//!
//! The telos-translator-rs crate depends on alloy 0.3.x, while reth uses
//! alloy 1.x. Both bring in `c-kzg` which links to the same native library,
//! causing an irreconcilable linker conflict. Once the translator is updated
//! to alloy 1.x, this crate can be refactored to embed the translator
//! directly in-process.

mod managed_consensus_client;

pub use managed_consensus_client::{spawn_ship_sync, ShipSyncConfig};
