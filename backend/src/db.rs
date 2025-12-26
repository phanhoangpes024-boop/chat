use crate::contract::{ContractError, Message, MessageRow};
use crate::sync::MessageRepo;
use scylla::{Session, SessionBuilder};
use std::sync::Arc;

// ============================================================================
// SCYLLADB REPOSITORY
// ============================================================================

/// Production repository backed by ScyllaDB
pub struct ScyllaRepo {
    session: Arc<Session>,
}

impl ScyllaRepo {
    /// Create new ScyllaDB repository
    /// 
    /// COMPATIBILITY: Uses Scylla 0.12 API (SessionBuilder::new().known_node())
    pub async fn new(nodes: Vec<&str>) -> Result<Self, ContractError> {
        let mut builder = SessionBuilder::new();
        
        // Add all cluster nodes (Scylla 0.12 syntax)
        for node in nodes {
            builder = builder.known_node(node);
        }
        
        let session = builder
            .build()
            .await
            .map_err(|e| ContractError::DbError(format!("Connection failed: {}", e)))?;
        
        // Set keyspace
        session
            .query("USE turbochat", &[])
            .await
            .map_err(|e| ContractError::DbError(format!("Keyspace error: {}", e)))?;
        
        Ok(Self {
            session: Arc::new(session),
        })
    }
    
    /// Get session for custom queries (e.g., migrations)
    pub fn session(&self) -> Arc<Session> {
        Arc::clone(&self.session)
    }
}

#[axum::async_trait]
impl MessageRepo for ScyllaRepo {
    /// Fetch messages with cursor-based pagination (Read Path)
    async fn fetch_messages(
        &self,
        chat_id: u64,
        after_message_id: u64,
        limit: u32,
    ) -> Result<Vec<Message>, ContractError> {
        // TÍNH BUCKET ĐỘNG từ thời gian hiện tại
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        
        let bucket = compute_bucket(now_us);
        
        eprintln!("🔍 ScyllaDB Query:");
        eprintln!("   chat_id={}, bucket={}", chat_id, bucket);
        eprintln!("   after_message_id={}, limit={}", after_message_id, limit);
        
        let query = "
            SELECT chat_id, bucket, message_id, sender_id, content, timestamp_us, content_crc
            FROM messages
            WHERE chat_id = ? AND bucket = ? AND message_id > ?
            ORDER BY message_id ASC
            LIMIT ?
        ";
        
        let values = (chat_id as i64, bucket, after_message_id as i64, limit as i32);
        
        use scylla::query::Query;
        use scylla::statement::Consistency;
        
        let mut prepared_query = Query::new(query);
        prepared_query.set_consistency(Consistency::One);
        
        let rows = self
            .session
            .query(prepared_query, values)
            .await
            .map_err(|e| {
                eprintln!("❌ ScyllaDB query failed: {:?}", e);
                ContractError::DbError(format!("Query failed: {}", e))
            })?;
        
        let rows_typed = rows
            .rows
            .ok_or_else(|| {
                eprintln!("⚠️ No rows returned from ScyllaDB");
                ContractError::DbError("No rows returned".into())
            })?;
        
        eprintln!("✅ ScyllaDB returned {} rows", rows_typed.len());
        
        let mut messages = Vec::with_capacity(rows_typed.len());
        let mut skipped_count = 0;
        
        for row in rows_typed {
            let msg_row: MessageRow = match row.into_typed() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("⚠️ SKIP ROW: Parse error: {:?}", e);
                    skipped_count += 1;
                    continue;
                }
            };
            
            let msg_id = msg_row.message_id;
            
            match Message::try_from(msg_row) {
                Ok(message) => {
                    messages.push(message);
                }
                Err(e) => {
                    eprintln!("⚠️ SKIP CORRUPTED MESSAGE: message_id={}, Error={}", msg_id, e);
                    skipped_count += 1;
                    continue;
                }
            }
        }
        
        if skipped_count > 0 {
            eprintln!("⚠️ Skipped {} corrupted/invalid messages", skipped_count);
        }
        
        eprintln!("✅ Returning {} valid messages", messages.len());
        
        Ok(messages)
    }
    
    /// Insert a new message into ScyllaDB (Write Path)
    /// 
    /// SYSCALL BUDGET: 1x INSERT query
    /// 
    /// INTEGRITY GUARANTEE:
    /// - CRC MUST be verified by caller BEFORE calling this method
    async fn insert_message(&self, message: &Message) -> Result<(), ContractError> {
        // Convert to MessageRow for DB insertion
        let row = MessageRow::from(message);
        
        eprintln!("💾 Inserting message:");
        eprintln!("   chat_id={}, bucket={}", row.chat_id, row.bucket);
        eprintln!("   message_id={}, content_len={}", row.message_id, row.content.len());
        
        let query = "
            INSERT INTO messages (chat_id, bucket, message_id, sender_id, content, timestamp_us, content_crc)
            VALUES (?, ?, ?, ?, ?, ?, ?)
        ";
        
        let values = (
            row.chat_id,
            row.bucket,
            row.message_id,
            row.sender_id,
            row.content,
            row.timestamp_us,
            row.content_crc,
        );
        
        use scylla::query::Query;
        use scylla::statement::Consistency;
        
        let mut prepared_query = Query::new(query);
        prepared_query.set_consistency(Consistency::One);
        
        self.session
            .query(prepared_query, values)
            .await
            .map_err(|e| {
                eprintln!("❌ ScyllaDB insert failed: {:?}", e);
                ContractError::DbError(format!("Insert failed: {}", e))
            })?;
        
        eprintln!("✅ Message inserted successfully");
        Ok(())
    }
}

// ============================================================================
// HELPER FUNCTION
// ============================================================================

/// Compute bucket from timestamp (YYYYMMDD format)
fn compute_bucket(timestamp_us: u64) -> i32 {
    use chrono::{DateTime, Datelike, Utc};
    
    let secs = (timestamp_us / 1_000_000) as i64;
    let dt = DateTime::from_timestamp(secs, 0)
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap());
    
    let year = dt.year();
    let month = dt.month();
    let day = dt.day();
    
    year * 10000 + month as i32 * 100 + day as i32
}