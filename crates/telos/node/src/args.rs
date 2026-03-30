//! clap [Args](clap::Args) for telos configuration

use reth_telos_rpc::eth::telos_client::TelosClientArgs;
use crate::DEFAULT_MAX_EXECUTE_BLOCK_BATCH_SIZE;

#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
#[clap(next_help_heading = "Telos")]
/// Telos arguments
pub struct TelosArgs {
    /// TelosZero endpoint to use for API calls (send_transaction, get gas price from table)
    #[arg(long = "telos.telos_endpoint", value_name = "HTTP_URL")]
    pub telos_endpoint: Option<String>,

    /// Signer account name
    #[arg(long = "telos.signer_account")]
    pub signer_account: Option<String>,

    /// Signer permission name
    #[arg(long = "telos.signer_permission")]
    pub signer_permission: Option<String>,

    /// Signer private key
    #[arg(long = "telos.signer_key")]
    pub signer_key: Option<String>,

    /// Seconds to cache gas price
    #[arg(long = "telos.gas_cache_seconds")]
    pub gas_cache_seconds: Option<u32>,

    /// Maximum number of blocks to execute sequentially in a batch.
    ///
    /// This is used as a cutoff to prevent long-running sequential block execution when we receive
    /// a batch of downloaded blocks.
    #[arg(long = "engine.max-execute-block-batch-size", default_value_t = DEFAULT_MAX_EXECUTE_BLOCK_BATCH_SIZE)]
    pub max_execute_block_batch_size: usize,

    /// Enable Two-way storage compare between reth and telos
    #[arg(long = "telos.two_way_storage_compare", default_value = "false")]
    pub two_way_storage_compare: bool,

    /// Block delta between native and EVM
    #[arg(long = "telos.block_delta")]
    pub block_delta: Option<u32>,

    /// Trust consensus client execution results (from nodeos) instead of re-verifying.
    /// When true, skips receipt root validation, tolerates EVM execution errors,
    /// skips state root recomputation, and bypasses static file tx number checks.
    #[arg(long = "telos.trust_consensus", default_value = "true")]
    pub trust_consensus: bool,

    /// SHIP WebSocket endpoint for embedded sync (e.g. ws://localhost:29999).
    /// When set, the translator runs inside reth and feeds blocks directly.
    #[arg(long = "telos.ship_endpoint")]
    pub ship_endpoint: Option<String>,

    /// Telos chain ID (40 for mainnet, 41 for testnet)
    #[arg(long = "telos.chain_id")]
    pub chain_id: Option<u64>,

    /// EVM start block number for the translator
    #[arg(long = "telos.evm_start_block")]
    pub evm_start_block: Option<u32>,

    /// Previous block hash for translator initialization
    #[arg(long = "telos.prev_hash")]
    pub prev_hash: Option<String>,

    /// Expected hash of the start block for validation (optional)
    #[arg(long = "telos.validate_hash")]
    pub validate_hash: Option<String>,

    /// EVM deploy block (skip events before this)
    #[arg(long = "telos.evm_deploy_block")]
    pub evm_deploy_block: Option<u32>,

    /// EVM stop block (optional, stop sync at this block)
    #[arg(long = "telos.evm_stop_block")]
    pub evm_stop_block: Option<u32>,

    /// Batch size for Engine API newPayload calls (default: 50)
    #[arg(long = "telos.ship_batch_size", default_value = "50")]
    pub ship_batch_size: usize,

    /// Path to the telos-consensus-client binary.
    /// If not set, searches PATH for "telos-consensus-client".
    #[arg(long = "telos.consensus_client_binary")]
    pub consensus_client_binary: Option<String>,
}

impl From<TelosArgs> for TelosClientArgs {
    fn from(args: TelosArgs) -> Self {
        TelosClientArgs {
            telos_endpoint: args.telos_endpoint,
            signer_account: args.signer_account,
            signer_permission: args.signer_permission,
            signer_key: args.signer_key,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Args, Parser};

    /// A helper type to parse Args more easily
    #[derive(Parser)]
    struct CommandParser<T: Args> {
        #[clap(flatten)]
        args: T,
    }

    #[test]
    fn test_parse_database_args() {
        let default_args = TelosArgs::default();
        let args = CommandParser::<TelosArgs>::parse_from(["reth"]).args;
        assert_eq!(args, default_args);
    }
}
