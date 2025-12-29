use leptos::prelude::*;
use leptos::html::Div;
use turbochat_shared::{Message as ChatMessage, AdminAuthRequest, AdminAuthResponse};
use prost::Message as ProstMessage;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{WebSocket, MessageEvent, CloseEvent, ErrorEvent};
use gloo_net::http::Request;
use serde::{Deserialize, Serialize};

// ===== API Configuration =====
const API_BASE_URL: &str = "http://localhost:8080";
const WS_BASE_URL: &str = "ws://localhost:8080";

#[derive(Clone)]
struct SendWebSocket(WebSocket);
unsafe impl Send for SendWebSocket {}
unsafe impl Sync for SendWebSocket {}

#[derive(Clone, Debug)]
struct ChatUser {
    guest_id: u64,
    name: String,
    last_message: String,
    time: String,
}

#[derive(Clone, Debug)]
struct DisplayMessage {
    sender_type: String,
    text: String,
    time: String,
}

#[component]
pub fn App() -> impl IntoView {
    // Auth state
    let (is_logged_in, set_is_logged_in) = signal(false);
    let (shop_id, set_shop_id) = signal(String::new());
    let (shop_name, set_shop_name) = signal(String::new());
    let (pin_input, set_pin_input) = signal(String::new());
    let (shop_id_input, set_shop_id_input) = signal(String::new());
    let (login_error, set_login_error) = signal(String::new());
    let (is_loading, set_is_loading) = signal(false);

    // Check URL params hoặc localStorage
    Effect::new(move |_| {
        let window = web_sys::window().unwrap();
        let storage = window.local_storage().unwrap().unwrap();
        
        // Kiểm tra đã login chưa
        if let (Some(saved_shop), Some(saved_name)) = (
            storage.get_item("turbochat_admin_shop").ok().flatten(),
            storage.get_item("turbochat_admin_name").ok().flatten()
        ) {
            set_shop_id.set(saved_shop);
            set_shop_name.set(saved_name);
            set_is_logged_in.set(true);
        }
    });

    // Login handler
    let do_login = move || {
        let shop = shop_id_input.get_untracked();
        let pin = pin_input.get_untracked();
        
        if shop.is_empty() || pin.len() != 6 {
            set_login_error.set("Shop ID và PIN 6 số là bắt buộc".to_string());
            return;
        }
        
        set_is_loading.set(true);
        set_login_error.set(String::new());
        
        spawn_local(async move {
            let req = AdminAuthRequest {
                shop_id: shop.clone(),
                admin_pin: pin,
            };
            
            let result = Request::post(&format!("{}/auth", API_BASE_URL))
                .header("Content-Type", "application/octet-stream")
                .body(req.encode_to_vec())
                .unwrap()
                .send()
                .await;
            
            set_is_loading.set(false);
            
            match result {
                Ok(resp) => {
                    if let Ok(bytes) = resp.binary().await {
                        if let Ok(auth_resp) = AdminAuthResponse::decode(&bytes[..]) {
                            if auth_resp.success {
                                // Lưu session
                                let window = web_sys::window().unwrap();
                                let storage = window.local_storage().unwrap().unwrap();
                                let _ = storage.set_item("turbochat_admin_shop", &shop);
                                let _ = storage.set_item("turbochat_admin_name", &auth_resp.shop_name);
                                
                                set_shop_id.set(shop);
                                set_shop_name.set(auth_resp.shop_name);
                                set_is_logged_in.set(true);
                            } else {
                                set_login_error.set(auth_resp.error);
                            }
                        }
                    }
                }
                Err(e) => {
                    set_login_error.set(format!("Lỗi kết nối: {}", e));
                }
            }
        });
    };

    // Logout handler
    let do_logout = move || {
        let window = web_sys::window().unwrap();
        let storage = window.local_storage().unwrap().unwrap();
        let _ = storage.remove_item("turbochat_admin_shop");
        let _ = storage.remove_item("turbochat_admin_name");
        set_is_logged_in.set(false);
        set_shop_id.set(String::new());
        set_shop_name.set(String::new());
    };

    view! {
        <Show 
            when=move || is_logged_in.get()
            fallback=move || view! { <LoginPage 
                shop_id_input=shop_id_input
                set_shop_id_input=set_shop_id_input
                pin_input=pin_input
                set_pin_input=set_pin_input
                login_error=login_error
                is_loading=is_loading
                on_login=do_login
            /> }
        >
            <Dashboard 
                shop_id=shop_id.get() 
                shop_name=shop_name.get()
                on_logout=do_logout
            />
        </Show>
    }
}

