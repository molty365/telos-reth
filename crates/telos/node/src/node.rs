//! Telos node implementation

use crate::engine::TelosEngineValidator;
use std::sync::Arc;

use reth_chainspec::{ChainSpec, EthereumHardforks, Hardforks};
use reth_ethereum_engine_primitives::{
    EthBuiltPayload, EthEngineTypes, EthPayloadAttributes, EthPayloadBuilderAttributes,
};
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::{ConfigureEvm, EvmFactory, EvmFactoryFor, NextBlockEnvAttributes};
use reth_node_api::{FullNodeComponents, HeaderTy, PrimitivesTy};
use alloy_rpc_types_engine::ExecutionData as EngineExecutionData;
use reth_engine_primitives::EngineTypes;
use reth_node_api::AddOnsContext;
use reth_node_builder::{
    components::{BasicPayloadServiceBuilder, ComponentsBuilder},
    node::FullNodeTypes,
    rpc::{BasicEngineApiBuilder, BasicEngineValidatorBuilder, EthApiBuilder, EthApiCtx, PayloadValidatorBuilder, RpcAddOns},
    Node, NodeAdapter,
};
use reth_node_ethereum::node::{
    EthereumAddOns, EthereumConsensusBuilder, EthereumEngineValidatorBuilder,
    EthereumExecutorBuilder, EthereumNetworkBuilder, EthereumPayloadBuilder, EthereumPoolBuilder,
};
use reth_node_types::NodeTypes;
use reth_payload_primitives::PayloadTypes;
use reth_provider::EthStorage;
use reth_rpc::eth::core::EthRpcConverterFor;
use reth_rpc_convert::RpcConvert;
use reth_rpc_eth_api::helpers::pending_block::BuildPendingEnv;
use reth_rpc_eth_types::{error::FromEvmError, EthApiError};
use reth_telos_rpc::eth::TelosEthApi;
use revm::context::TxEnv;

use crate::args::TelosArgs;

/// Type configuration for a regular Telos node.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct TelosNode {
    /// Additional Telos args
    pub args: TelosArgs,
}

impl TelosNode {
    /// Creates a new instance of the Telos node type.
    pub const fn new(args: TelosArgs) -> Self {
        Self { args }
    }

    /// Returns a [`ComponentsBuilder`] configured for a regular Ethereum node.
    pub fn components<Node>() -> ComponentsBuilder<
        Node,
        EthereumPoolBuilder,
        BasicPayloadServiceBuilder<EthereumPayloadBuilder>,
        EthereumNetworkBuilder,
        EthereumExecutorBuilder,
        EthereumConsensusBuilder,
    >
    where
        Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
        <Node::Types as NodeTypes>::Payload: PayloadTypes<
            BuiltPayload = EthBuiltPayload,
            PayloadAttributes = EthPayloadAttributes,
            PayloadBuilderAttributes = EthPayloadBuilderAttributes,
        >,
    {
        ComponentsBuilder::default()
            .node_types::<Node>()
            .pool(EthereumPoolBuilder::default())
            .executor(EthereumExecutorBuilder::default())
            .payload(BasicPayloadServiceBuilder::default())
            .network(EthereumNetworkBuilder::default())
            .consensus(EthereumConsensusBuilder::default())
    }
}

impl NodeTypes for TelosNode {
    type Primitives = EthPrimitives;
    type ChainSpec = ChainSpec;
    type Storage = EthStorage;
    type Payload = EthEngineTypes;
}

/// Builds [`TelosEthApi`] for Telos.
#[derive(Debug, Default)]
pub struct TelosEthApiBuilder;

impl<N> EthApiBuilder<N> for TelosEthApiBuilder
where
    N: FullNodeComponents<
        Types: NodeTypes<ChainSpec: Hardforks + EthereumHardforks>,
        Evm: ConfigureEvm<NextBlockEnvCtx: BuildPendingEnv<HeaderTy<N::Types>>>,
    >,
    EthRpcConverterFor<N>: RpcConvert<
        Primitives = PrimitivesTy<N::Types>,
        Error = EthApiError,
        Evm = N::Evm,
    >,
    EthApiError: FromEvmError<N::Evm>,
    EvmFactoryFor<N::Evm>: EvmFactory<Tx = TxEnv>,
{
    type EthApi = TelosEthApi<N, EthRpcConverterFor<N>>;

    async fn build_eth_api(self, ctx: EthApiCtx<'_, N>) -> eyre::Result<Self::EthApi> {
        let inner = ctx.eth_api_builder().map_converter(|r| r.with_network()).build_inner();
        Ok(TelosEthApi::new(Arc::new(inner)))
    }
}

/// Telos engine validator builder - uses TelosEngineValidator which trusts
/// the block_hash from the consensus client instead of recomputing it.
/// This is needed for compatibility with the legacy Telos consensus client (alloy 0.3.x).
#[derive(Debug, Clone, Default)]
pub struct TelosEngineValidatorBuilder;

impl<Node, Types> PayloadValidatorBuilder<Node> for TelosEngineValidatorBuilder
where
    Types: NodeTypes<
        ChainSpec: reth_chainspec::Hardforks + reth_chainspec::EthereumHardforks + Clone + 'static,
        Payload: EngineTypes<ExecutionData = EngineExecutionData>
                     + PayloadTypes<PayloadAttributes = EthPayloadAttributes>,
        Primitives = reth_ethereum_primitives::EthPrimitives,
    >,
    Node: FullNodeComponents<Types = Types>,
{
    type Validator = TelosEngineValidator<Types::ChainSpec>;

    async fn build(self, ctx: &AddOnsContext<'_, Node>) -> eyre::Result<Self::Validator> {
        Ok(TelosEngineValidator::new(ctx.config.chain.clone()))
    }
}

impl<N> Node<N> for TelosNode
where
    N: FullNodeTypes<Types = Self>,
{
    type ComponentsBuilder = ComponentsBuilder<
        N,
        EthereumPoolBuilder,
        BasicPayloadServiceBuilder<EthereumPayloadBuilder>,
        EthereumNetworkBuilder,
        EthereumExecutorBuilder,
        EthereumConsensusBuilder,
    >;

    type AddOns =
        EthereumAddOns<NodeAdapter<N>, TelosEthApiBuilder, TelosEngineValidatorBuilder>;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        Self::components()
    }

    fn add_ons(&self) -> Self::AddOns {
        EthereumAddOns::new(RpcAddOns::new(
            TelosEthApiBuilder,
            TelosEngineValidatorBuilder::default(),
            BasicEngineApiBuilder::default(),
            BasicEngineValidatorBuilder::default(),
            Default::default(),
        ))
    }
}
