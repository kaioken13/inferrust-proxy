# InferRust-Proxy 🦀⚡

A high-performance, asynchronous LLM inference sidecar proxy built in Rust. Designed to reduce GPU compute costs, decrease inference latency (p95/p99), and enforce granular token-budget limits for production AI workloads.

---

## 🏛️ Architecture Overview

`InferRust-Proxy` sits between client applications and downstream LLM inference backends (e.g., vLLM, Ollama, OpenAI API). It intercepts HTTP/gRPC traffic to perform zero-GPU cache resolution, lock-free token rate limiting, and automatic cloud failover.

[ Client Application ]
│
▼ (HTTP / gRPC)
┌──────────────────────────────────────────────────────────────┐
│                  InferRust-Proxy (Rust / Axum)               │
│                                                              │
│  ├── 1. Exact & Semantic Prompt Caching (moka / Redis)       │
│  ├── 2. Lock-free Token Bucket Rate Limiter (tokio)          │
│  ├── 3. In-Memory Token Counter (HuggingFace tokenizers)     │
│  └── 4. Stream Routing & Cloud Fallback                      │
└──────────────────────────────────────────────────────────────┘
│
├───► Local GPU Backend (vLLM / Ollama)
└───► Cloud Provider Fallback (OpenAI / Anthropic)

---

## ✨ Key Features

* **Sub-millisecond Prompt Caching:** Serves cached prompt responses directly from memory (`moka`) in < 2ms, bypassing GPU execution.
* **Token Budgeting & Rate Limiting:** Enforces real-time token and request limits per API key using non-blocking, lock-free primitives.
* **Stream Routing & Resiliency:** Seamlessly proxies streaming responses and routes requests to fallback cloud providers during local GPU degradation.
* **Native MLOps Observability:** Exposes Prometheus metrics out of the box (`/metrics`), tracking Time To First Token (TTFT), Tokens Per Second (TPS), and p99 latencies.
* **Minimal Memory Footprint:** Built with async Rust (`axum` + `tokio`), running in a lightweight distroless Docker image.

---

## 🛠️ Tech Stack

* **Language:** Rust (2021 Edition)
* **Async Runtime:** `tokio`
* **Web Framework:** `axum`
* **Caching:** `moka` (Concurrent in-memory cache)
* **Tokenization:** `tokenizers` (Hugging Face)
* **Observability:** `tracing`, `prometheus`
* **Containerization:** Docker (Distroless build), Kubernetes / k3s ready

---

## 🚀 Getting Started

### Prerequisites

* Rust 1.75+ toolchain
* A running local LLM backend (e.g., Ollama or vLLM)

### Running Locally

1. **Clone the repository:**
   ```bash
   git clone [https://github.com/your-username/inferrust-proxy.git](https://github.com/your-username/inferrust-proxy.git)
   cd inferrust-proxy
   ´´´

1. **Configure environment variables:**
   export BACKEND_URL="http://localhost:11434" # Ollama endpoint
   export RUST_LOG="inferrust_proxy=debug,info"

2. **Build and Run:**
   cargo run --release

3. **Send a test completion request:**
   curl -X POST [http://127.0.0.1:3000/v1/chat/completions](http://127.0.0.1:3000/v1/chat/completions) \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama3",
    "messages": [{"role": "user", "content": "Hello, world!"}]
  }'

📊 Benchmarks & Performance
Benchmarks comparing InferRust-Proxy against standard Python/FastAPI gateway solutions will be updated upon completion of Phase 4.

Cache Hit Latency: < 2ms (p99)

Idle Memory Consumption: ~15 MB RAM

📄 License
Distributed under the MIT License. See LICENSE for more information.
