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
use crate::sync::MessageRepo;

pub struct WebSocketState {
    pub redis_url: String,
    pub tx: broadcast::Sender<Vec<u8>>,
    pub repo: Arc<dyn MessageRepo>,
}

impl WebSocketState {
    pub async fn new(
        redis_url: &str,
        repo: Arc<dyn MessageRepo>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, _rx) = broadcast::channel(1000);
        
        Ok(Self {
            redis_url: redis_url.to_string(),
            tx,
            repo,
        })
    }
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<WebSocketState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<WebSocketState>) {
    let (mut sender, mut receiver) = socket.split();
    
    let mut rx = state.tx.subscribe();
    
    let mut send_task = tokio::spawn(async move {
        while let Ok(bytes) = rx.recv().await {
            eprintln!("📤 Sending {} bytes to client", bytes.len());
            if sender.send(WsMessage::Binary(bytes)).await.is_err() {
                eprintln!("❌ Failed to send to client");
                break;
            }
            eprintln!("✅ Sent to client successfully");
        }
    });
    
    let state_clone = Arc::clone(&state);
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                WsMessage::Binary(data) => {
                    if let Ok(chat_msg) = ChatMessage::decode(&data[..]) {
                        eprintln!("📥 WebSocket received message: id={}", chat_msg.message_id);
                        
                        if let Err(e) = state_clone.repo.insert_message(&chat_msg).await {
                            eprintln!("❌ DB insert failed: {:?}", e);
                            continue;
                        }
                        eprintln!("✅ Message saved to DB");
                        
                        if let Err(e) = publish_to_redis(&state_clone, &chat_msg).await {
                            eprintln!("❌ Redis publish failed: {:?}", e);
                        } else {
                            eprintln!("✅ Published to Redis");
                        }
                    }
                }
                WsMessage::Close(_) => break,
                _ => {}
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
    
    let channel = format!("chat:{}", msg.chat_id);
    let payload = msg.encode_to_vec();
    
    eprintln!("📮 Publishing to Redis channel '{}' ({} bytes)", channel, payload.len());
    
    conn.publish::<_, _, ()>(&channel, &payload).await?;
    
    Ok(())
}

pub async fn redis_subscriber_task(state: Arc<WebSocketState>) {
    eprintln!("🔄 Starting Redis subscriber task...");
    
    let client = match redis::Client::open(state.redis_url.as_str()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("❌ Redis client failed: {}", e);
            return;
        }
    };

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

    eprintln!("✅ Redis subscriber started - listening on 'chat:*'");

    let mut stream = pubsub.on_message();

    while let Some(msg) = stream.next().await {
        let payload: Vec<u8> = match msg.get_payload::<Vec<u8>>() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("⚠️ Failed to get payload: {:?}", e);
                continue;
            }
        };

        eprintln!("📢 Redis → Broadcasting {} bytes to clients", payload.len());
        
        match state.tx.send(payload) {
            Ok(count) => eprintln!("✅ Broadcasted to {} connected clients", count),
            Err(e) => eprintln!("❌ Broadcast failed: {:?}", e),
        }
    }
    
    eprintln!("⚠️ Redis subscriber stream ended");
}