// src/cache.rs
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn generate_cache_key(payload: &Value) -> String {
    let mut hasher = Sha256::new();

    // We feed the hasher incrementally to avoid string formatting on the heap
    if let Some(model) = payload["model"].as_str() {
        hasher.update(model.as_bytes());
    } else {
        hasher.update(b"default");
    }
    hasher.update(b"|");

    // Instead of serializing the entire array to a String, we pass the raw JSON bytes directly
    if let Ok(messages_bytes) = serde_json::to_vec(&payload["messages"]) {
        hasher.update(&messages_bytes);
    }
    hasher.update(b"|");

    let temperature = payload["temperature"].as_f64().unwrap_or(1.0);
    hasher.update(temperature.to_be_bytes());
    hasher.update(b"|");

    let max_tokens = payload["max_tokens"].as_u64().unwrap_or(2048);
    hasher.update(max_tokens.to_be_bytes());

    let result = hasher.finalize();
    hex::encode(result)
}