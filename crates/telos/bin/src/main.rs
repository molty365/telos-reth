#![allow(missing_docs)]

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

use alloy_eips::BlockId;
use clap::Parser;
use tracing::{error, info, warn};
use reth::chainspec::EthereumChainSpecParser;
use reth_engine_tree::tree::TreeConfig;
use reth_node_builder::EngineNodeLauncher;
use reth::cli::Cli;
use reth_node_telos::{TelosArgs, TelosNode};
use reth_telos_rpc::TelosClient;
use reth::rpc::types::BlockNumberOrTag;
use reth_provider::{DBProvider, DatabaseProviderFactory, StateProviderFactory};
use reth_db::{PlainAccountState, PlainStorageState};



fn main() {
    use reth_provider::BlockNumReader;

    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }

    if let Err(err) = Cli::<EthereumChainSpecParser, TelosArgs>::parse().run(|builder, telos_args| async move {
        // Set the global trust_consensus flag from CLI args
        reth_telos_primitives_traits::set_trust_consensus(telos_args.trust_consensus);
        if telos_args.trust_consensus {
            info!("Telos: trust_consensus enabled - trusting nodeos consensus for execution results");
        }

        let two_way_storage_compare = telos_args.two_way_storage_compare.clone();
        let telos_rpc = telos_args.telos_endpoint.clone();
        let block_delta = telos_args.block_delta.clone();

        // Capture SHIP sync args before telos_args is moved
        let ship_endpoint = telos_args.ship_endpoint.clone();
        let ship_chain_id = telos_args.chain_id;
        let ship_evm_start_block = telos_args.evm_start_block;
        let ship_prev_hash = telos_args.prev_hash.clone();
        let ship_validate_hash = telos_args.validate_hash.clone();
        let ship_evm_deploy_block = telos_args.evm_deploy_block;
        let ship_evm_stop_block = telos_args.evm_stop_block;
        let ship_batch_size = telos_args.ship_batch_size;
        let consensus_client_binary = telos_args.consensus_client_binary.clone();
        // telos_endpoint doubles as the Antelope HTTP endpoint for the translator
        let ship_http_endpoint = telos_args.telos_endpoint.clone();

        // Capture the jwt.hex path and data dir from the builder config before launch
        let datadir = builder.config().datadir();
        let jwt_hex_path = datadir.jwt();
        let data_dir_path = datadir.data_dir().to_path_buf();

        let engine_tree_config = TreeConfig::default()
            .with_max_execute_block_batch_size(telos_args.max_execute_block_batch_size);

        let handle = builder
            .node(TelosNode::new(telos_args.clone()))
            .extend_rpc_modules(move |ctx| {
                if telos_args.telos_endpoint.is_some() {
                    ctx.registry
                        .eth_api()
                        .set_telos_client(TelosClient::new(telos_args.into()));
                }

                Ok(())
            })
            .launch_with_fn(|builder| {
                let launcher = EngineNodeLauncher::new(
                    builder.task_executor().clone(),
                    builder.config().datadir(),
                    engine_tree_config,
                );
                builder.launch_with(launcher)
            })
            .await?;

        // Start embedded SHIP sync if configured
        if let Some(ship_ep) = ship_endpoint {
            let chain_id = ship_chain_id.expect("--telos.chain_id is required when --telos.ship_endpoint is set");
            let evm_start_block = ship_evm_start_block.expect("--telos.evm_start_block is required when --telos.ship_endpoint is set");
            let prev_hash = ship_prev_hash.unwrap_or_else(|| "0000000000000000000000000000000000000000000000000000000000000000".to_string());
            let http_endpoint = ship_http_endpoint.expect("--telos.telos_endpoint is required when --telos.ship_endpoint is set (used as Antelope HTTP endpoint)");

            // Get the Engine API URL from the auth server handle
            let engine_api_url = handle.node.auth_server_handle().http_url();

            // Read JWT secret from the data directory's jwt.hex
            let jwt_secret = std::fs::read_to_string(&jwt_hex_path)
                .unwrap_or_else(|e| panic!("Failed to read JWT secret from {}: {}", jwt_hex_path.display(), e))
                .trim()
                .to_string();

            let ship_sync_config = reth_telos_ship_sync::ShipSyncConfig {
                binary_path: consensus_client_binary.map(std::path::PathBuf::from),
                ship_endpoint: ship_ep,
                http_endpoint,
                engine_api_url,
                jwt_secret,
                chain_id,
                evm_start_block,
                prev_hash,
                validate_hash: ship_validate_hash,
                evm_deploy_block: ship_evm_deploy_block,
                evm_stop_block: ship_evm_stop_block,
                batch_size: ship_batch_size,
                data_path: data_dir_path.clone(),
            };

            info!("Starting embedded SHIP sync");
            let _ship_handle = reth_telos_ship_sync::spawn_ship_sync(ship_sync_config);
        }

        match two_way_storage_compare {
            true => {
                if telos_rpc.is_none() {
                    warn!("Telos RPC Endpoint is not specified, skipping two-way storage compare");
                } else if block_delta.is_none() {
                    warn!("Block delta is not specified, skipping two-way storage compare");
                } else {
                    info!("Fetching account and accountstate from Telos native RPC (Can take a long time)...");

                    let (account_table, accountstate_table, block_number) = reth_node_telos::two_way_storage_compare::get_telos_tables(telos_rpc.unwrap().as_str(), block_delta.unwrap()).await;

                    if block_number.as_u64().unwrap() <= handle.node.provider.best_block_number().unwrap() {
                        info!("Two-way comparing state (Reth vs. Telos) at height: {:?}", block_number);

                        let state_at_specific_height = handle.node.provider.state_by_block_id(BlockId::Number(BlockNumberOrTag::Number(block_number.as_u64().unwrap()))).unwrap();
                        let plain_account_state = handle.node.provider.database_provider_ro().unwrap().table::<PlainAccountState>().unwrap();
                        let plain_storage_state = handle.node.provider.database_provider_ro().unwrap().table::<PlainStorageState>().unwrap();

                        let match_counter = reth_node_telos::two_way_storage_compare::two_side_state_compare(account_table, accountstate_table, state_at_specific_height, plain_account_state, plain_storage_state).await;
                        match_counter.print();

                        info!("Comparing done");
                    } else {
                        error!("Nodeos is ahead of reth, failed to compare state");
                    }
                }
            }
            _ => {}
        }

        handle.node_exit_future.await
    }) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
