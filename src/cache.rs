// src/cache.rs
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn generate_cache_key(payload: &Value) -> String {
    let model = payload["model"].as_str().unwrap_or("default");
    let messages = payload["messages"].to_string();
    let temperature = payload["temperature"].as_f64().unwrap_or(1.0);
    let max_tokens = payload["max_tokens"].as_u64().unwrap_or(2048);

    let canonical = format!(
        "{}|{}|{}|{}",
        model, messages, temperature, max_tokens
    );
    
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let result = hasher.finalize();
    
    hex::encode(result)
}