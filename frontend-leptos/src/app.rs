use leptos::prelude::*;
use turbochat_shared::{SyncRequest, SyncResponse, SendRequest, SendResponse};
use prost::Message as ProstMessage;
use gloo_net::http::Request;
use wasm_bindgen_futures::spawn_local;

#[derive(Clone)]
struct DisplayMessage {
    content: String,
    time: String,
}

#[component]
pub fn App() -> impl IntoView {
    let (status, set_status) = signal(String::from("Chưa sync"));
    let (messages, set_messages) = signal(Vec::<DisplayMessage>::new());
    let (input_text, set_input_text) = signal(String::new());

    let sync = move |_| {
        spawn_local(async move {
            set_status.set("⏳ Đang sync...".into());
            
            match do_sync().await {
                Ok(msgs) => {
                    set_messages.set(msgs.clone());
                    set_status.set(format!("✅ Sync OK: {} tin nhắn", msgs.len()));
                }
                Err(e) => {
                    set_status.set(format!("❌ Lỗi sync: {}", e));
                }
            }
        });
    };

    let send = move |_| {
        let text = input_text.get();
        if text.is_empty() {
            set_status.set("⚠️  Nội dung rỗng!".into());
            return;
        }

        spawn_local(async move {
            set_status.set("📨 Đang gửi...".into());
            
            match do_send(&text).await {
                Ok(_) => {
                    set_status.set("✅ Gửi thành công!".into());
                    set_input_text.set(String::new());
                    
                    // Auto sync
                    if let Ok(msgs) = do_sync().await {
                        set_messages.set(msgs);
                    }
                }
                Err(e) => {
                    set_status.set(format!("❌ Lỗi gửi: {}", e));
                }
            }
        });
    };

    view! {
        <div style="padding: 20px; font-family: Arial; max-width: 800px; margin: 0 auto;">
            <h1>"🚀 TurboChat"</h1>
            
            <div style="margin: 20px 0; display: flex; gap: 10px;">
                <input
                    type="text"
                    placeholder="Nhập tin nhắn..."
                    prop:value=move || input_text.get()
                    on:input=move |e| set_input_text.set(event_target_value(&e))
                    style="flex: 1; padding: 12px; font-size: 16px; 
                           border: 1px solid #ccc; border-radius: 5px;"
                />
                
                <button 
                    on:click=send
                    style="padding: 12px 24px; font-size: 16px; cursor: pointer; 
                           background: #4CAF50; color: white; border: none; 
                           border-radius: 5px; font-weight: bold;"
                >
                    "📤 Gửi"
                </button>
                
                <button 
                    on:click=sync
                    style="padding: 12px 24px; font-size: 16px; cursor: pointer; 
                           background: #2196F3; color: white; border: none; 
                           border-radius: 5px; font-weight: bold;"
                >
                    "🔄 Sync"
                </button>
            </div>
            
            <div style="padding: 15px; background: #f0f0f0; border-radius: 8px; margin: 10px 0;">
                {move || status.get()}
            </div>
            
            <div style="margin-top: 20px;">
                <h2>"Tin nhắn (" {move || messages.get().len()} ")"</h2>
                
                <For
                    each=move || messages.get()
                    key=|msg| msg.time.clone()
                    children=move |msg: DisplayMessage| {
                        view! {
                            <div style="padding: 15px; margin: 10px 0; background: #e3f2fd; 
                                        border-radius: 8px; border-left: 4px solid #2196F3;">
                                <div style="font-size: 14px; color: #666; margin-bottom: 5px;">
                                    {msg.time.clone()}
                                </div>
                                <div style="font-size: 16px;">
                                    {msg.content.clone()}
                                </div>
                            </div>
                        }
                    }
                />
            </div>
        </div>
    }
}

async fn do_sync() -> Result<Vec<DisplayMessage>, String> {
    let request = SyncRequest {
        chat_id: 100,
        after_message_id: 0,
        limit: 50,
    };
    
    let request_bytes = request.encode_to_vec();
    
    let response = Request::post("http://localhost:8080/sync")
        .header("Content-Type", "application/protobuf")
        .body(request_bytes)
        .map_err(|e| format!("{:?}", e))?
        .send()
        .await
        .map_err(|e| format!("{:?}", e))?;
    
    let response_bytes = response.binary().await.map_err(|e| format!("{:?}", e))?;
    let sync_response = SyncResponse::decode(&response_bytes[..]).map_err(|e| format!("{:?}", e))?;
    
    sync_response.verify_crc().map_err(|e| format!("{:?}", e))?;
    
    let mut msgs = Vec::new();
    for msg in sync_response.messages {
        let content = String::from_utf8_lossy(&msg.content).to_string();
        let time = format_time(msg.timestamp_us);
        msgs.push(DisplayMessage { content, time });
    }
    
    Ok(msgs)
}

async fn do_send(text: &str) -> Result<(), String> {
    let content_bytes = text.as_bytes();
    let content_crc = crc32c::crc32c(content_bytes);
    
    let request = SendRequest {
        chat_id: 100,
        sender_id: 42,
        content: content_bytes.to_vec().into(),
        content_crc,
    };
    
    let request_bytes = request.encode_to_vec();
    
    let response = Request::post("http://localhost:8080/send")
        .header("Content-Type", "application/protobuf")
        .body(request_bytes)
        .map_err(|e| format!("{:?}", e))?
        .send()
        .await
        .map_err(|e| format!("{:?}", e))?;
    
    let response_bytes = response.binary().await.map_err(|e| format!("{:?}", e))?;
    let send_response = SendResponse::decode(&response_bytes[..]).map_err(|e| format!("{:?}", e))?;
    
    if !send_response.success {
        return Err(send_response.error);
    }
    
    Ok(())
}

fn format_time(timestamp_us: u64) -> String {
    let secs = (timestamp_us / 1_000_000) as i64;
    
    let datetime = js_sys::Date::new(&(secs as f64 * 1000.0).into());
    let hours = datetime.get_hours();
    let minutes = datetime.get_minutes();
    let seconds = datetime.get_seconds();
    
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}