#![deny(missing_docs)]
//! holochain specific wrapper around more generic p2p module

use holo_hash::*;
use holochain_keystore::*;
use holochain_serialized_bytes::prelude::*;
use holochain_zome_types::{capability::CapSecret, zome::ZomeName};
use std::sync::Arc;

mod types;
pub use types::*;

mod spawn;
pub use spawn::*;

/// A wrapper around HolochainP2pSender that partially applies the dna_hash / agent_pub_key.
/// I.e. a sender that is tied to a specific cell.
#[derive(Clone)]
pub struct HolochainP2pCell {
    sender: actor::HolochainP2pSender,
    dna_hash: Arc<DnaHash>,
    from_agent: Arc<AgentPubKey>,
}

impl HolochainP2pCell {
    /// The p2p module must be informed at runtime which dna/agent pairs it should be tracking.
    pub async fn join(&mut self) -> actor::HolochainP2pResult<()> {
        self.sender
            .join((*self.dna_hash).clone(), (*self.from_agent).clone())
            .await
    }

    /// If a cell is deactivated, we'll need to \"leave\" the network module as well.
    pub async fn leave(&mut self) -> actor::HolochainP2pResult<()> {
        self.sender
            .leave((*self.dna_hash).clone(), (*self.from_agent).clone())
            .await
    }

    /// Invoke a zome function on a remote node (if you have been granted the capability).
    pub async fn call_remote(
        &mut self,
        to_agent: AgentPubKey,
        zome_name: ZomeName,
        fn_name: String,
        cap: CapSecret,
        request: SerializedBytes,
    ) -> actor::HolochainP2pResult<SerializedBytes> {
        self.sender
            .call_remote(
                (*self.dna_hash).clone(),
                (*self.from_agent).clone(),
                to_agent,
                zome_name,
                fn_name,
                cap,
                request,
            )
            .await
    }

    /// Publish data to the correct neigborhood.
    pub async fn publish(
        &mut self,
        request_validation_receipt: bool,
        dht_hash: holochain_types::composite_hash::AnyDhtHash,
        ops: Vec<(holo_hash::DhtOpHash, holochain_types::dht_op::DhtOp)>,
        timeout_ms: Option<u64>,
    ) -> actor::HolochainP2pResult<()> {
        self.sender
            .publish(
                (*self.dna_hash).clone(),
                (*self.from_agent).clone(),
                request_validation_receipt,
                dht_hash,
                ops,
                timeout_ms,
            )
            .await
    }

    /// Request a validation package.
    pub async fn get_validation_package(&mut self) -> actor::HolochainP2pResult<()> {
        self.sender
            .get_validation_package(actor::GetValidationPackage {
                dna_hash: (*self.dna_hash).clone(),
                agent_pub_key: (*self.from_agent).clone(),
            })
            .await
    }

    /// Get an entry from the DHT.
    pub async fn get(
        &mut self,
        dht_hash: holochain_types::composite_hash::AnyDhtHash,
        options: actor::GetOptions,
    ) -> actor::HolochainP2pResult<Vec<SerializedBytes>> {
        self.sender
            .get(
                (*self.dna_hash).clone(),
                (*self.from_agent).clone(),
                dht_hash,
                options,
            )
            .await
    }

    /// Get links from the DHT.
    pub async fn get_links(&mut self) -> actor::HolochainP2pResult<()> {
        self.sender
            .get_links(actor::GetLinks {
                dna_hash: (*self.dna_hash).clone(),
                agent_pub_key: (*self.from_agent).clone(),
            })
            .await
    }

    /// Send a validation receipt to a remote node.
    pub async fn send_validation_receipt(
        &mut self,
        receipt: SerializedBytes,
    ) -> actor::HolochainP2pResult<()> {
        self.sender
            .send_validation_receipt(
                (*self.dna_hash).clone(),
                (*self.from_agent).clone(),
                receipt,
            )
            .await
    }
}

mod test;