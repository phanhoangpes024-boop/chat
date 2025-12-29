mod proto {
    include!(concat!(env!("OUT_DIR"), "/turbochat.v1.rs"));
}

pub use proto::*;

use bytes::Bytes;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("CRC mismatch: expected {expected:08x}, got {actual:08x}")]
    CrcMismatch { expected: u32, actual: u32 },

    #[error("Protobuf decode error: {0}")]
    DecodeError(#[from] prost::DecodeError),

    #[error("Database error: {0}")]
    DbError(String),

    #[error("Auth error: {0}")]
    AuthError(String),
}

impl Message {
    pub fn new(
        shop_id: String,
        guest_id: u64,
        message_id: u64,
        sender_type: String,
        content: Bytes,
        timestamp_us: u64,
    ) -> Self {
        let content_crc = crc32c::crc32c(&content);
        Self {
            shop_id,
            guest_id,
            message_id,
            sender_type,
            content,
            timestamp_us,
            content_crc,
        }
    }

    pub fn verify_content(&self) -> Result<(), ContractError> {
        let computed = crc32c::crc32c(&self.content);
        if computed != self.content_crc {
            return Err(ContractError::CrcMismatch {
                expected: self.content_crc,
                actual: computed,
            });
        }
        Ok(())
    }
}

impl SyncResponse {
    pub fn compute_crc(&self) -> u32 {
        use prost::Message as ProstMessage;
        let mut buf = Vec::with_capacity(self.messages.len() * 128 + 32);
        for msg in &self.messages {
            buf.extend_from_slice(&msg.encode_to_vec());
        }
        buf.extend_from_slice(&self.server_timestamp_us.to_le_bytes());
        if self.has_more { buf.push(1); }
        crc32c::crc32c(&buf)
    }

    pub fn verify_crc(&self) -> Result<(), ContractError> {
        let computed = self.compute_crc();
        if computed != self.crc32 {
            return Err(ContractError::CrcMismatch {
                expected: self.crc32,
                actual: computed,
            });
        }
        Ok(())
    }

    pub fn finalize(&mut self) {
        self.crc32 = self.compute_crc();
    }
}