// ============================================================================
// LOGIN PAGE
// ============================================================================
#[component]
fn LoginPage(
    shop_id_input: ReadSignal<String>,
    set_shop_id_input: WriteSignal<String>,
    pin_input: ReadSignal<String>,
    set_pin_input: WriteSignal<String>,
    login_error: ReadSignal<String>,
    is_loading: ReadSignal<bool>,
    on_login: impl Fn() + 'static + Clone,
) -> impl IntoView {
    let on_login_click = on_login.clone();
    let on_login_enter = on_login.clone();
    
    view! {
        <style>{include_str!("../login_style.css")}</style>
        
        <div class="login-container">
            <div class="login-box">
                <div class="login-header">
                    <h1>"🚀 TurboChat"</h1>
                    <p>"Admin Dashboard"</p>
                </div>
                
                <div class="login-form">
                    <div class="input-group">
                        <label>"Shop ID"</label>
                        <input 
                            type="text" 
                            placeholder="Ví dụ: demo123"
                            prop:value=move || shop_id_input.get()
                            on:input=move |e| set_shop_id_input.set(event_target_value(&e))
                        />
                    </div>
                    
                    <div class="input-group">
                        <label>"Mã PIN (6 số)"</label>
                        <input 
                            type="password" 
                            placeholder="••••••"
                            maxlength="6"
                            prop:value=move || pin_input.get()
                            on:input=move |e| set_pin_input.set(event_target_value(&e))
                            on:keypress={
                                let login = on_login_enter.clone();
                                move |e: web_sys::KeyboardEvent| {
                                    if e.key() == "Enter" { login(); }
                                }
                            }
                        />
                    </div>
                    
                    <Show when=move || !login_error.get().is_empty()>
                        <div class="error-message">{move || login_error.get()}</div>
                    </Show>
                    
                    <button 
                        class="login-btn"
                        disabled=move || is_loading.get()
                        on:click=move |_| on_login_click()
                    >
                        {move || if is_loading.get() { "Đang xác thực..." } else { "Đăng nhập" }}
                    </button>
                </div>
                
                <div class="login-footer">
                    <p>"Shop demo: "<strong>"demo123"</strong>" / PIN: "<strong>"123456"</strong></p>
                </div>
            </div>
        </div>
    }
}

