mod widget;

use wasm_bindgen::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    leptos::logging::log!("🚀 TurboChat Widget starting...");
    
    if !try_mount() {
        leptos::logging::log!("⏳ Waiting for DOM...");
        wait_and_mount();
    }
}

fn try_mount() -> bool {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return false,
    };
    let document = match window.document() {
        Some(d) => d,
        None => return false,
    };
    
    let root = match document.get_element_by_id("turbochat-root") {
        Some(r) => r,
        None => return false,
    };
    
    // Đọc shop_id từ data-shop-id attribute
    let shop_id = root.get_attribute("data-shop-id")
        .unwrap_or_else(|| "demo123".to_string());
    
    leptos::logging::log!("✅ Found root, shop_id: {}", shop_id);
    
    // Mount widget với shop_id
    leptos::mount::mount_to(
        root.unchecked_into(),
        move || widget::Widget(widget::WidgetProps { shop_id: shop_id.clone() })
    ).forget();
    
    leptos::logging::log!("✅ Widget mounted!");
    true
}

fn wait_and_mount() {
    use wasm_bindgen::closure::Closure;
    use std::cell::Cell;
    use std::rc::Rc;
    
    let window = web_sys::window().unwrap();
    let attempts = Rc::new(Cell::new(0));
    
    let closure: Closure<dyn FnMut()> = Closure::new(move || {
        if try_mount() {
            leptos::logging::log!("✅ Mounted after retry");
            return;
        }
        
        let count = attempts.get() + 1;
        attempts.set(count);
        
        if count >= 50 {
            leptos::logging::log!("❌ Failed to find #turbochat-root");
        }
    });
    
    let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
        closure.as_ref().unchecked_ref(), 100
    );
    closure.forget();
}