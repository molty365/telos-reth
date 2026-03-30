//! Managed telos-consensus-client subprocess.
//!
//! Spawns the consensus client binary as a child process, configured to
//! connect to the local reth Engine API.

use eyre::{Context, Result};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{debug, error, info, warn};

/// Configuration for the embedded SHIP sync.
#[derive(Debug, Clone)]
pub struct ShipSyncConfig {
    /// Path to the telos-consensus-client binary.
    /// If not provided, searches PATH for "telos-consensus-client".
    pub binary_path: Option<PathBuf>,
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
    /// Working directory for the consensus client (for RocksDB data)
    pub data_path: PathBuf,
}

/// The consensus client config file format (matches AppConfig in the client).
#[derive(Debug)]
struct ConsensusClientConfig {
    log_level: String,
    chain_id: u64,
    execution_endpoint: String,
    jwt_secret: String,
    ship_endpoint: String,
    chain_endpoint: String,
    batch_size: usize,
    prev_hash: String,
    evm_deploy_block: Option<u32>,
    evm_start_block: u32,
    validate_hash: Option<String>,
    evm_stop_block: Option<u32>,
    data_path: String,
    block_checkpoint_interval: u32,
    latest_blocks_in_db_num: u32,
    maximum_sync_range: u32,
}

/// Spawn the consensus client as a managed subprocess.
///
/// Returns a join handle. The subprocess is automatically killed when dropped.
pub fn spawn_ship_sync(
    config: ShipSyncConfig,
) -> tokio::task::JoinHandle<Result<()>> {
    tokio::spawn(async move {
        run_managed_client(config).await
    })
}

async fn run_managed_client(config: ShipSyncConfig) -> Result<()> {
    // Create the data directory for the consensus client
    let data_path = config.data_path.join("consensus-client");
    std::fs::create_dir_all(&data_path)
        .wrap_err_with(|| format!("Failed to create data dir: {}", data_path.display()))?;

    // Write the config file
    let client_config = ConsensusClientConfig {
        log_level: "info".to_string(),
        chain_id: config.chain_id,
        execution_endpoint: config.engine_api_url.clone(),
        jwt_secret: config.jwt_secret.clone(),
        ship_endpoint: config.ship_endpoint.clone(),
        chain_endpoint: config.http_endpoint.clone(),
        batch_size: config.batch_size,
        prev_hash: config.prev_hash.clone(),
        evm_deploy_block: config.evm_deploy_block,
        evm_start_block: config.evm_start_block,
        validate_hash: config.validate_hash.clone(),
        evm_stop_block: config.evm_stop_block,
        data_path: data_path.to_string_lossy().to_string(),
        block_checkpoint_interval: 1000,
        latest_blocks_in_db_num: 100,
        maximum_sync_range: 1000000,
    };

    let config_path = data_path.join("config.toml");
    let config_toml = toml_string(&client_config)?;
    std::fs::write(&config_path, &config_toml)
        .wrap_err_with(|| format!("Failed to write config: {}", config_path.display()))?;

    info!(
        config_path = %config_path.display(),
        ship_endpoint = %config.ship_endpoint,
        engine_api = %config.engine_api_url,
        chain_id = config.chain_id,
        evm_start_block = config.evm_start_block,
        "Starting managed consensus client"
    );

    // Find the binary
    let binary = config.binary_path
        .unwrap_or_else(|| PathBuf::from("telos-consensus-client"));

    loop {
        let mut child = Command::new(&binary)
            .arg("--config")
            .arg(&config_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .wrap_err_with(|| format!(
                "Failed to spawn consensus client. Binary: {}. \
                 Install it from https://github.com/telosnetwork/telos-consensus-client \
                 or provide --telos.consensus_client_binary",
                binary.display()
            ))?;

        info!(pid = child.id(), "Consensus client started");

        // Forward stdout/stderr with prefixed logging
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        if let Some(stdout) = stdout {
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(target: "telos::consensus-client", "{}", line);
                }
            });
        }

        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    warn!(target: "telos::consensus-client", "{}", line);
                }
            });
        }

        let status = child.wait().await
            .wrap_err("Failed to wait for consensus client")?;

        if status.success() {
            info!("Consensus client exited cleanly");
            return Ok(());
        }

        error!(
            exit_code = status.code(),
            "Consensus client exited with error, restarting in 5s"
        );
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

fn toml_string(config: &ConsensusClientConfig) -> Result<String> {
    // Manual TOML serialization to avoid adding toml dependency
    let mut s = String::new();
    s.push_str(&format!("log_level = \"{}\"\n", config.log_level));
    s.push_str(&format!("chain_id = {}\n", config.chain_id));
    s.push_str(&format!("execution_endpoint = \"{}\"\n", config.execution_endpoint));
    s.push_str(&format!("jwt_secret = \"{}\"\n", config.jwt_secret));
    s.push_str(&format!("ship_endpoint = \"{}\"\n", config.ship_endpoint));
    s.push_str(&format!("chain_endpoint = \"{}\"\n", config.chain_endpoint));
    s.push_str(&format!("batch_size = {}\n", config.batch_size));
    s.push_str(&format!("prev_hash = \"{}\"\n", config.prev_hash));
    if let Some(v) = config.evm_deploy_block {
        s.push_str(&format!("evm_deploy_block = {}\n", v));
    }
    s.push_str(&format!("evm_start_block = {}\n", config.evm_start_block));
    if let Some(ref v) = config.validate_hash {
        s.push_str(&format!("validate_hash = \"{}\"\n", v));
    }
    if let Some(v) = config.evm_stop_block {
        s.push_str(&format!("evm_stop_block = {}\n", v));
    }
    s.push_str(&format!("data_path = \"{}\"\n", config.data_path));
    s.push_str(&format!("block_checkpoint_interval = {}\n", config.block_checkpoint_interval));
    s.push_str(&format!("latest_blocks_in_db_num = {}\n", config.latest_blocks_in_db_num));
    s.push_str(&format!("maximum_sync_range = {}\n", config.maximum_sync_range));
    Ok(s)
}
