use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::contract::{ContractError, Message, SyncRequest, SyncResponse, SendRequest, SendResponse};

// ============================================================================
// TRAIT: Repository Abstraction (For Testing)
// ============================================================================

/// Repository trait cho việc truy vấn messages
/// Cho phép mock trong test mà không cần DB thật
#[axum::async_trait]
pub trait MessageRepo: Send + Sync {
    async fn fetch_messages(
        &self,
        chat_id: u64,
        after_message_id: u64,
        limit: u32,
    ) -> Result<Vec<Message>, ContractError>;
    
    /// Insert a new message (Write Path)
    async fn insert_message(&self, message: &Message) -> Result<(), ContractError>;
}

// ============================================================================
// MOCK IMPLEMENTATION (For TDD)
// ============================================================================

/// Mock repository - lưu data trong RAM (HashMap)
pub struct MockRepo {
    messages: Arc<RwLock<Vec<Message>>>,
}

impl MockRepo {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Helper để seed test data
    pub async fn seed(&self, msgs: Vec<Message>) {
        let mut store = self.messages.write().await;
        store.extend(msgs);
        store.sort_by_key(|m| m.message_id);
    }
}

#[axum::async_trait]
impl MessageRepo for MockRepo {
    async fn fetch_messages(
        &self,
        chat_id: u64,
        after_message_id: u64,
        limit: u32,
    ) -> Result<Vec<Message>, ContractError> {
        let store = self.messages.read().await;
        
        // Filter by chat_id, message_id > cursor, limit
        let filtered: Vec<Message> = store
            .iter()
            .filter(|m| m.chat_id == chat_id && m.message_id > after_message_id)
            .take(limit as usize)
            .cloned()
            .collect();

        Ok(filtered)
    }
    
    async fn insert_message(&self, message: &Message) -> Result<(), ContractError> {
        let mut store = self.messages.write().await;
        store.push(message.clone());
        store.sort_by_key(|m| m.message_id);
        Ok(())
    }
}

// ============================================================================
// AXUM HANDLER
// ============================================================================

/// Shared app state
pub struct AppState<R: MessageRepo> {
    pub repo: R,
}

/// POST /sync handler (Read Path)
pub async fn sync_handler<R: MessageRepo + 'static>(
    State(state): State<Arc<AppState<R>>>,
    body: Bytes,
) -> Result<Response, AppError> {
    eprintln!("\n========================================");
    eprintln!("📦 NEW SYNC REQUEST");
    eprintln!("========================================");
    eprintln!("   Received {} bytes", body.len());
    
    // STEP 0: Decode Protobuf request
    use prost::Message as ProstMessage;
    let req = SyncRequest::decode(&body[..])
        .map_err(|e| {
            eprintln!("❌ Protobuf decode failed: {:?}", e);
            eprintln!("   Raw bytes: {:?}", &body[..body.len().min(50)]);
            AppError(ContractError::DecodeError(e))
        })?;

    eprintln!("✅ Protobuf decoded successfully:");
    eprintln!("   chat_id: {}", req.chat_id);
    eprintln!("   after_message_id: {}", req.after_message_id);
    eprintln!("   limit: {}", req.limit);

    // STEP 1: Query database
    let messages = state
        .repo
        .fetch_messages(req.chat_id, req.after_message_id, req.limit)
        .await?;

    eprintln!("✅ Repository returned {} messages", messages.len());

    // STEP 2: Build response
    let has_more = messages.len() == req.limit as usize;
    let server_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;

    let mut response = SyncResponse {
        messages,
        server_timestamp_us: server_ts,
        has_more,
        crc32: 0,
    };

    // STEP 3: Finalize CRC
    response.finalize();
    eprintln!("✅ Response CRC computed: 0x{:08x}", response.crc32);

    // STEP 4: Serialize to Protobuf
    let encoded = response.encode_to_vec();
    let body = Bytes::from(encoded);
    
    eprintln!("✅ Sending {} bytes to client", body.len());
    eprintln!("========================================\n");

    Ok((StatusCode::OK, body).into_response())
}

// ============================================================================
// SEND HANDLER (Write Path)
// ============================================================================

