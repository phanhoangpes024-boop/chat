fn main() {
    let mut config = prost_build::Config::new();
    // Dòng này quan trọng: Ép tất cả các trường kiểu 'bytes' trong Proto
    // chuyển thành kiểu 'bytes::Bytes' của Rust thay vì 'Vec<u8>'
    config.bytes(&["."]); 
    
    config.compile_protos(&["proto/chat.proto"], &["proto/"]).unwrap();
}