//! Loads and formats Telos transaction RPC response.

use std::time::Duration;

use alloy_primitives::{Bytes, B256};
use reth_primitives_traits::{Recovered, WithEncoded};
use reth_rpc_convert::RpcConvert;
use reth_rpc_eth_api::{
    helpers::{spec::SignersForRpc, EthTransactions, LoadTransaction, SpawnBlocking},
    FromEthApiError, FromEvmError, FullEthApiTypes, RpcNodeCore,
};
use reth_rpc_eth_types::EthApiError;
use reth_transaction_pool::{PoolPooledTx, PoolTransaction, TransactionPool};

use crate::eth::TelosClient;
use crate::eth::TelosEthApi;

impl<N, Rpc> EthTransactions for TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    EthApiError: FromEvmError<N::Evm>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError>,
{
    fn signers(&self) -> &SignersForRpc<Self::Provider, Self::NetworkTypes> {
        self.inner.signers()
    }

    fn send_raw_transaction_sync_timeout(&self) -> Duration {
        self.inner.send_raw_transaction_sync_timeout()
    }

    /// Submits a raw transaction to the Telos native network for inclusion in a block.
    async fn send_transaction(
        &self,
        origin: reth_transaction_pool::TransactionOrigin,
        tx: WithEncoded<Recovered<PoolPooledTx<Self::Pool>>>,
    ) -> Result<B256, Self::Error> {
        let (raw_tx, recovered) = tx.split();
        let pool_transaction =
            <Self::Pool as TransactionPool>::Transaction::from_pooled(recovered);

        // On Telos, transactions are forwarded directly to the native network to be included in a
        // block.
        if let Some(client) = self.raw_tx_forwarder().as_ref() {
            tracing::debug!(target: "rpc::eth", "forwarding raw transaction to Telos native");
            let result = client.send_to_telos(&raw_tx).await.inspect_err(|err| {
                tracing::debug!(target: "rpc::eth", %err, hash=% *pool_transaction.hash(), "failed to forward raw transaction");
            });

            if let Err(err) = result {
                return Err(err);
            }
        }

        let hash = *pool_transaction.hash();
        Ok(hash)
    }
}

impl<N, Rpc> LoadTransaction for TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    EthApiError: FromEvmError<N::Evm>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError>,
{
}

impl<N, Rpc> TelosEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert,
{
    /// Sets a [`TelosClient`] for `eth_sendRawTransaction` to forward transactions to.
    pub fn set_telos_client(&self, telos_client: TelosClient) {
        self.telos_client.set(telos_client).expect("Telos client can be set only once");
    }

    /// Returns the [`TelosClient`] if one is set.
    pub fn raw_tx_forwarder(&self) -> Option<TelosClient> {
        self.telos_client.get().cloned()
    }
}
