fn main() {
    let mut config = prost_build::Config::new();
    config.bytes(&["."]);  // Dùng bytes::Bytes thay vì Vec<u8>
    
    config
        .compile_protos(&["proto/chat.proto"], &["proto/"])
        .unwrap();
}