// ============================================================================
// DASHBOARD (Chat interface)
// ============================================================================
#[component]
fn Dashboard(
    shop_id: String,
    shop_name: String,
    on_logout: impl Fn() + 'static + Clone,
) -> impl IntoView {
    let (chat_users, set_chat_users) = signal(Vec::<ChatUser>::new());
    let (current_guest_id, set_current_guest_id) = signal(0u64);
    let (messages, set_messages) = signal(Vec::<DisplayMessage>::new());
    let (message_input, set_message_input) = signal(String::new());
    let (connection_status, set_connection_status) = signal("🔴 Đang kết nối...".to_string());
    let (send_trigger, set_send_trigger) = signal(0u64);
    
    let scrollable_ref = NodeRef::<Div>::new();
    let ws_ref = StoredValue::new(None::<SendWebSocket>);

    let shop_id_ws = shop_id.clone();
    
    // WebSocket connection
    Effect::new(move |_| {
        let url = format!("{}/ws?shop_id={}", WS_BASE_URL, shop_id_ws);
        let ws = match WebSocket::new(&url) {
            Ok(w) => w,
            Err(_) => return,
        };
        
        {
            let on_open = Closure::wrap(Box::new(move |_: JsValue| {
                set_connection_status.set("🟢 Đã kết nối".to_string());
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
            on_open.forget();
        }
        
        {
            let on_message = Closure::wrap(Box::new(move |event: MessageEvent| {
                if let Ok(blob) = event.data().dyn_into::<web_sys::Blob>() {
                    let fr = web_sys::FileReader::new().unwrap();
                    let fr_clone = fr.clone();
                    
                    let onload = Closure::wrap(Box::new(move |_: web_sys::ProgressEvent| {
                        if let Ok(ab) = fr_clone.result().unwrap().dyn_into::<js_sys::ArrayBuffer>() {
                            let bytes = js_sys::Uint8Array::new(&ab).to_vec();
                            
                            if let Ok(msg) = ChatMessage::decode(&bytes[..]) {
                                let guest_id = msg.guest_id;
                                
                                set_chat_users.update(|users| {
                                    if !users.iter().any(|u| u.guest_id == guest_id) {
                                        users.push(ChatUser {
                                            guest_id,
                                            name: format!("Khách #{}", guest_id % 10000),
                                            last_message: String::from_utf8_lossy(&msg.content).to_string(),
                                            time: format_time(msg.timestamp_us),
                                        });
                                    } else if let Some(user) = users.iter_mut().find(|u| u.guest_id == guest_id) {
                                        user.last_message = String::from_utf8_lossy(&msg.content).to_string();
                                        user.time = format_time(msg.timestamp_us);
                                    }
                                });
                                
                                set_messages.update(|msgs| {
                                    msgs.push(DisplayMessage {
                                        sender_type: msg.sender_type.clone(),
                                        text: String::from_utf8_lossy(&msg.content).to_string(),
                                        time: format_time(msg.timestamp_us),
                                    });
                                });
                            }
                        }
                    }) as Box<dyn FnMut(web_sys::ProgressEvent)>);
                    
                    fr.set_onload(Some(onload.as_ref().unchecked_ref()));
                    onload.forget();
                    let _ = fr.read_as_array_buffer(&blob);
                }
            }) as Box<dyn FnMut(MessageEvent)>);
            ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            on_message.forget();
        }
        
        {
            let on_close = Closure::wrap(Box::new(move |_: CloseEvent| {
                set_connection_status.set("🟡 Mất kết nối".to_string());
            }) as Box<dyn FnMut(CloseEvent)>);
            ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
            on_close.forget();
        }
        
        ws_ref.set_value(Some(SendWebSocket(ws)));
    });

    // Send message effect
    let shop_id_send = shop_id.clone();
    Effect::new(move |_| {
        let trigger = send_trigger.get();
        if trigger == 0 { return; }
        
        let text = message_input.get_untracked();
        if text.trim().is_empty() { return; }

        let guest_id = current_guest_id.get_untracked();
        if guest_id == 0 { return; }

        if let Some(ws) = ws_ref.get_value() {
            if ws.0.ready_state() == WebSocket::OPEN {
                let ts = js_sys::Date::now() as u64 * 1000;
                let content = text.as_bytes();
                
                let msg = ChatMessage {
                    shop_id: shop_id_send.clone(),
                    guest_id,
                    message_id: ts,
                    sender_type: "admin".to_string(),
                    content: content.to_vec().into(),
                    timestamp_us: ts,
                    content_crc: crc32c::crc32c(content),
                };
                
                let bytes = msg.encode_to_vec();
                let arr = js_sys::Uint8Array::from(&bytes[..]);
                let _ = ws.0.send_with_array_buffer(&arr.buffer());
                set_message_input.set(String::new());
            }
        }
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

    let shop_name_display = shop_name.clone();
    let on_logout_click = on_logout.clone();

    view! {
        <style>{include_str!("../telegram_style.css")}</style>

        <div class="app-container">
            <div class="sidebar">
                <div class="sidebar-header">
                    <div class="shop-info">
                        <span class="shop-name">{shop_name_display}</span>
                        <button class="logout-btn" on:click=move |_| on_logout_click()>"Đăng xuất"</button>
                    </div>
                </div>

                <div class="chat-list">
                    <Show when=move || chat_users.get().is_empty()>
                        <div class="empty-state">"Chưa có khách nào nhắn tin"</div>
                    </Show>
                    <For
                        each=move || chat_users.get()
                        key=|chat| chat.guest_id
                        children=move |chat: ChatUser| {
                            let guest_id = chat.guest_id;
                            let is_active = move || current_guest_id.get() == guest_id;
                            
                            view! {
                                <div class="chat-item" class:active=is_active
                                    on:click=move |_| set_current_guest_id.set(guest_id)>
                                    <div class="avatar green">"K"</div>
                                    <div class="chat-info">
                                        <div class="chat-header">
                                            <span class="chat-name">{chat.name.clone()}</span>
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

            <div class="chat-area">
                <div class="chat-header-bar">
                    <div class="chat-header-info">
                        <div class="chat-header-name">
                            {move || if current_guest_id.get() == 0 { 
                                "Chọn cuộc trò chuyện".to_string() 
                            } else { 
                                format!("Khách #{}", current_guest_id.get() % 10000) 
                            }}
                        </div>
                        <div class="chat-header-status">{move || connection_status.get()}</div>
                    </div>
                </div>

                <div class="scrollable-content" node_ref=scrollable_ref>
                    <div class="messages-container">
                        <For
                            each=move || messages.get()
                            key=|msg| format!("{}{}", msg.time, msg.text)
                            children=move |msg: DisplayMessage| {
                                let class = if msg.sender_type == "admin" { "message sent" } else { "message received" };
                                view! {
                                    <div class=class>
                                        <div class="message-bubble">
                                            <div class="message-text">{msg.text.clone()}</div>
                                            <div class="message-meta"><span>{msg.time.clone()}</span></div>
                                        </div>
                                    </div>
                                }
                            }
                        />
                    </div>
                </div>

                <div class="input-area">
                    <div class="input-bubble">
                        <input type="text" class="message-input" placeholder="Nhập tin nhắn..."
                            disabled=move || current_guest_id.get() == 0
                            prop:value=move || message_input.get()
                            on:input=move |e| set_message_input.set(event_target_value(&e))
                            on:keypress=move |e: web_sys::KeyboardEvent| { 
                                if e.key() == "Enter" { 
                                    set_send_trigger.set(js_sys::Date::now() as u64); 
                                } 
                            }
                        />
                        <button class="send-button" 
                            disabled=move || current_guest_id.get() == 0
                            on:click=move |_| set_send_trigger.set(js_sys::Date::now() as u64)
                        >"➤"</button>
                    </div>
                </div>
            </div>
        </div>
    }
}

fn format_time(timestamp_us: u64) -> String {
    let secs = (timestamp_us / 1_000_000) as f64;
    let datetime = js_sys::Date::new(&(secs * 1000.0).into());
    format!("{:02}:{:02}", datetime.get_hours(), datetime.get_minutes())
}