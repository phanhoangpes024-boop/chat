use leptos::prelude::*;
use turbochat_shared::Message as ChatMessage;
use prost::Message as ProstMessage;
use wasm_bindgen::prelude::*;
use web_sys::{WebSocket, MessageEvent};

#[derive(Clone)]
struct SendWs(WebSocket);
unsafe impl Send for SendWs {}
unsafe impl Sync for SendWs {}

#[component]
pub fn Widget(shop_id: String) -> impl IntoView {
    let (is_open, set_is_open) = signal(false);
    let (messages, set_messages) = signal(Vec::<(String, String)>::new());
    let (input, set_input) = signal(String::new());
    let (send_trigger, set_send_trigger) = signal(0u64);
    let ws_ref = StoredValue::new(None::<SendWs>);
    
    // Guest ID - lưu localStorage
    let shop_id_storage = shop_id.clone();
    let guest_id = StoredValue::new({
        let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
        let key = format!("turbochat_guest_{}", shop_id_storage);
        if let Some(id) = storage.get_item(&key).ok().flatten() {
            id.parse::<u64>().unwrap_or_else(|_| {
                let new_id = js_sys::Date::now() as u64;
                let _ = storage.set_item(&key, &new_id.to_string());
                new_id
            })
        } else {
            let new_id = js_sys::Date::now() as u64;
            let _ = storage.set_item(&key, &new_id.to_string());
            new_id
        }
    });

    let shop_id_ws = shop_id.clone();
    let guest_id_val = guest_id.get_value();
    
    // WebSocket connection
    Effect::new(move |_| {
        let url = format!("ws://localhost:8080/ws?shop_id={}&guest_id={}", shop_id_ws, guest_id_val);
        let ws = match WebSocket::new(&url) {
            Ok(w) => w,
            Err(_) => return,
        };
        
        let on_message = Closure::wrap(Box::new(move |event: MessageEvent| {
            if let Ok(blob) = event.data().dyn_into::<web_sys::Blob>() {
                let fr = web_sys::FileReader::new().unwrap();
                let fr_clone = fr.clone();
                
                let onload = Closure::wrap(Box::new(move |_: web_sys::ProgressEvent| {
                    if let Ok(ab) = fr_clone.result().unwrap().dyn_into::<js_sys::ArrayBuffer>() {
                        let bytes = js_sys::Uint8Array::new(&ab).to_vec();
                        if let Ok(msg) = ChatMessage::decode(&bytes[..]) {
                            let text = String::from_utf8_lossy(&msg.content).to_string();
                            let sender = msg.sender_type.clone();
                            set_messages.update(|m| m.push((sender, text)));
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
        ws_ref.set_value(Some(SendWs(ws)));
    });

    // Effect xử lý gửi tin nhắn
    let shop_id_send = shop_id.clone();
    Effect::new(move |_| {
        let trigger = send_trigger.get();
        if trigger == 0 { return; }
        
        let text = input.get_untracked();
        if text.trim().is_empty() { return; }

        if let Some(ws) = ws_ref.get_value() {
            if ws.0.ready_state() == WebSocket::OPEN {
                let ts = js_sys::Date::now() as u64 * 1000;
                let content = text.as_bytes();
                
                let msg = ChatMessage {
                    shop_id: shop_id_send.clone(),
                    guest_id: guest_id.get_value(),
                    message_id: ts,
                    sender_type: "guest".to_string(),
                    content: content.to_vec().into(),
                    timestamp_us: ts,
                    content_crc: crc32c::crc32c(content),
                };
                
                let bytes = msg.encode_to_vec();
                let arr = js_sys::Uint8Array::from(&bytes[..]);
                let _ = ws.0.send_with_array_buffer(&arr.buffer());
                set_input.set(String::new());
            }
        }
    });

    view! {
        <div class="turbochat-widget">
            <button class="turbochat-launcher" on:click=move |_| set_is_open.update(|o| *o = !*o)>"💬"</button>
            <Show when=move || is_open.get()>
                <div class="turbochat-popup">
                    <div class="turbochat-header">
                        <span>"Chat với chúng tôi"</span>
                        <button on:click=move |_| set_is_open.set(false)>"✕"</button>
                    </div>
                    <div class="turbochat-messages">
                        <For each=move || messages.get() key=|m| m.1.clone() children=move |(sender, text)| {
                            let class = if sender == "guest" { "turbochat-message sent" } else { "turbochat-message received" };
                            view! { <div class=class>{text}</div> }
                        }/>
                    </div>
                    <div class="turbochat-input">
                        <input type="text" placeholder="Nhập tin nhắn..."
                            prop:value=move || input.get()
                            on:input=move |e| set_input.set(event_target_value(&e))
                            on:keypress=move |e: web_sys::KeyboardEvent| { 
                                if e.key() == "Enter" { 
                                    set_send_trigger.set(js_sys::Date::now() as u64); 
                                } 
                            }
                        />
                        <button on:click=move |_| set_send_trigger.set(js_sys::Date::now() as u64)>"Gửi"</button>
                    </div>
                </div>
            </Show>
        </div>
    }
}