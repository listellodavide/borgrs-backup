//! Recovery system using Reed-Solomon erasure coding
//!
//! Implements chunk-level error correction.

use crate::error::{BorgError, Result};
use reed_solomon_erasure::galois_8::ReedSolomon;
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Write};
use std::convert::TryInto;

const DATA_SHARDS: usize = 16;
const MAGIC: &[u8; 4] = b"PARR";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum RecoveryProfile {
    /// ~6.25% overhead (1 parity for 16 data)
    Low = 7,
    /// ~12.5% overhead (2 parity for 16 data)
    Medium = 13,
    /// ~25% overhead (4 parity for 16 data)
    High = 25,
}

impl Default for RecoveryProfile {
    fn default() -> Self {
        RecoveryProfile::Medium
    }
}

impl RecoveryProfile {
    pub fn parity_shards(&self) -> usize {
        match self {
            RecoveryProfile::Low => 1,
            RecoveryProfile::Medium => 2,
            RecoveryProfile::High => 4,
        }
    }
}

pub struct RecoveryCodec {
    rs: ReedSolomon,
    parity_shards: usize,
}

impl RecoveryCodec {
    pub fn new(profile: RecoveryProfile) -> Result<Self> {
        let parity_shards = profile.parity_shards();
        let rs = ReedSolomon::new(DATA_SHARDS, parity_shards)
            .map_err(|e| BorgError::Internal(format!("Failed to create RS codec: {}", e)))?;
        
        Ok(Self { rs, parity_shards })
    }

    /// Generate parity data for a blob
    pub fn encode(&self, data: &[u8]) -> Result<Vec<u8>> {
        let original_len = data.len();
        let shard_size = (original_len + DATA_SHARDS - 1) / DATA_SHARDS;

        // Prepare data shards
        let mut shards: Vec<Vec<u8>> = Vec::with_capacity(DATA_SHARDS + self.parity_shards);
        let mut crcs = Vec::with_capacity(DATA_SHARDS);
        
        for i in 0..DATA_SHARDS {
            let start = i * shard_size;
            let mut shard = vec![0u8; shard_size];
            if start < original_len {
                let end = std::cmp::min(start + shard_size, original_len);
                shard[0..(end - start)].copy_from_slice(&data[start..end]);
            }
            // Calculate CRC for this shard to help identification during recovery
            crcs.push(crc32fast::hash(&shard));
            shards.push(shard);
        }

        for _ in 0..self.parity_shards {
            shards.push(vec![0u8; shard_size]);
        }

        self.rs.encode(&mut shards)
            .map_err(|e| BorgError::Internal(format!("RS encode failed: {}", e)))?;

        // Format: Header + CRCs + Parity Shards
        let mut output = Vec::new();
        output.write_all(MAGIC)?;
        output.write_all(&[1u8])?; // Version
        output.write_all(&[DATA_SHARDS as u8])?;
        output.write_all(&[self.parity_shards as u8])?;
        output.write_all(&[0u8])?; // Reserved
        output.write_all(&(original_len as u64).to_le_bytes())?;
        
        // Write CRCs (4 bytes * 16 = 64 bytes)
        for crc in crcs {
            output.write_all(&crc.to_le_bytes())?;
        }

        // Write parity shards
        for i in 0..self.parity_shards {
            output.write_all(&shards[DATA_SHARDS + i])?;
        }

        Ok(output)
    }

    /// Reconstruct data using corrupted data and parity blob
    pub fn reconstruct(&self, corrupted_data: &[u8], parity_blob: &[u8]) -> Result<Vec<u8>> {
        if parity_blob.len() < 16 + (DATA_SHARDS * 4) || &parity_blob[0..4] != MAGIC {
            return Err(BorgError::IntegrityCheck { expected: "PARR magic".into(), actual: "invalid".into() });
        }
        
        let mut rdr = Cursor::new(&parity_blob[4..]);
        let mut header = [0u8; 12];
        rdr.read_exact(&mut header)?;
        
        let data_shards_stored = header[1] as usize;
        let parity_shards_stored = header[2] as usize;
        let original_len = u64::from_le_bytes(header[4..12].try_into().unwrap()) as usize;

        if data_shards_stored != DATA_SHARDS || parity_shards_stored != self.parity_shards {
             return Err(BorgError::Internal("Recovery profile mismatch".into()));
        }

        // Read CRCs
        let mut stored_crcs = Vec::with_capacity(DATA_SHARDS);
        for _ in 0..DATA_SHARDS {
            let mut buf = [0u8; 4];
            rdr.read_exact(&mut buf)?;
            stored_crcs.push(u32::from_le_bytes(buf));
        }

        let shard_size = (original_len + DATA_SHARDS - 1) / DATA_SHARDS;
        
        // Split corrupted data into shards and verify against CRCs
        let mut shards: Vec<Option<Vec<u8>>> = vec![None; DATA_SHARDS + self.parity_shards];
        let mut valid_count = 0;

        for i in 0..DATA_SHARDS {
            let start = i * shard_size;
            let mut shard = vec![0u8; shard_size];
            if start < corrupted_data.len() {
                let end = std::cmp::min(start + shard_size, corrupted_data.len());
                shard[0..(end - start)].copy_from_slice(&corrupted_data[start..end]);
            }
            
            let crc = crc32fast::hash(&shard);
            if crc == stored_crcs[i] {
                shards[i] = Some(shard);
                valid_count += 1;
            } else {
                // Shard is corrupt, leave as None (erasure)
            }
        }

        // Load parity shards
        for i in 0..self.parity_shards {
            let mut shard = vec![0u8; shard_size];
            rdr.read_exact(&mut shard)?;
            shards[DATA_SHARDS + i] = Some(shard);
            valid_count += 1;
        }

        if valid_count < DATA_SHARDS {
            return Err(BorgError::Internal("Not enough valid shards to recover".into()));
        }

        self.rs.reconstruct(&mut shards)
            .map_err(|e| BorgError::Internal(format!("RS reconstruct failed: {}", e)))?;

        // Reassemble
        let mut result = Vec::with_capacity(original_len);
        for i in 0..DATA_SHARDS {
            if let Some(shard) = &shards[i] {
                result.extend_from_slice(shard);
            }
        }
        
        result.truncate(original_len);
        Ok(result)
    }
}