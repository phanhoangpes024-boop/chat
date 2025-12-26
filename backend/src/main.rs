use backend::{sync, db};
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
    
    if use_mock {
        println!("📦 MockRepo (In-Memory)");
        let repo = sync::MockRepo::new();
        let state = Arc::new(sync::AppState { repo });
        start_server(state).await
    } else {
        println!("🗄️  Connecting to ScyllaDB...");
        let nodes = vec!["127.0.0.1:9042"];
        
        let repo = match db::ScyllaRepo::new(nodes).await {
            Ok(r) => {
                println!("✅ ScyllaDB connected");
                r
            }
            Err(e) => {
                eprintln!("❌ ScyllaDB failed: {}", e);
                eprintln!("   Run: docker run -d -p 9042:9042 scylladb/scylla");
                eprintln!("   Or: $env:USE_MOCK_DB=\"true\"");
                return Err(Box::new(e));
            }
        };
        
        let state = Arc::new(sync::AppState { repo });
        start_server(state).await
    }
}

async fn start_server<R: sync::MessageRepo + 'static>(
    state: Arc<sync::AppState<R>>
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
        .layer(cors)
        .with_state(state);
    
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    
    println!("🌐 HTTP Server (Dev Only)");
    println!("   http://localhost:8080");
    println!("");
    println!("📡 Endpoints:");
    println!("   POST /sync (read)");
    println!("   POST /send (write)");
    println!("");
    println!("✅ Ready! Press Ctrl+C to stop");
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    
    axum::serve(listener, app)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}