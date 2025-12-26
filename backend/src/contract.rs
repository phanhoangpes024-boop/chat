// Re-export từ shared
pub use turbochat_shared::{Message, SyncRequest, SyncResponse, SendRequest, SendResponse, User, ContractError};

use bytes::Bytes;
use scylla::FromRow;
use std::convert::TryFrom;
use chrono::{DateTime, Datelike, Utc};

// ============================================================================
// SCYLLADB ROW MAPPING
// ============================================================================

#[derive(FromRow, Debug, Clone)]
pub struct MessageRow {
    pub chat_id: i64,
    pub bucket: i32,
    pub message_id: i64,
    pub sender_id: i64,
    pub content: Vec<u8>,
    pub timestamp_us: i64,
    pub content_crc: i32,
}

// Row → Message (với CRC check)
impl TryFrom<MessageRow> for Message {
    type Error = ContractError;

    fn try_from(row: MessageRow) -> Result<Self, Self::Error> {
        let message_id = row.message_id;
        let content = Bytes::from(row.content);
        let computed_crc = crc32c::crc32c(&content);
        let stored_crc = row.content_crc as u32;

        if computed_crc != stored_crc {
            eprintln!("🔴 CRC FAIL: msg={}, expected={:08x}, got={:08x}", 
                message_id, stored_crc, computed_crc);
            return Err(ContractError::CrcMismatch {
                expected: stored_crc,
                actual: computed_crc,
            });
        }

        Ok(Message {
            message_id: row.message_id as u64,
            chat_id: row.chat_id as u64,
            sender_id: row.sender_id as u64,
            content,
            timestamp_us: row.timestamp_us as u64,
            content_crc: stored_crc,
        })
    }
}

// Message → Row (để insert vào DB)
impl From<&Message> for MessageRow {
    fn from(msg: &Message) -> Self {
        Self {
            chat_id: msg.chat_id as i64,
            bucket: compute_bucket(msg.timestamp_us),
            message_id: msg.message_id as i64,
            sender_id: msg.sender_id as i64,
            content: msg.content.to_vec(),
            timestamp_us: msg.timestamp_us as i64,
            content_crc: msg.content_crc as i32,
        }
    }
}

// ============================================================================
// HELPER
// ============================================================================

fn compute_bucket(timestamp_us: u64) -> i32 {
    let secs = (timestamp_us / 1_000_000) as i64;
    let dt = DateTime::from_timestamp(secs, 0)
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap());
    
    let year = dt.year();
    let month = dt.month();
    let day = dt.day();
    
    year * 10000 + month as i32 * 100 + day as i32
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bucket_leap_year() {
        let ts = 1709164800_000000; // Feb 29, 2024
        assert_eq!(compute_bucket(ts), 20240229);
    }

    #[test]
    fn test_roundtrip() {
        let msg = Message::new(1, 100, 42, Bytes::from_static(b"test"), 1703462400_000000);
        let row = MessageRow::from(&msg);
        let recovered = Message::try_from(row).unwrap();
        assert_eq!(recovered.message_id, 1);
    }

    #[test]
    fn test_crc_reject() {
        let row = MessageRow {
            chat_id: 1, bucket: 20241224, message_id: 1,
            sender_id: 1, content: vec![1, 2, 3],
            timestamp_us: 0, content_crc: 0xDEADBEEF_u32 as i32,
        };
        assert!(Message::try_from(row).is_err());
    }
}