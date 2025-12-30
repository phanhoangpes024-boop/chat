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
    pub guest_id: Option<u64>,
}

pub struct WebSocketState {
    pub redis_client: redis::Client,
    pub tx: broadcast::Sender<Vec<u8>>,
    pub repo: Arc<AstraRepo>,
}

impl WebSocketState {
    pub async fn new(redis_url: &str, repo: Arc<AstraRepo>) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, _) = broadcast::channel(1000);
        let redis_client = redis::Client::open(redis_url)?;
        Ok(Self { redis_client, tx, repo })
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
    
    let mut rx = state.tx.subscribe();
    
    // Task gửi tin từ broadcast → Client
    let shop_filter = shop_id.clone();
    let guest_filter = guest_id;
    let mut send_task = tokio::spawn(async move {
        while let Ok(bytes) = rx.recv().await {
            if let Ok(msg) = ChatMessage::decode(&bytes[..]) {
                if msg.shop_id == shop_filter {
                    // Guest chỉ nhận tin của mình, Admin nhận tất cả
                    if guest_filter.is_none() || guest_filter == Some(msg.guest_id) {
                        println!("📤 Forwarding to client: {} bytes", bytes.len());
                        if sender.send(WsMessage::Binary(bytes.into())).await.is_err() {
                            println!("❌ Failed to send to client");
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
        
        while let Some(result) = receiver.next().await {
            match result {
                Ok(msg) => {
                    println!("📩 Received WebSocket message: {:?}", msg_type(&msg));
                    
                    match msg {
                        WsMessage::Binary(data) => {
                            println!("📦 Binary data: {} bytes", data.len());
                            
                            match ChatMessage::decode(&data[..]) {
                                Ok(mut chat_msg) => {
                                    chat_msg.shop_id = shop_id_clone.clone();
                                    println!("💬 Message decoded: shop={}, guest={}, sender={}, content={:?}", 
                                        chat_msg.shop_id, 
                                        chat_msg.guest_id, 
                                        chat_msg.sender_type,
                                        String::from_utf8_lossy(&chat_msg.content)
                                    );
                                    
                                    // Upsert guest
                                    if chat_msg.sender_type == "guest" {
                                        let name = format!("Guest #{}", chat_msg.guest_id % 10000);
                                        if let Err(e) = state_clone.repo.upsert_guest(&chat_msg.shop_id, chat_msg.guest_id, &name).await {
                                            println!("⚠️ Upsert guest failed: {:?}", e);
                                        }
                                    }
                                    
                                    // Lưu DB
                                    match state_clone.repo.insert_message(&chat_msg).await {
                                        Ok(_) => println!("✅ Message saved to DB"),
                                        Err(e) => {
                                            println!("❌ DB insert failed: {:?}", e);
                                            continue;
                                        }
                                    }
                                    
                                    // Publish Redis
                                    match publish_to_redis(&state_clone, &chat_msg).await {
                                        Ok(_) => println!("✅ Published to Redis"),
                                        Err(e) => println!("❌ Redis publish failed: {:?}", e),
                                    }
                                }
                                Err(e) => {
                                    println!("❌ Protobuf decode failed: {:?}", e);
                                }
                            }
                        }
                        WsMessage::Text(text) => {
                            println!("📝 Text message (unexpected): {}", text);
                        }
                        WsMessage::Ping(_) => println!("🏓 Ping"),
                        WsMessage::Pong(_) => println!("🏓 Pong"),
                        WsMessage::Close(_) => {
                            println!("👋 Close frame received");
                            break;
                        }
                    }
                }
                Err(e) => {
                    println!("❌ WebSocket error: {:?}", e);
                    break;
                }
            }
        }
        println!("🔴 Client disconnected");
    });

    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    }
    
    println!("🔌 WebSocket handler finished: shop={}", shop_id);
}

fn msg_type(msg: &WsMessage) -> &'static str {
    match msg {
        WsMessage::Binary(_) => "Binary",
        WsMessage::Text(_) => "Text",
        WsMessage::Ping(_) => "Ping",
        WsMessage::Pong(_) => "Pong",
        WsMessage::Close(_) => "Close",
    }
}

async fn publish_to_redis(state: &Arc<WebSocketState>, msg: &ChatMessage) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut conn = state.redis_client.get_multiplexed_async_connection().await?;
    let channel = format!("chat:{}", msg.shop_id);
    let payload = msg.encode_to_vec();
    println!("📡 Publishing to channel: {}", channel);
    conn.publish::<_, _, ()>(&channel, &payload).await?;
    Ok(())
}

pub async fn redis_subscriber_task(state: Arc<WebSocketState>) {
    println!("🔔 Redis subscriber starting...");
    
    loop {
        match state.redis_client.get_async_pubsub().await {
            Ok(mut pubsub) => {
                if let Err(e) = pubsub.psubscribe("chat:*").await {
                    println!("❌ Redis psubscribe error: {:?}", e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    continue;
                }
                
                println!("✅ Redis subscribed to chat:*");
                
                let mut stream = pubsub.on_message();
                
                loop {
                    match stream.next().await {
                        Some(msg) => {
                            let channel: String = msg.get_channel_name().to_string();
                            if let Ok(payload) = msg.get_payload::<Vec<u8>>() {
                                println!("📬 Redis received on {}: {} bytes", channel, payload.len());
                                match state.tx.send(payload) {
                                    Ok(n) => println!("📢 Broadcast to {} receivers", n),
                                    Err(_) => println!("⚠️ No receivers"),
                                }
                            }
                        }
                        None => {
                            println!("⚠️ Redis stream ended, reconnecting...");
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                println!("❌ Redis pubsub error: {:?}", e);
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        }
    }
}