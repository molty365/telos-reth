//! In-process SHIP translator + Engine API feeder.
//!
//! Embeds telos-translator-rs directly and feeds blocks to reth's Engine API
//! via local HTTP, eliminating the need for a separate consensus client binary.

use alloy_rpc_types_engine::ForkchoiceState;
use eyre::{Context, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use std::path::PathBuf;
use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::translator::{Translator, TranslatorConfig};
use telos_translator_rs::types::translator_types::ChainId;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// Configuration for the embedded SHIP sync.
#[derive(Debug, Clone)]
pub struct ShipSyncConfig {
    /// SHIP WebSocket endpoint (e.g. ws://localhost:29999)
    pub ship_endpoint: String,
    /// Antelope HTTP endpoint (e.g. http://localhost:8888)
    pub http_endpoint: String,
    /// URL of the local auth Engine API (e.g. http://127.0.0.1:8551)
    pub engine_api_url: String,
    /// JWT secret hex string for authenticating Engine API calls
    pub jwt_secret: String,
    /// Telos chain ID (40 for mainnet, 41 for testnet)
    pub chain_id: u64,
    /// EVM start block number
    pub evm_start_block: u32,
    /// Previous block hash for translator init
    pub prev_hash: String,
    /// Expected hash of the start block (optional)
    pub validate_hash: Option<String>,
    /// EVM deploy block (optional)
    pub evm_deploy_block: Option<u32>,
    /// EVM stop block (optional)
    pub evm_stop_block: Option<u32>,
    /// Batch size for Engine API newPayload calls
    pub batch_size: usize,
    /// Working directory for data
    pub data_path: PathBuf,
}

/// Generate a JWT token for Engine API authentication.
fn generate_jwt_token(secret_hex: &str) -> Result<String> {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    let secret_hex = secret_hex.strip_prefix("0x").unwrap_or(secret_hex).trim();
    let secret_bytes = hex::decode(secret_hex).wrap_err("Invalid JWT secret hex")?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let claims = json!({
        "iat": now,
        "exp": now + 60,
    });

    let header = Header::new(Algorithm::HS256);
    let key = EncodingKey::from_secret(&secret_bytes);
    encode(&header, &claims, &key).wrap_err("Failed to encode JWT")
}

/// Send a JSON-RPC request to the Engine API.
async fn engine_rpc(
    client: &reqwest::Client,
    url: &str,
    jwt_secret: &str,
    method: &str,
    params: Value,
) -> Result<Value> {
    let token = generate_jwt_token(jwt_secret)?;
    let body = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });

    let response = client
        .post(url)
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await
        .wrap_err("Engine API request failed")?;

    let json: Value = response.json().await.wrap_err("Failed to parse Engine API response")?;

    if let Some(error) = json.get("error") {
        error!("Engine API error: {:?}", error);
        return Err(eyre::eyre!("Engine API error: {}", error));
    }

    Ok(json["result"].clone())
}

/// Send a batch of JSON-RPC requests to the Engine API.
async fn engine_rpc_batch(
    client: &reqwest::Client,
    url: &str,
    jwt_secret: &str,
    requests: Vec<(&str, Value)>,
) -> Result<Vec<Value>> {
    let token = generate_jwt_token(jwt_secret)?;
    let batch: Vec<Value> = requests
        .iter()
        .enumerate()
        .map(|(id, (method, params))| {
            json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
                "id": id,
            })
        })
        .collect();

    let response = client
        .post(url)
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .header(CONTENT_TYPE, "application/json")
        .json(&batch)
        .send()
        .await
        .wrap_err("Engine API batch request failed")?;

    let json: Vec<Value> = response.json().await.wrap_err("Failed to parse batch response")?;

    // Check for errors
    let errors: Vec<String> = json
        .iter()
        .filter_map(|r| r.get("error").map(|e| e.to_string()))
        .collect();

    if !errors.is_empty() {
        return Err(eyre::eyre!("Engine API batch errors: {}", errors.join(", ")));
    }

    Ok(json.into_iter().map(|r| r["result"].clone()).collect())
}

/// Spawn the embedded SHIP sync as a tokio task.
///
/// The translator reads blocks from SHIP and sends them to the local Engine API.
pub fn spawn_ship_sync(config: ShipSyncConfig) -> tokio::task::JoinHandle<Result<()>> {
    tokio::spawn(async move { run_ship_sync(config).await })
}

async fn run_ship_sync(config: ShipSyncConfig) -> Result<()> {
    info!(
        ship_endpoint = %config.ship_endpoint,
        http_endpoint = %config.http_endpoint,
        engine_api = %config.engine_api_url,
        chain_id = config.chain_id,
        evm_start_block = config.evm_start_block,
        "Starting embedded SHIP sync (in-process translator)"
    );

    // Build translator config
    let translator_config = TranslatorConfig {
        chain_id: ChainId(config.chain_id),
        evm_deploy_block: config.evm_deploy_block,
        evm_start_block: config.evm_start_block,
        evm_stop_block: config.evm_stop_block,
        prev_hash: config.prev_hash.clone(),
        validate_hash: config.validate_hash.clone(),
        http_endpoint: config.http_endpoint.clone(),
        ship_endpoint: config.ship_endpoint.clone(),
        raw_message_channel_size: 1000,
        block_message_channel_size: 1000,
        final_message_channel_size: 1000,
    };

    // Channel for receiving translated blocks
    let (block_tx, mut block_rx) = mpsc::channel::<TelosEVMBlock>(1000);

    // Spawn the translator
    let translator = Translator::new(translator_config);
    let translator_handle = tokio::spawn(async move {
        if let Err(e) = translator.launch(Some(block_tx)).await {
            error!("Translator error: {:?}", e);
        }
    });

    // HTTP client for Engine API
    let client = reqwest::Client::new();
    let mut block_count: u64 = 0;

    info!("Waiting for blocks from translator...");

    loop {
        let block = match block_rx.recv().await {
            Some(block) => block,
            None => {
                info!("Translator channel closed, shutting down");
                break;
            }
        };

        let block_num = block.block_num;
        let _block_hash = block.block_hash;
        block_count += 1;

        if block_count % 1000 == 0 {
            info!(block_num, total_processed = block_count, "Processing blocks...");
        }

        // Send each block individually to allow persistence between blocks
        // This ensures the state provider can find parent state for each new block
        if let Err(e) = send_single_block(&client, &config, &block).await {
            warn!(block_num, error = %e, "Failed to send block, retrying after delay...");
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if let Err(e) = send_single_block(&client, &config, &block).await {
                error!(block_num, error = %e, "Retry failed, skipping block");
            }
        }
    }

    translator_handle.abort();
    Ok(())
}

async fn send_single_block(
    client: &reqwest::Client,
    config: &ShipSyncConfig,
    block: &TelosEVMBlock,
) -> Result<()> {
    // Send newPayload
    engine_rpc(
        client,
        &config.engine_api_url,
        &config.jwt_secret,
        "engine_newPayloadV1",
        json!([
            block.execution_payload,
            block.extra_fields,
        ]),
    )
    .await?;

    // Send fork choice update
    let fork_choice_state = ForkchoiceState {
        head_block_hash: block.block_hash,
        safe_block_hash: block.block_hash,
        finalized_block_hash: block.block_hash,
    };

    engine_rpc(
        client,
        &config.engine_api_url,
        &config.jwt_secret,
        "engine_forkchoiceUpdatedV1",
        json!([fork_choice_state]),
    )
    .await?;

    Ok(())
}
