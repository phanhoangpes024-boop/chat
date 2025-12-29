-- TurboChat Schema - Multi-tenant
CREATE KEYSPACE IF NOT EXISTS turbochat
WITH replication = {
    'class': 'SimpleStrategy',
    'replication_factor': 1
};

USE turbochat;

-- ============================================================================
-- SHOPS - Chủ shop đăng ký widget
-- ============================================================================
CREATE TABLE IF NOT EXISTS shops (
    shop_id text,
    shop_name text,
    admin_pin text,          -- PIN 6 số để vào admin
    created_at bigint,
    PRIMARY KEY (shop_id)
);

-- ============================================================================
-- GUESTS - Khách vào chat của mỗi shop
-- ============================================================================
CREATE TABLE IF NOT EXISTS guests (
    shop_id text,
    guest_id bigint,
    guest_name text,
    created_at bigint,
    last_seen bigint,
    PRIMARY KEY ((shop_id), guest_id)
) WITH CLUSTERING ORDER BY (guest_id DESC);

-- ============================================================================
-- MESSAGES - Tin nhắn theo shop + guest
-- ============================================================================
CREATE TABLE IF NOT EXISTS messages (
    shop_id text,
    guest_id bigint,
    message_id bigint,
    sender_type text,        -- 'guest' hoặc 'admin'
    content blob,
    timestamp_us bigint,
    content_crc int,
    PRIMARY KEY ((shop_id, guest_id), message_id)
) WITH CLUSTERING ORDER BY (message_id ASC);

-- ============================================================================
-- SEED DATA - Shop mẫu để test
-- ============================================================================
INSERT INTO shops (shop_id, shop_name, admin_pin, created_at) 
VALUES ('demo123', 'Shop Demo', '123456', 1735400000000000);