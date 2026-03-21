//! Telos `eth_` endpoint implementation.

pub mod receipt;
pub mod transaction;

mod block;
mod call;
mod pending_block;

/// Client for interacting with Telos node.
pub mod telos_client;

use std::{fmt, sync::Arc};

use crate::TelosClient;
use alloy_primitives::U256;
use derive_more::Deref;
use reth_rpc::eth::core::EthApiInner;
use reth_rpc_convert::RpcConvert;
use reth_rpc_eth_api::{
    helpers::{
        pending_block::PendingEnvBuilder,
        spec::SignersForRpc,
        EthApiSpec, EthFees, EthState, LoadFee, LoadPendingBlock, LoadState, SpawnBlocking, Trace,
    },
    node::RpcNodeCoreExt,
    EthApiTypes, FromEvmError, RpcNodeCore,
};
use reth_rpc_eth_types::{
    builder::config::PendingBlockKind, EthApiError, EthStateCache, FeeHistoryCache,
    GasPriceOracle, PendingBlock,
};
use reth_storage_api::ProviderHeader;
use reth_tasks::{
    pool::{BlockingTaskGuard, BlockingTaskPool},
    TaskSpawner,
};
use tokio::sync::OnceCell;

use crate::error::TelosEthApiError;

/// Telos `Eth` API implementation.
///
/// This type provides the functionality for handling `eth_` related requests.
///
/// This wraps a default `Eth` implementation, and provides additional functionality where the
/// Telos spec deviates from the default (ethereum) spec, e.g. transaction forwarding to the
/// native network
///
/// This type implements the [`FullEthApi`](reth_rpc_eth_api::helpers::FullEthApi) by implemented
/// all the `Eth` helper traits and prerequisite traits.
#[derive(Deref)]
pub struct TelosEthApi<N: RpcNodeCore, Rpc: RpcConvert> {
    /// Gateway to node's core components.
    #[deref]
    inner: Arc<EthApiInner<N, Rpc>>,
    telos_client: Arc<OnceCell<TelosClient>>,
}

impl<N: RpcNodeCore, Rpc: RpcConvert> Clone for TelosEthApi<N, Rpc> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), telos_client: self.telos_client.clone() }
    }
}

impl<N: RpcNodeCore, Rpc: RpcConvert> TelosEthApi<N, Rpc> {
    /// Creates a new `TelosEthApi` wrapping the given `EthApiInner`.
    pub fn new(inner: Arc<EthApiInner<N, Rpc>>) -> Self {
        Self { inner, telos_client: Arc::new(OnceCell::new()) }
    }
}

impl<N, Rpc> EthApiTypes for TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Error = EthApiError>,
{
    type Error = TelosEthApiError;
    type NetworkTypes = Rpc::Network;
    type RpcConvert = Rpc;

    fn converter(&self) -> &Self::RpcConvert {
        self.inner.converter()
    }
}

impl<N, Rpc> RpcNodeCore for TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert,
{
    type Primitives = N::Primitives;
    type Provider = N::Provider;
    type Pool = N::Pool;
    type Evm = N::Evm;
    type Network = N::Network;

    fn pool(&self) -> &Self::Pool {
        self.inner.pool()
    }

    fn evm_config(&self) -> &Self::Evm {
        self.inner.evm_config()
    }

    fn network(&self) -> &Self::Network {
        self.inner.network()
    }

    fn provider(&self) -> &Self::Provider {
        self.inner.provider()
    }
}

impl<N, Rpc> RpcNodeCoreExt for TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert,
{
    #[inline]
    fn cache(&self) -> &EthStateCache<N::Primitives> {
        self.inner.cache()
    }
}

impl<N, Rpc> EthApiSpec for TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError>,
{
    #[inline]
    fn starting_block(&self) -> U256 {
        self.inner.starting_block()
    }
}

impl<N, Rpc> SpawnBlocking for TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Error = EthApiError>,
{
    #[inline]
    fn io_task_spawner(&self) -> impl TaskSpawner {
        self.inner.task_spawner()
    }

    #[inline]
    fn tracing_task_pool(&self) -> &BlockingTaskPool {
        self.inner.blocking_task_pool()
    }

    #[inline]
    fn tracing_task_guard(&self) -> &BlockingTaskGuard {
        self.inner.blocking_task_guard()
    }

    #[inline]
    fn blocking_io_task_guard(&self) -> &std::sync::Arc<tokio::sync::Semaphore> {
        self.inner.blocking_io_request_semaphore()
    }
}

impl<N, Rpc> LoadFee for TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    EthApiError: FromEvmError<N::Evm>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError>,
{
    #[inline]
    fn gas_oracle(&self) -> &GasPriceOracle<Self::Provider> {
        self.inner.gas_oracle()
    }

    #[inline]
    fn fee_history_cache(&self) -> &FeeHistoryCache<ProviderHeader<N::Provider>> {
        self.inner.fee_history_cache()
    }
}

impl<N, Rpc> LoadState for TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives>,
    Self: LoadPendingBlock,
{
}

impl<N, Rpc> EthState for TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError>,
    Self: LoadPendingBlock,
{
    #[inline]
    fn max_proof_window(&self) -> u64 {
        self.inner.eth_proof_window()
    }
}

impl<N, Rpc> EthFees for TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    EthApiError: FromEvmError<N::Evm>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError>,
{
}

impl<N, Rpc> Trace for TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    EthApiError: FromEvmError<N::Evm>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
{
}

impl<N: RpcNodeCore, Rpc: RpcConvert> fmt::Debug for TelosEthApi<N, Rpc> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelosEthApi").finish_non_exhaustive()
    }
}
