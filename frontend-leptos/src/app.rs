use leptos::prelude::*;
use leptos::html::Div;
use turbochat_shared::{SyncRequest, SyncResponse, SendRequest, SendResponse};
use prost::Message as ProstMessage;
use gloo_net::http::Request;
use wasm_bindgen_futures::spawn_local;
use wasm_bindgen::prelude::*;
use web_sys::{WebSocket, MessageEvent, CloseEvent, ErrorEvent};

// ============================================================================
// SEND WRAPPER - Fix "cannot be sent between threads" error
// ============================================================================

/// Wrapper để WebSocket có thể dùng trong StoredValue
/// WASM chạy single-threaded nên việc này an toàn
#[derive(Clone, Debug)]
struct SendWebSocket(WebSocket);

unsafe impl Send for SendWebSocket {}
unsafe impl Sync for SendWebSocket {}

impl SendWebSocket {
    // Hàm tạo mới
    pub fn new(ws: WebSocket) -> Self {
        Self(ws)
    }
    
    // Hàm lấy WebSocket bên trong ra dùng
    pub fn inner(&self) -> &WebSocket {
        &self.0
    }
}

// ============================================================================
// DATA STRUCTURES
// ============================================================================

#[derive(Clone, Debug)]
struct Chat {
    id: u64,
    name: String,
    initials: String,
    color: String,
    last_message: String,
    time: String,
    verified: bool,
}

#[derive(Clone, Debug)]
struct DisplayMessage {
    message_type: String,
    text: String,
    time: String,
}

// ============================================================================
// MAIN APP COMPONENT
// ============================================================================

