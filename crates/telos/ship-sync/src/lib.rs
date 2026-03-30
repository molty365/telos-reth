//! Telos SHIP Sync — Embedded block translator for telos-reth.
//!
//! Runs the telos-translator-rs SHIP reader directly inside the reth process,
//! translating Antelope blocks to EVM blocks and feeding them to the Engine API.
//! No separate consensus client binary needed.

mod engine_feeder;

pub use engine_feeder::{ShipSyncConfig, spawn_ship_sync};
