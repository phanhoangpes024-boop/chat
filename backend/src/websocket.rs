use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures::{sink::SinkExt, stream::StreamExt};
use redis::AsyncCommands;
use std::sync::Arc;
use tokio::sync::broadcast;
use prost::Message as ProstMessage;

use crate::contract::Message as ChatMessage;

// ============================================================================
// WEBSOCKET STATE
// ============================================================================

pub struct WebSocketState {
    /// Redis connection URL
    pub redis_url: String,
    
    /// Broadcast channel (in-memory per instance)
    pub tx: broadcast::Sender<Vec<u8>>, // Protobuf bytes
}

impl WebSocketState {
    pub async fn new(redis_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, _rx) = broadcast::channel(1000);
        
        Ok(Self {
            redis_url: redis_url.to_string(),
            tx,
        })
    }
}

// ============================================================================
// WEBSOCKET UPGRADE HANDLER
// ============================================================================

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<WebSocketState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

// ============================================================================
// WEBSOCKET CONNECTION HANDLER
// ============================================================================

async fn handle_socket(socket: WebSocket, state: Arc<WebSocketState>) {
    let (mut sender, mut receiver) = socket.split();
    
    // Subscribe to broadcast channel
    let mut rx = state.tx.subscribe();
    
    // Task 1: Broadcast → Client
    let mut send_task = tokio::spawn(async move {
        while let Ok(bytes) = rx.recv().await {
            // Gửi Protobuf bytes xuống client
            if sender.send(WsMessage::Binary(bytes)).await.is_err() {
                break;
            }
        }
    });
    
    // Task 2: Client → Redis Pub
    let state_clone = Arc::clone(&state);
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                WsMessage::Binary(data) => {
                    // Client gửi Protobuf message
                    if let Ok(chat_msg) = ChatMessage::decode(&data[..]) {
                        // Publish to Redis
                        let _ = publish_to_redis(&state_clone, &chat_msg).await;
                    }
                }
                WsMessage::Close(_) => break,
                _ => {}
            }
        }
    });
    
    // Đợi một task kết thúc
    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    }
}

// ============================================================================
// REDIS PUB/SUB
// ============================================================================

async fn publish_to_redis(state: &Arc<WebSocketState>, msg: &ChatMessage) -> Result<(), Box<dyn std::error::Error>> {
    let client = redis::Client::open(state.redis_url.as_str())?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    
    let channel = format!("chat:{}", msg.chat_id);
    let payload = msg.encode_to_vec();
    
    conn.publish::<_, _, ()>(&channel, &payload).await?;
    
    Ok(())
}

// backend/src/websocket.rs

/// Background task: Subscribe Redis → Broadcast local
pub async fn redis_subscriber_task(state: Arc<WebSocketState>) {
    let client = match redis::Client::open(state.redis_url.as_str()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("❌ Redis client failed: {}", e);
            return;
        }
    };

    // SỬA 1: Lấy connection trước, sau đó gọi into_pubsub()
    // Thay vì client.get_async_pubsub()
    let mut pubsub = match client.get_async_connection().await {
        Ok(conn) => conn.into_pubsub(), 
        Err(e) => {
            eprintln!("❌ Redis connection failed: {}", e);
            return;
        }
    };

    if let Err(e) = pubsub.psubscribe("chat:*").await {
        eprintln!("❌ Redis psubscribe failed: {}", e);
        return;
    }

    eprintln!("✅ Redis subscriber started");

    let mut stream = pubsub.on_message();

    while let Some(msg) = stream.next().await {
        // SỬA 2: Chỉ định rõ kiểu dữ liệu mong muốn là Vec<u8> (bytes)
        // Cú pháp turbofish ::<Vec<u8>> giúp Rust hiểu kiểu dữ liệu
        let payload: Vec<u8> = match msg.get_payload::<Vec<u8>>() { 
            Ok(p) => p,
            Err(_) => continue,
        };

        // Broadcast to all local WebSocket clients
        let _ = state.tx.send(payload);
    }
}