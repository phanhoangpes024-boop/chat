// Re-export generated protobuf code
mod proto {
    include!(concat!(env!("OUT_DIR"), "/turbochat.v1.rs"));
}

pub use proto::{Message, SyncRequest, SyncResponse, SendRequest, SendResponse, User};

use bytes::Bytes;
use thiserror::Error;

// ============================================================================
// ERROR TYPES
// ============================================================================

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("CRC mismatch: expected {expected:08x}, got {actual:08x}")]
    CrcMismatch { expected: u32, actual: u32 },

    #[error("Protobuf decode error: {0}")]
    DecodeError(#[from] prost::DecodeError),

    #[error("Database error: {0}")]
    DbError(String),
}

// ============================================================================
// MESSAGE HELPERS (Dùng cả Backend + Frontend)
// ============================================================================

impl Message {
    /// Tạo tin nhắn mới với CRC tự động
    pub fn new(
        message_id: u64,
        chat_id: u64,
        sender_id: u64,
        content: Bytes,
        timestamp_us: u64,
    ) -> Self {
        let content_crc = crc32c::crc32c(&content);
        Self {
            message_id,
            chat_id,
            sender_id,
            content,
            timestamp_us,
            content_crc,
        }
    }

    /// Kiểm tra tính toàn vẹn của content
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

// ============================================================================
// SYNC RESPONSE INTEGRITY
// ============================================================================

impl SyncResponse {
    /// Tính CRC cho toàn bộ response (không clone!)
    pub fn compute_crc(&self) -> u32 {
        use prost::encoding::{encode_key, encode_varint, WireType};
        use prost::Message as ProstMessage;
        
        let mut buf = Vec::with_capacity(self.messages.len() * 128 + 32);
        
        // Encode từng message
        for msg in &self.messages {
            encode_key(1, WireType::LengthDelimited, &mut buf);
            let msg_bytes = msg.encode_to_vec();
            encode_varint(msg_bytes.len() as u64, &mut buf);
            buf.extend_from_slice(&msg_bytes);
        }
        
        // Encode timestamp
        if self.server_timestamp_us != 0 {
            encode_key(2, WireType::SixtyFourBit, &mut buf);
            buf.extend_from_slice(&self.server_timestamp_us.to_le_bytes());
        }
        
        // Encode has_more
        if self.has_more {
            encode_key(3, WireType::Varint, &mut buf);
            encode_varint(1, &mut buf);
        }
        
        crc32c::crc32c(&buf)
    }

    /// Kiểm tra CRC của response nhận được
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

    /// Gắn CRC trước khi gửi (server-side)
    pub fn finalize(&mut self) {
        self.crc32 = self.compute_crc();
    }
}