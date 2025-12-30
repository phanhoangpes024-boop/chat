use crate::contract::{ContractError, Message, Guest};
use bytes::Bytes;
use reqwest::Client;
use serde_json::json;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

pub struct AstraRepo {
    client: Client,
    base_url: String,
    token: String,
}

impl AstraRepo {
    pub async fn new() -> Result<Self, ContractError> {
        let db_id = std::env::var("ASTRA_DB_ID")
            .map_err(|_| ContractError::DbError("Missing ASTRA_DB_ID".into()))?;
        let region = std::env::var("ASTRA_DB_REGION")
            .map_err(|_| ContractError::DbError("Missing ASTRA_DB_REGION".into()))?;
        let keyspace = std::env::var("ASTRA_DB_KEYSPACE")
            .map_err(|_| ContractError::DbError("Missing ASTRA_DB_KEYSPACE".into()))?;
        let token = std::env::var("ASTRA_DB_TOKEN")
            .map_err(|_| ContractError::DbError("Missing ASTRA_DB_TOKEN".into()))?;

        let base_url = format!(
            "https://{}-{}.apps.astra.datastax.com/api/rest/v2/keyspaces/{}",
            db_id, region, keyspace
        );

        println!("✅ AstraDB URL: {}", base_url);

        Ok(Self {
            client: Client::new(),
            base_url,
            token,
        })
    }

    // ========== SHOP ==========
    pub async fn verify_admin(&self, shop_id: &str, pin: &str) -> Result<Option<String>, ContractError> {
        let url = format!("{}/shops/{}", self.base_url, shop_id);
        
        let resp = self.client
            .get(&url)
            .header("X-Cassandra-Token", &self.token)
            .send()
            .await
            .map_err(|e| ContractError::DbError(format!("Request failed: {}", e)))?;

        if resp.status().as_u16() == 404 {
            return Ok(None);
        }

        let body: serde_json::Value = resp.json().await
            .map_err(|e| ContractError::DbError(format!("Parse failed: {}", e)))?;

        let data = &body["data"][0];
        let stored_pin = data["admin_pin"].as_str().unwrap_or("");
        let shop_name = data["shop_name"].as_str().unwrap_or("");

        if stored_pin == pin {
            Ok(Some(shop_name.to_string()))
        } else {
            Ok(None)
        }
    }

    // ========== GUEST ==========
    pub async fn upsert_guest(&self, shop_id: &str, guest_id: u64, name: &str) -> Result<(), ContractError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as i64;

        let url = format!("{}/guests", self.base_url);
        
        let payload = json!({
            "shop_id": shop_id,
            "guest_id": guest_id as i64,
            "guest_name": name,
            "created_at": now,
            "last_seen": now
        });

        self.client
            .post(&url)
            .header("X-Cassandra-Token", &self.token)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| ContractError::DbError(format!("Insert guest failed: {}", e)))?;

        Ok(())
    }

    pub async fn get_guests(&self, shop_id: &str) -> Result<Vec<Guest>, ContractError> {
        let url = format!("{}/guests?where={{\"shop_id\":{{\"$eq\":\"{}\"}}}}", self.base_url, shop_id);
        
        let resp = self.client
            .get(&url)
            .header("X-Cassandra-Token", &self.token)
            .send()
            .await
            .map_err(|e| ContractError::DbError(format!("Get guests failed: {}", e)))?;

        let body: serde_json::Value = resp.json().await
            .map_err(|e| ContractError::DbError(format!("Parse failed: {}", e)))?;

        let mut guests = Vec::new();
        if let Some(rows) = body["data"].as_array() {
            for row in rows {
                guests.push(Guest {
                    shop_id: row["shop_id"].as_str().unwrap_or("").to_string(),
                    guest_id: row["guest_id"].as_i64().unwrap_or(0) as u64,
                    guest_name: row["guest_name"].as_str().unwrap_or("").to_string(),
                    created_at: row["created_at"].as_i64().unwrap_or(0) as u64,
                    last_seen: row["last_seen"].as_i64().unwrap_or(0) as u64,
                });
            }
        }

        Ok(guests)
    }

    // ========== MESSAGE ==========
    pub async fn insert_message(&self, msg: &Message) -> Result<(), ContractError> {
        let url = format!("{}/messages", self.base_url);
        
        // Base64 mới (không deprecated)
        let content_base64 = BASE64.encode(&msg.content);
        
        let payload = json!({
            "shop_id": msg.shop_id,
            "guest_id": msg.guest_id as i64,
            "message_id": msg.message_id as i64,
            "sender_type": msg.sender_type,
            "content": content_base64,
            "timestamp_us": msg.timestamp_us as i64,
            "content_crc": msg.content_crc as i32
        });

        self.client
            .post(&url)
            .header("X-Cassandra-Token", &self.token)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| ContractError::DbError(format!("Insert message failed: {}", e)))?;

        Ok(())
    }

    pub async fn fetch_messages(
        &self,
        shop_id: &str,
        guest_id: u64,
        after_id: u64,
        limit: u32,
    ) -> Result<Vec<Message>, ContractError> {
        let url = format!(
            "{}/messages?where={{\"shop_id\":{{\"$eq\":\"{}\"}},\"guest_id\":{{\"$eq\":{}}},\"message_id\":{{\"$gt\":{}}}}}&page-size={}",
            self.base_url, shop_id, guest_id as i64, after_id as i64, limit
        );
        
        let resp = self.client
            .get(&url)
            .header("X-Cassandra-Token", &self.token)
            .send()
            .await
            .map_err(|e| ContractError::DbError(format!("Fetch messages failed: {}", e)))?;

        let body: serde_json::Value = resp.json().await
            .map_err(|e| ContractError::DbError(format!("Parse failed: {}", e)))?;

        let mut messages = Vec::new();
        if let Some(rows) = body["data"].as_array() {
            for row in rows {
                // Base64 decode mới
                let content_b64 = row["content"].as_str().unwrap_or("");
                let content_bytes = BASE64.decode(content_b64)
                    .map_err(|e| ContractError::DbError(format!("Base64 decode failed: {}", e)))?;

                messages.push(Message {
                    shop_id: row["shop_id"].as_str().unwrap_or("").to_string(),
                    guest_id: row["guest_id"].as_i64().unwrap_or(0) as u64,
                    message_id: row["message_id"].as_i64().unwrap_or(0) as u64,
                    sender_type: row["sender_type"].as_str().unwrap_or("").to_string(),
                    content: Bytes::from(content_bytes),
                    timestamp_us: row["timestamp_us"].as_i64().unwrap_or(0) as u64,
                    content_crc: row["content_crc"].as_i64().unwrap_or(0) as u32,
                });
            }
        }

        Ok(messages)
    }
}