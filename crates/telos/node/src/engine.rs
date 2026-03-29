//! Telos engine validator - overrides block hash validation for legacy consensus client compatibility.
//!
//! The Telos consensus client (alloy 0.3.x) computes block_hash with base_fee_per_gas=None
//! but sends a non-zero base_fee_per_gas in the ExecutionPayloadV1. Reth v1.11.x recomputes
//! the hash using the payload's base_fee_per_gas and gets a mismatch.
//!
//! Fix: trust the block_hash provided by the consensus client, skip hash recomputation.

use alloy_rpc_types_engine::{ExecutionData, PayloadError};
use reth_chainspec::{EthChainSpec, EthereumHardforks};
use reth_engine_primitives::{EngineApiValidator, PayloadValidator};
use reth_ethereum_engine_primitives::EthPayloadAttributes;
use reth_ethereum_primitives::Block;
use reth_node_api::PayloadTypes;
use reth_payload_primitives::{
    validate_execution_requests, validate_version_specific_fields, EngineApiMessageVersion,
    EngineObjectValidationError, NewPayloadError, PayloadOrAttributes,
};
use reth_payload_validator::{cancun, prague, shanghai};
use reth_primitives_traits::{SealedBlock, RecoveredBlock, Block as BlockTrait, SignedTransaction};
use reth_engine_primitives::NewPayloadError;
use std::sync::Arc;

/// Telos engine validator that trusts block_hash from the consensus client.
#[derive(Debug, Clone)]
pub struct TelosEngineValidator<ChainSpec = reth_chainspec::ChainSpec> {
    chain_spec: Arc<ChainSpec>,
}

impl<ChainSpec> TelosEngineValidator<ChainSpec> {
    /// Create a new Telos engine validator.
    pub const fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self { chain_spec }
    }
}

/// Convert payload to block, trusting the block_hash from the consensus client.
///
/// This is a modified version of `EthereumExecutionPayloadValidator::ensure_well_formed_payload`
/// that replaces `seal_slow()` with `from_parts_unchecked()` to avoid recomputing the hash.
///
/// The legacy Telos consensus client (alloy 0.3.x) computes hashes with `base_fee_per_gas: None`
/// in the header RLP but sends a non-zero `base_fee_per_gas` in the `ExecutionPayloadV1`.
fn telos_ensure_well_formed_payload(
    chain_spec: &impl EthereumHardforks,
    payload: ExecutionData,
) -> Result<SealedBlock<Block>, PayloadError> {
    let ExecutionData { payload, sidecar } = payload;

    // KEY FIX: preserve the hash from the consensus client, don't recompute it
    let trusted_hash = payload.block_hash();

    // Parse the block (this sets base_fee_per_gas=Some(...) in the header)
    let block: Block = payload.try_into_block_with_sidecar(&sidecar)?;

    // Seal with the trusted hash instead of recomputing via seal_slow()
    let alloy_consensus::Block { header, body } = block;
    let sealed_block = SealedBlock::<Block>::from_parts_unchecked(header, body, trusted_hash);

    // Still validate EIP structural requirements (just not the hash)
    shanghai::ensure_well_formed_fields(
        sealed_block.body(),
        chain_spec.is_shanghai_active_at_timestamp(sealed_block.timestamp),
    )?;

    cancun::ensure_well_formed_fields(
        &sealed_block,
        sidecar.cancun(),
        chain_spec.is_cancun_active_at_timestamp(sealed_block.timestamp),
    )?;

    prague::ensure_well_formed_fields(
        sealed_block.body(),
        sidecar.prague(),
        chain_spec.is_prague_active_at_timestamp(sealed_block.timestamp),
    )?;

    Ok(sealed_block)
}

impl<ChainSpec, Types> PayloadValidator<Types> for TelosEngineValidator<ChainSpec>
where
    ChainSpec: EthChainSpec + EthereumHardforks + 'static,
    Types: PayloadTypes<ExecutionData = ExecutionData>,
{
    type Block = Block;

    fn convert_payload_to_block(
        &self,
        payload: ExecutionData,
    ) -> Result<SealedBlock<Self::Block>, NewPayloadError> {
        telos_ensure_well_formed_payload(self.chain_spec.as_ref(), payload).map_err(Into::into)
    }

    fn ensure_well_formed_payload(
        &self,
        payload: ExecutionData,
    ) -> Result<RecoveredBlock<Self::Block>, NewPayloadError> {
        let sealed_block = self.convert_payload_to_block(payload)?;
        
        // Telos: Custom recovery that handles non-standard signatures
        // In Telos, system transactions encode the sender in the S field
        let block = sealed_block.clone_sealed_header();
        let txs = sealed_block.body().transactions();
        let mut senders = Vec::with_capacity(txs.len());
        
        for tx in txs {
            match tx.recover_signer() {
                Ok(addr) => senders.push(addr),
                Err(_) => {
                    // Telos recovery: sender address is in the first 20 bytes of S
                    let s = tx.signature().s();
                    let s_bytes = s.to_be_bytes::<32>();
                    let addr = alloy_primitives::Address::from_slice(&s_bytes[..20]);
                    senders.push(addr);
                }
            }
        }
        
        let (header, body) = sealed_block.split();
        Ok(RecoveredBlock::new_sealed(header, body, senders))
    }
}

impl<ChainSpec, Types> EngineApiValidator<Types> for TelosEngineValidator<ChainSpec>
where
    ChainSpec: EthChainSpec + EthereumHardforks + 'static,
    Types: PayloadTypes<PayloadAttributes = EthPayloadAttributes, ExecutionData = ExecutionData>,
{
    fn validate_version_specific_fields(
        &self,
        version: EngineApiMessageVersion,
        payload_or_attrs: PayloadOrAttributes<'_, Types::ExecutionData, EthPayloadAttributes>,
    ) -> Result<(), EngineObjectValidationError> {
        payload_or_attrs
            .execution_requests()
            .map(|requests| validate_execution_requests(requests))
            .transpose()?;

        validate_version_specific_fields(&self.chain_spec, version, payload_or_attrs)
    }

    fn ensure_well_formed_attributes(
        &self,
        version: EngineApiMessageVersion,
        attributes: &EthPayloadAttributes,
    ) -> Result<(), EngineObjectValidationError> {
        validate_version_specific_fields(
            &self.chain_spec,
            version,
            PayloadOrAttributes::<Types::ExecutionData, EthPayloadAttributes>::PayloadAttributes(
                attributes,
            ),
        )
    }
}