/// POST /send handler - The Gatekeeper
/// 
/// INTEGRITY FIRST:
/// - Verifies CRC32 BEFORE any database operation
/// - Rejects corrupted data at the gate (400 Bad Request)
/// - Only chemically pure data enters the system
/// 
/// SYSCALL BUDGET:
/// - 1x DB INSERT query
/// Total: 1 syscall (within budget)
pub async fn send_handler<R: MessageRepo + 'static>(
    State(state): State<Arc<AppState<R>>>,
    body: Bytes,
) -> Result<Response, AppError> {
    eprintln!("\n========================================");
    eprintln!("📨 NEW SEND REQUEST");
    eprintln!("========================================");
    eprintln!("   Received {} bytes", body.len());
    
    // STEP 0: Decode Protobuf request
    use prost::Message as ProstMessage;
    let req = SendRequest::decode(&body[..])
        .map_err(|e| {
            eprintln!("❌ Protobuf decode failed: {:?}", e);
            AppError(ContractError::DecodeError(e))
        })?;

    eprintln!("✅ SendRequest decoded:");
    eprintln!("   chat_id: {}", req.chat_id);
    eprintln!("   sender_id: {}", req.sender_id);
    eprintln!("   content_len: {}", req.content.len());
    eprintln!("   client_crc: 0x{:08x}", req.content_crc);

    // STEP 1: THE GATEKEEPER - Verify CRC32 FIRST
    let calculated_crc = crc32c::crc32c(&req.content);
    
    if calculated_crc != req.content_crc {
        eprintln!("🔴 CRC MISMATCH DETECTED!");
        eprintln!("   Expected: 0x{:08x}", req.content_crc);
        eprintln!("   Calculated: 0x{:08x}", calculated_crc);
        eprintln!("   Rejecting request (potential attack or corruption)");
        
        let error_response = SendResponse {
            success: false,
            message_id: 0,
            timestamp_us: 0,
            error: format!(
                "CRC mismatch: expected {:08x}, got {:08x}",
                req.content_crc, calculated_crc
            ),
        };
        
        let encoded = error_response.encode_to_vec();
        return Ok((StatusCode::BAD_REQUEST, Bytes::from(encoded)).into_response());
    }
    
    eprintln!("✅ CRC verification passed");

    // STEP 2: Generate message_id and timestamp
    let timestamp_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;
    
    // NOTE: Simple timestamp as message_id (NOT production-safe for distributed systems)
    // TODO: Replace with Snowflake/Sonyflake for collision-free IDs
    let message_id = timestamp_us;
    
    eprintln!("   Generated message_id: {}", message_id);
    eprintln!("   timestamp_us: {}", timestamp_us);

    // STEP 3: Create Message struct
    let message = Message {
        message_id,
        chat_id: req.chat_id,
        sender_id: req.sender_id,
        content: req.content, // Zero-copy (Bytes)
        timestamp_us,
        content_crc: req.content_crc,
    };

    // STEP 4: Insert into database
    match state.repo.insert_message(&message).await {
        Ok(_) => {
            eprintln!("✅ Message inserted successfully");
            
            let response = SendResponse {
                success: true,
                message_id,
                timestamp_us,
                error: String::new(),
            };
            
            let encoded = response.encode_to_vec();
            eprintln!("========================================\n");
            Ok((StatusCode::OK, Bytes::from(encoded)).into_response())
        }
        Err(e) => {
            eprintln!("❌ Database insert failed: {:?}", e);
            
            let error_response = SendResponse {
                success: false,
                message_id: 0,
                timestamp_us: 0,
                error: format!("Database error: {}", e),
            };
            
            let encoded = error_response.encode_to_vec();
            Ok((StatusCode::INTERNAL_SERVER_ERROR, Bytes::from(encoded)).into_response())
        }
    }
}

// ============================================================================
// ERROR HANDLING (No Panics)
// ============================================================================

pub struct AppError(ContractError);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let msg = format!("Error: {}", self.0);
        eprintln!("❌ HANDLER ERROR: {}", msg);
        (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
    }
}

impl From<ContractError> for AppError {
    fn from(err: ContractError) -> Self {
        AppError(err)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use prost::Message as ProstMessage;

    #[tokio::test]
    async fn test_sync_returns_updates_after_cursor() {
        let repo = MockRepo::new();
        
        repo.seed(vec![
            Message::new(101, 100, 42, Bytes::from_static(b"Msg101"), 1000000),
            Message::new(102, 100, 42, Bytes::from_static(b"Msg102"), 2000000),
            Message::new(103, 100, 43, Bytes::from_static(b"Msg103"), 3000000),
        ]).await;

        let app_state = Arc::new(AppState { repo });
        let app = axum::Router::new()
            .route("/sync", axum::routing::post(sync_handler))
            .with_state(app_state);

        let request = SyncRequest {
            chat_id: 100,
            after_message_id: 100,
            limit: 10,
        };

        let req_bytes = request.encode_to_vec();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sync")
                    .header("content-type", "application/protobuf")
                    .body(Body::from(req_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body();
        let body_bytes = body.collect().await.unwrap().to_bytes();

        let sync_resp = SyncResponse::decode(&body_bytes[..]).unwrap();

        assert_eq!(sync_resp.messages.len(), 3);
        assert_eq!(sync_resp.messages[0].message_id, 101);
        assert!(sync_resp.verify_crc().is_ok());
    }
    
    #[tokio::test]
    async fn test_send_rejects_bad_crc() {
        let repo = MockRepo::new();
        let app_state = Arc::new(AppState { repo });
        let app = axum::Router::new()
            .route("/send", axum::routing::post(send_handler))
            .with_state(app_state);

        let content = Bytes::from_static(b"Test message");
        let bad_crc = 0xDEADBEEF; // Wrong CRC

        let request = SendRequest {
            chat_id: 100,
            sender_id: 42,
            content,
            content_crc: bad_crc,
        };

        let req_bytes = request.encode_to_vec();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/send")
                    .body(Body::from(req_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should reject with 400 Bad Request
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}