#[component]
pub fn App() -> impl IntoView {
    // State management
    let (chats, _set_chats) = signal(vec![
        Chat {
            id: 100,
            name: "TurboChat Room".to_string(),
            initials: "TC".to_string(),
            color: "blue".to_string(),
            last_message: "Realtime với WebSocket + Redis".to_string(),
            time: "Dec 27".to_string(),
            verified: true,
        },
    ]);

    let (current_chat_id, set_current_chat_id) = signal(100u64);
    let (messages, set_messages) = signal(Vec::<DisplayMessage>::new());
    let (message_input, set_message_input) = signal(String::new());
    let (search_input, set_search_input) = signal(String::new());
    let (sidebar_hidden, set_sidebar_hidden) = signal(false);
    let (connection_status, set_connection_status) = signal(String::from("🔴 Disconnected"));
    
    let scrollable_ref = NodeRef::<Div>::new();
    let ws_ref = StoredValue::new(None::<SendWebSocket>);

    // WebSocket connection
    Effect::new(move |_| {
        let ws = WebSocket::new("ws://localhost:8080/ws").unwrap();
        
        // OnOpen
        {
            let on_open = Closure::wrap(Box::new(move |_event: JsValue| {
                set_connection_status.set("🟢 Connected".to_string());
                leptos::logging::log!("✅ WebSocket connected");
            }) as Box<dyn FnMut(JsValue)>);
            
            ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
            on_open.forget();
        }
        
        // OnMessage - Nhận Protobuf binary
        {
            let on_message = Closure::wrap(Box::new(move |event: MessageEvent| {
                leptos::logging::log!("📨 WebSocket received event");
                
                // XỬ LÝ BLOB
                if let Ok(blob) = event.data().dyn_into::<web_sys::Blob>() {
                    leptos::logging::log!("📦 Received Blob: {} bytes", blob.size() as usize);
                    
                    // Convert Blob → ArrayBuffer → Vec<u8>
                    let file_reader = web_sys::FileReader::new().unwrap();
                    let fr_clone = file_reader.clone();
                    
                    let onload = Closure::wrap(Box::new(move |_event: web_sys::ProgressEvent| {
                        if let Ok(array_buffer) = fr_clone.result().unwrap().dyn_into::<js_sys::ArrayBuffer>() {
                            let uint8_array = js_sys::Uint8Array::new(&array_buffer);
                            let bytes = uint8_array.to_vec();
                            
                            leptos::logging::log!("✅ Converted to {} bytes", bytes.len());
                            
                            if let Ok(msg) = turbochat_shared::Message::decode(&bytes[..]) {
                                let content = String::from_utf8_lossy(&msg.content).to_string();
                                let time = format_time(msg.timestamp_us);
                                
                                leptos::logging::log!("✅ Decoded message: {}", content);
                                
                                let message_type = if msg.sender_id == 42 {
                                    "sent"
                                } else {
                                    "received"
                                };
                                
                                let display_msg = DisplayMessage {
                                    message_type: message_type.to_string(),
                                    text: content,
                                    time,
                                };
                                
                                set_messages.update(|msgs| {
                                    msgs.push(display_msg);
                                });
                                
                                leptos::logging::log!("✅ Message added to UI");
                            }
                        }
                    }) as Box<dyn FnMut(web_sys::ProgressEvent)>);
                    
                    file_reader.set_onload(Some(onload.as_ref().unchecked_ref()));
                    onload.forget();
                    
                    let _ = file_reader.read_as_array_buffer(&blob);
                    
                } else if let Ok(array_buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
                    // Fallback cho ArrayBuffer (nếu có)
                    let uint8_array = js_sys::Uint8Array::new(&array_buffer);
                    let bytes = uint8_array.to_vec();
                    
                    if let Ok(msg) = turbochat_shared::Message::decode(&bytes[..]) {
                        let content = String::from_utf8_lossy(&msg.content).to_string();
                        let time = format_time(msg.timestamp_us);
                        
                        let message_type = if msg.sender_id == 42 { "sent" } else { "received" };
                        
                        let display_msg = DisplayMessage {
                            message_type: message_type.to_string(),
                            text: content,
                            time,
                        };
                        
                        set_messages.update(|msgs| {
                            msgs.push(display_msg);
                        });
                    }
                } else {
                    leptos::logging::log!("❌ Unknown data type");
                }
            }) as Box<dyn FnMut(MessageEvent)>);
            
            ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            on_message.forget();
        }
        
        // OnError
        {
            let on_error = Closure::wrap(Box::new(move |_error: ErrorEvent| {
                set_connection_status.set("🔴 Error".to_string());
                leptos::logging::log!("❌ WebSocket error");
            }) as Box<dyn FnMut(ErrorEvent)>);
            
            ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            on_error.forget();
        }
        
        // OnClose
        {
            let on_close = Closure::wrap(Box::new(move |_event: CloseEvent| {
                set_connection_status.set("🟡 Reconnecting...".to_string());
                leptos::logging::log!("⚠️ WebSocket closed");
            }) as Box<dyn FnMut(CloseEvent)>);
            
            ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
            on_close.forget();
        }
        
        ws_ref.set_value(Some(SendWebSocket::new(ws)));
    });

    // Auto-scroll
    Effect::new(move |_| {
        messages.get();
        
        request_animation_frame(move || {
            if let Some(div) = scrollable_ref.get_untracked() {
                div.set_scroll_top(div.scroll_height());
            }
        });
    });

    // Event handlers
    let send_message = move || {
        let text = message_input.get_untracked();
        if text.trim().is_empty() {
            return;
        }

        let chat_id = current_chat_id.get_untracked();

        
        // Gửi qua WebSocket (Protobuf binary)
        if let Some(ws_wrapper) = ws_ref.get_value() {
            let ws = ws_wrapper.inner();
            
            if ws.ready_state() == WebSocket::OPEN {
                // Tạo ChatMessage
                let timestamp_us = js_sys::Date::now() as u64 * 1000;
                let content_bytes = text.as_bytes();
                let content_crc = crc32c::crc32c(content_bytes);
                
                let msg = turbochat_shared::Message {
                    message_id: timestamp_us,
                    chat_id,
                    sender_id: 42,
                    content: content_bytes.to_vec().into(),
                    timestamp_us,
                    content_crc,
                };
                
                // Encode to Protobuf
                let bytes = msg.encode_to_vec();
                
                // Send binary
                let array = js_sys::Uint8Array::from(&bytes[..]);
                let _ = ws.send_with_array_buffer(&array.buffer());
                
                set_message_input.set(String::new());
            }
        }
    };

    let on_send_click = move |_| {
        send_message();
    };

    let on_key_enter = move |e: web_sys::KeyboardEvent| {
        if e.key() == "Enter" {
            send_message();
        }
    };

    let on_sync = move |_| {
        let chat_id = current_chat_id.get_untracked();
        spawn_local(async move {
            if let Ok(msgs) = do_sync(chat_id).await {
                set_messages.set(msgs);
            }
        });
    };

    let on_back = move |_| {
        set_sidebar_hidden.set(false);
    };

    let on_chat_click = move |chat_id: u64| {
        set_current_chat_id.set(chat_id);
        set_sidebar_hidden.set(true);
        
        spawn_local(async move {
            if let Ok(msgs) = do_sync(chat_id).await {
                set_messages.set(msgs);
            }
        });
    };

    view! {
        <style>
            {include_str!("../telegram_style.css")}
        </style>

        <div class="app-container">
            // SIDEBAR
            <div class="sidebar" class:hidden=move || sidebar_hidden.get()>
                <div class="sidebar-header">
                    <div class="menu-icon">
                        <span></span>
                        <span></span>
                        <span></span>
                    </div>
                    <input
                        type="text"
                        class="search-box"
                        placeholder="Search"
                        prop:value=move || search_input.get()
                        on:input=move |e| set_search_input.set(event_target_value(&e))
                    />
                </div>

                <div class="chat-list">
                    <For
                        each=move || chats.get()
                        key=|chat| chat.id
                        children=move |chat: Chat| {
                            let chat_id = chat.id;
                            let is_active = move || current_chat_id.get() == chat_id;
                            
                            view! {
                                <div
                                    class="chat-item"
                                    class:active=is_active
                                    on:click=move |_| on_chat_click(chat_id)
                                >
                                    <div class=format!("avatar {}", chat.color)>
                                        {chat.initials.clone()}
                                    </div>
                                    <div class="chat-info">
                                        <div class="chat-header">
                                            <span class="chat-name">{chat.name.clone()}</span>
                                            {if chat.verified {
                                                view! { <span class="verified-badge">"✓"</span> }.into_any()
                                            } else {
                                                view! {}.into_any()
                                            }}
                                        </div>
                                        <div class="chat-message">{chat.last_message.clone()}</div>
                                    </div>
                                    <span class="chat-time">{chat.time.clone()}</span>
                                </div>
                            }
                        }
                    />
                </div>
            </div>

            <div class="resize-divider"></div>

            // CHAT AREA
            <div class="chat-area">
                <div class="chat-header-bar">
                    <span class="back-button" on:click=on_back>"←"</span>
                    <div class="chat-header-avatar blue">"TC"</div>
                    <div class="chat-header-info">
                        <div class="chat-header-name">"TurboChat Room"</div>
                        <div class="chat-header-status">
                            {move || connection_status.get()}
                        </div>
                    </div>
                    <div class="chat-header-icons">
                        <span on:click=on_sync style="cursor: pointer;">"🔄"</span>
                        <span>"📞"</span>
                        <span>"⋮"</span>
                    </div>
                </div>

                <div class="scrollable-content" node_ref=scrollable_ref>
                    <div class="messages-container">
                        <For
                            each=move || messages.get()
                            key=|msg| format!("{}{}", msg.time, msg.text)
                            children=move |msg: DisplayMessage| {
                                view! {
                                    <div class=format!("message {}", msg.message_type)>
                                        <div class="message-bubble">
                                            <div class="message-text">{msg.text.clone()}</div>
                                            <div class="message-meta">
                                                <span>{msg.time.clone()}</span>
                                                {if msg.message_type == "sent" {
                                                    view! { <span class="checkmark">"✓✓"</span> }.into_any()
                                                } else {
                                                    view! {}.into_any()
                                                }}
                                            </div>
                                        </div>
                                    </div>
                                }
                            }
                        />
                    </div>
                </div>

                <div class="input-area">
                    <div class="input-bubble">
                        <span class="input-icon emoji-icon">"😊"</span>
                        
                        <input
                            type="text"
                            class="message-input"
                            placeholder="Message"
                            prop:value=move || message_input.get()
                            on:input=move |e| set_message_input.set(event_target_value(&e))
                            on:keypress=on_key_enter
                        />
                        
                        <span class="input-icon attachment-icon">"📎"</span>
                        
                        <button class="send-button" on:click=on_send_click>"➤"</button>
                    </div>
                </div>
            </div>
        </div>
    }
}

// Fallback HTTP sync
async fn do_sync(chat_id: u64) -> Result<Vec<DisplayMessage>, String> {
    let request = SyncRequest {
        chat_id,
        after_message_id: 0,
        limit: 100,
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
        
        let message_type = if msg.sender_id == 42 {
            "sent"
        } else {
            "received"
        };
        
        msgs.push(DisplayMessage {
            message_type: message_type.to_string(),
            text: content,
            time,
        });
    }
    
    Ok(msgs)
}

fn format_time(timestamp_us: u64) -> String {
    let secs = (timestamp_us / 1_000_000) as i64;
    let datetime = js_sys::Date::new(&(secs as f64 * 1000.0).into());
    
    let hours = datetime.get_hours();
    let minutes = datetime.get_minutes();
    
    format!("{:02}:{:02}", hours, minutes)
}