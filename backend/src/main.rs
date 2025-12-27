use backend::{sync, db, websocket};
use axum::Router;
use std::sync::Arc;
use tower_http::cors::{CorsLayer, Any};
use std::time::Duration;
use axum::http::Method;
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    if let Err(e) = run_server().await {
        eprintln!("❌ FATAL ERROR: {}", e);
        std::process::exit(1);
    }
}

async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 TurboChat Backend Starting...");
    
    let use_mock = std::env::var("USE_MOCK_DB")
        .unwrap_or_else(|_| "false".to_string())
        .parse::<bool>()
        .unwrap_or(false);
    
    let redis_url = "redis://127.0.0.1:6379";
    
    if use_mock {
        println!("📦 MockRepo (In-Memory)");
        let repo_concrete = Arc::new(sync::MockRepo::new());
        let repo_dyn: Arc<dyn sync::MessageRepo> = repo_concrete;
        
        println!("🔗 Setting up WebSocket + Redis...");
        let ws_state = Arc::new(
            websocket::WebSocketState::new(redis_url, repo_dyn.clone())
                .await
                .map_err(|e| format!("WebSocket state failed: {}", e))?
        );
        
        let ws_state_clone = Arc::clone(&ws_state);
        tokio::spawn(async move {
            websocket::redis_subscriber_task(ws_state_clone).await;
        });
        
        println!("✅ WebSocket + Redis ready");
        
        let state = Arc::new(sync::AppState { repo: repo_dyn });
        start_server(state, ws_state).await
    } else {
        println!("🗄️  Connecting to ScyllaDB...");
        let nodes = vec!["127.0.0.1:9042"];
        
        let repo_concrete = match db::ScyllaRepo::new(nodes).await {
            Ok(r) => {
                println!("✅ ScyllaDB connected");
                Arc::new(r)
            }
            Err(e) => {
                eprintln!("❌ ScyllaDB failed: {}", e);
                return Err(Box::new(e));
            }
        };
        
        let repo_dyn: Arc<dyn sync::MessageRepo> = repo_concrete;
        
        println!("🔗 Setting up WebSocket + Redis...");
        let ws_state = Arc::new(
            websocket::WebSocketState::new(redis_url, repo_dyn.clone())
                .await
                .map_err(|e| format!("WebSocket state failed: {}", e))?
        );
        
        let ws_state_clone = Arc::clone(&ws_state);
        tokio::spawn(async move {
            websocket::redis_subscriber_task(ws_state_clone).await;
        });
        
        println!("✅ WebSocket + Redis ready");
        
        let state = Arc::new(sync::AppState { repo: repo_dyn });
        start_server(state, ws_state).await
    }
}

async fn start_server<R: sync::MessageRepo + 'static>(
    state: Arc<sync::AppState<R>>,
    ws_state: Arc<websocket::WebSocketState>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any)
        .expose_headers(Any)
        .max_age(Duration::from_secs(3600));
    
    let app = Router::new()
        .route("/sync", axum::routing::post(sync::sync_handler))
        .route("/send", axum::routing::post(sync::send_handler))
        .with_state(state)
        .route("/ws", axum::routing::get(websocket::ws_handler))
        .with_state(ws_state)
        .layer(cors);
    
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    
    println!("🌐 Server Ready!");
    println!("   HTTP: http://localhost:8080");
    println!("   WebSocket: ws://localhost:8080/ws");
    println!("");
    println!("📡 Endpoints:");
    println!("   POST /sync (polling)");
    println!("   POST /send (write)");
    println!("   GET  /ws   (realtime)");
    println!("");
    println!("✅ Ready! Press Ctrl+C to stop");
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    
    axum::serve(listener, app)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}