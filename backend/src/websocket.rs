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
    pub async fn new(redis_url: &str, repo: Arc<AstraRepo>) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, _) = broadcast::channel(1000);
        Ok(Self { redis_url: redis_url.to_string(), tx, repo })
    }
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<Arc<WebSocketState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, query))
}

async fn handle_socket(socket: WebSocket, state: Arc<WebSocketState>, query: WsQuery) {
    let (mut sender, mut receiver) = socket.split();
    let shop_id = query.shop_id.clone();
    let guest_id = query.guest_id;
    
    // Subscribe Redis channel cho shop này
    let _channel = format!("chat:{}", shop_id);
    let mut rx = state.tx.subscribe();
    
    // Task gửi tin từ Redis → Client
    let shop_filter = shop_id.clone();
    let mut send_task = tokio::spawn(async move {
        while let Ok(bytes) = rx.recv().await {
            if let Ok(msg) = ChatMessage::decode(&bytes[..]) {
                if msg.shop_id == shop_filter {
                    // Guest chỉ nhận tin của mình, Admin nhận tất cả
                    if guest_id.is_none() || guest_id == Some(msg.guest_id) {
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
        while let Some(Ok(msg)) = receiver.next().await {
            if let WsMessage::Binary(data) = msg {
                if let Ok(mut chat_msg) = ChatMessage::decode(&data[..]) {
                    chat_msg.shop_id = shop_id_clone.clone();
                    
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
                    
                    // Publish Redis
                    let _ = publish_to_redis(&state_clone, &chat_msg).await;
                }
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    }
}

async fn publish_to_redis(state: &Arc<WebSocketState>, msg: &ChatMessage) -> Result<(), Box<dyn std::error::Error>> {
    let client = redis::Client::open(state.redis_url.as_str())?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let channel = format!("chat:{}", msg.shop_id);
    let payload = msg.encode_to_vec();
    conn.publish::<_, _, ()>(&channel, &payload).await?;
    Ok(())
}

pub async fn redis_subscriber_task(state: Arc<WebSocketState>) {
    let client = match redis::Client::open(state.redis_url.as_str()) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut pubsub = match client.get_async_connection().await {
        Ok(conn) => conn.into_pubsub(),
        Err(_) => return,
    };

    let _ = pubsub.psubscribe("chat:*").await;
    let mut stream = pubsub.on_message();

    while let Some(msg) = stream.next().await {
        if let Ok(payload) = msg.get_payload::<Vec<u8>>() {
            let _ = state.tx.send(payload);
        }
    }
}