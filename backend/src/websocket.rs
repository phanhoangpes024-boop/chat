use axum::{
    extract::{ws::{Message as WsMessage, WebSocket, WebSocketUpgrade}, State, Query},
    response::IntoResponse,
};
use futures::{sink::SinkExt, stream::StreamExt};
use redis::AsyncCommands;
use std::sync::Arc;
use tokio::sync::broadcast;
use prost::Message as ProstMessage;
use serde::Deserialize;

use crate::contract::Message as ChatMessage;
use crate::db::AstraRepo;

#[derive(Deserialize)]
pub struct WsQuery {
    pub shop_id: String,
    pub guest_id: Option<u64>,  // None = admin, Some = guest
}

pub struct WebSocketState {
    pub redis_url: String,
    pub tx: broadcast::Sender<Vec<u8>>,
    pub repo: Arc<AstraRepo>,
}

impl WebSocketState {
    pub async fn new(redis_url: &str, repo: Arc<AstraRepo>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (tx, _) = broadcast::channel(1000);
        Ok(Self { redis_url: redis_url.to_string(), tx, repo })
    }
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<Arc<WebSocketState>>,
) -> impl IntoResponse {
    println!("🔌 WebSocket upgrade request: shop={}, guest={:?}", query.shop_id, query.guest_id);
    ws.on_upgrade(move |socket| handle_socket(socket, state, query))
}

async fn handle_socket(socket: WebSocket, state: Arc<WebSocketState>, query: WsQuery) {
    let (mut sender, mut receiver) = socket.split();
    let shop_id = query.shop_id.clone();
    let guest_id = query.guest_id;
    
    println!("✅ WebSocket connected: shop={}, guest={:?}", shop_id, guest_id);
    
    // Subscribe Redis channel cho shop này
    let mut rx = state.tx.subscribe();
    
    // Task gửi tin từ Redis → Client
    let shop_filter = shop_id.clone();
    let mut send_task = tokio::spawn(async move {
        while let Ok(bytes) = rx.recv().await {
            if let Ok(msg) = ChatMessage::decode(&bytes[..]) {
                if msg.shop_id == shop_filter {
                    // Guest chỉ nhận tin của mình, Admin nhận tất cả
                    if guest_id.is_none() || guest_id == Some(msg.guest_id) {
                        println!("📤 Forwarding to client: {} bytes", bytes.len());
                        if sender.send(WsMessage::Binary(bytes)).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });

    // Task nhận tin từ Client → Redis
    let state_clone = Arc::clone(&state);
    let shop_id_clone = shop_id.clone();
    let mut recv_task = tokio::spawn(async move {
        println!("👂 Listening for messages from client...");
        while let Some(Ok(msg)) = receiver.next().await {
            if let WsMessage::Binary(data) = msg {
                println!("📩 Received WebSocket message: {:?}", "Binary");
                println!("📦 Binary data: {} bytes", data.len());
                
                if let Ok(mut chat_msg) = ChatMessage::decode(&data[..]) {
                    chat_msg.shop_id = shop_id_clone.clone();
                    println!("💬 Message decoded: shop={}, guest={}, sender={}, content={:?}",
                        chat_msg.shop_id, chat_msg.guest_id, chat_msg.sender_type,
                        String::from_utf8_lossy(&chat_msg.content));
                    
                    // Tạo/cập nhật guest nếu là guest
                    if chat_msg.sender_type == "guest" {
                        let name = format!("Guest #{}", chat_msg.guest_id % 10000);
                        let _ = state_clone.repo.upsert_guest(&chat_msg.shop_id, chat_msg.guest_id, &name).await;
                    }
                    
                    // Lưu DB
                    if let Err(e) = state_clone.repo.insert_message(&chat_msg).await {
                        eprintln!("❌ DB insert failed: {:?}", e);
                        continue;
                    }
                    println!("✅ Message saved to DB");
                    
                    // Publish Redis
                    if let Err(e) = publish_to_redis(&state_clone, &chat_msg).await {
                        eprintln!("❌ Redis publish failed: {:?}", e);
                    }
                }
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    }
    
    println!("🔌 WebSocket disconnected: shop={}", shop_id);
}

async fn publish_to_redis(state: &Arc<WebSocketState>, msg: &ChatMessage) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = redis::Client::open(state.redis_url.as_str())?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let channel = format!("chat:{}", msg.shop_id);
    let payload = msg.encode_to_vec();
    println!("📡 Publishing to channel: {}", channel);
    conn.publish::<_, _, ()>(&channel, &payload).await?;
    println!("✅ Published to Redis");
    Ok(())
}

pub async fn redis_subscriber_task(state: Arc<WebSocketState>) {
    println!("🔔 Redis subscriber starting...");
    
    loop {
        match run_redis_subscriber(&state).await {
            Ok(_) => {
                println!("⚠️ Redis stream ended, reconnecting...");
            }
            Err(e) => {
                eprintln!("❌ Redis pubsub error: {:?}", e);
            }
        }
        // Đợi 2 giây trước khi reconnect
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }
}

async fn run_redis_subscriber(state: &Arc<WebSocketState>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = redis::Client::open(state.redis_url.as_str())?;
    
    // Dùng get_async_pubsub() cho redis 0.27+
    let mut pubsub = client.get_async_pubsub().await?;
    
    // Subscribe pattern chat:*
    pubsub.psubscribe("chat:*").await?;
    println!("✅ Redis subscribed to chat:*");
    
    // Lấy stream từ pubsub
    let mut stream = pubsub.on_message();
    
    while let Some(msg) = stream.next().await {
        let channel: String = msg.get_channel()?;
        let payload: Vec<u8> = msg.get_payload()?;
        
        println!("📬 Redis received on {}: {} bytes", channel, payload.len());
        
        let receiver_count = state.tx.receiver_count();
        println!("📢 Broadcast to {} receivers", receiver_count);
        
        let _ = state.tx.send(payload);
    }
    
    Ok(())
}