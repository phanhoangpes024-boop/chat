mod contract;
mod db;
mod websocket;

use axum::{Router, routing::{get, post}, extract::State, body::Bytes, http::StatusCode, response::IntoResponse};
use std::sync::Arc;
use tower_http::cors::{CorsLayer, Any};
use prost::Message as ProstMessage;

use contract::*;
use db::AstraRepo;

struct AppState {
    repo: Arc<AstraRepo>,
    ws_state: Arc<websocket::WebSocketState>,
}

#[tokio::main]
async fn main() {
    println!("🚀 TurboChat Backend Starting...");
    dotenvy::dotenv().ok();
    
    // Connect ScyllaDB
    let repo = Arc::new(AstraRepo::new().await
    .expect("❌ AstraDB connection failed"));
println!("✅ AstraDB connected");
    
    // Setup WebSocket + Redis
    let redis_url = std::env::var("REDIS_URL").expect("❌ REDIS_URL not found in .env");
let ws_state = Arc::new(websocket::WebSocketState::new(&redis_url, repo.clone()).await
        .expect("❌ Redis connection failed"));
    println!("✅ Redis connected");
    
    // Redis subscriber task
    let ws_clone = Arc::clone(&ws_state);
    tokio::spawn(async move { websocket::redis_subscriber_task(ws_clone).await });
    
    let state = Arc::new(AppState { repo, ws_state: ws_state.clone() });
    
    // CORS - cho phép mọi nguồn
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    
    let app = Router::new()
        .route("/ws", get(websocket::ws_handler))
        .with_state(ws_state)
        .route("/auth", post(auth_handler))
        .route("/guests", post(guests_handler))
        .route("/sync", post(sync_handler))
        .with_state(state)
        .layer(cors);
    
    println!("🌐 Server: http://localhost:8080");
    println!("📡 WebSocket: ws://localhost:8080/ws?shop_id=demo123");
    
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// POST /auth - Xác thực admin
async fn auth_handler(State(state): State<Arc<AppState>>, body: Bytes) -> impl IntoResponse {
    let req = match AdminAuthRequest::decode(&body[..]) {
        Ok(r) => r,
        Err(_) => return (StatusCode::BAD_REQUEST, Bytes::new()),
    };
    
    let resp = match state.repo.verify_admin(&req.shop_id, &req.admin_pin).await {
        Ok(Some(name)) => AdminAuthResponse { success: true, shop_name: name, error: String::new() },
        Ok(None) => AdminAuthResponse { success: false, shop_name: String::new(), error: "Invalid PIN".into() },
        Err(e) => AdminAuthResponse { success: false, shop_name: String::new(), error: e.to_string() },
    };
    
    (StatusCode::OK, Bytes::from(resp.encode_to_vec()))
}

// POST /guests - Lấy danh sách guest
async fn guests_handler(State(state): State<Arc<AppState>>, body: Bytes) -> impl IntoResponse {
    let req = match GuestListRequest::decode(&body[..]) {
        Ok(r) => r,
        Err(_) => return (StatusCode::BAD_REQUEST, Bytes::new()),
    };
    
    // Verify admin trước
    if state.repo.verify_admin(&req.shop_id, &req.admin_pin).await.ok().flatten().is_none() {
        let resp = GuestListResponse { success: false, guests: vec![], error: "Unauthorized".into() };
        return (StatusCode::UNAUTHORIZED, Bytes::from(resp.encode_to_vec()));
    }
    
    let guests = state.repo.get_guests(&req.shop_id).await.unwrap_or_default();
    let resp = GuestListResponse { success: true, guests, error: String::new() };
    (StatusCode::OK, Bytes::from(resp.encode_to_vec()))
}

// POST /sync - Lấy tin nhắn
async fn sync_handler(State(state): State<Arc<AppState>>, body: Bytes) -> impl IntoResponse {
    let req = match SyncRequest::decode(&body[..]) {
        Ok(r) => r,
        Err(_) => return (StatusCode::BAD_REQUEST, Bytes::new()),
    };
    
    let messages = state.repo.fetch_messages(&req.shop_id, req.guest_id, req.after_message_id, req.limit).await
        .unwrap_or_default();
    
    let mut resp = SyncResponse {
        messages,
        server_timestamp_us: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as u64,
        has_more: false,
        crc32: 0,
    };
    resp.finalize();
    
    (StatusCode::OK, Bytes::from(resp.encode_to_vec()))
}