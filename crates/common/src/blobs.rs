// Copyright 2024 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::client::log;
use crate::rkyv::kzg::{BlobDef, Bytes48Def};
use alloy_eips::eip4844::{
    kzg_to_versioned_hash, Blob, IndexedBlobHash, BLS_MODULUS, FIELD_ELEMENTS_PER_BLOB,
};
use alloy_primitives::{B256, U256};
use alloy_rpc_types_beacon::sidecar::BlobData;
use async_trait::async_trait;
use c_kzg::{ethereum_kzg_settings, Bytes48};
use kona_derive::errors::BlobProviderError;
use kona_derive::traits::BlobProvider;
use kona_protocol::BlockInfo;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobFetchRequest {
    pub block_ref: BlockInfo,
    pub blob_hash: IndexedBlobHash,
}

#[derive(
    Clone, Debug, Default, Serialize, Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct BlobWitnessData {
    #[rkyv(with = rkyv::with::Map<BlobDef>)]
    pub blobs: Vec<Blob>,
    #[rkyv(with = rkyv::with::Map<Bytes48Def>)]
    pub commitments: Vec<Bytes48>,
    #[rkyv(with = rkyv::with::Map<Bytes48Def>)]
    pub proofs: Vec<Bytes48>,
}

#[derive(Clone, Debug, Default)]
pub struct PreloadedBlobProvider {
    entries: Vec<(B256, Blob)>,
}

impl From<BlobWitnessData> for PreloadedBlobProvider {
    fn from(value: BlobWitnessData) -> Self {
        let blobs = value
            .blobs
            .into_iter()
            .map(|b| c_kzg::Blob::new(b.0))
            .collect::<Vec<_>>();
        ethereum_kzg_settings(0)
            .verify_blob_kzg_proof_batch(
                blobs.as_slice(),
                value.commitments.as_slice(),
                value.proofs.as_slice(),
            )
            .expect("Failed to batch validate kzg proofs");
        let hashes = value
            .commitments
            .iter()
            .map(|c| kzg_to_versioned_hash(c.as_slice()))
            .collect::<Vec<_>>();
        let entries = core::iter::zip(hashes, blobs.into_iter().map(|b| Blob::from(*b)))
            .rev()
            .collect::<Vec<_>>();
        Self { entries }
    }
}

#[async_trait]
impl BlobProvider for PreloadedBlobProvider {
    type Error = BlobProviderError;

    async fn get_blobs(
        &mut self,
        _block_ref: &BlockInfo,
        blob_hashes: &[IndexedBlobHash],
    ) -> Result<Vec<Box<Blob>>, Self::Error> {
        let blob_count = blob_hashes.len();
        log(&format!("FETCH {blob_count} BLOB(S)"));
        let mut blobs = Vec::with_capacity(blob_count);
        for hash in blob_hashes {
            let (blob_hash, blob) = self.entries.pop().unwrap();
            if hash.hash == blob_hash {
                blobs.push(Box::new(blob));
            }
        }
        Ok(blobs)
    }
}

pub fn intermediate_outputs(blob_data: &BlobData, blocks: usize) -> anyhow::Result<Vec<U256>> {
    field_elements(blob_data, 0..blocks)
}

pub fn trail_data(blob_data: &BlobData, blocks: usize) -> anyhow::Result<Vec<U256>> {
    field_elements(blob_data, blocks..FIELD_ELEMENTS_PER_BLOB as usize)
}

pub fn field_elements(
    blob_data: &BlobData,
    iterator: impl Iterator<Item = usize>,
) -> anyhow::Result<Vec<U256>> {
    let mut field_elements = vec![];
    for index in iterator.map(|i| 32 * i) {
        let bytes: [u8; 32] = blob_data.blob.0[index..index + 32].try_into()?;
        field_elements.push(U256::from_be_bytes(bytes));
    }
    Ok(field_elements)
}

pub fn hash_to_fe(hash: B256) -> U256 {
    U256::from_be_bytes(hash.0).reduce_mod(BLS_MODULUS)
}
