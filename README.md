# 🚀 inferrust-proxy (v3 - Ultimate System Design & LLMOps Edition)

A high-performance, resilient AI inference gateway and reverse proxy written in **Rust** [1]. Designed to scale LLM workloads globally, mitigate tail latency using dynamic, TTFT-focused **Hedged Requests**, and provide a production-ready, highly concurrent middleware suite. 

`inferrust-proxy` sits seamlessly between your application and a cluster of downstream LLM inference replicas (such as vLLM, SGLang, or Ollama) [1]. It bridges the gap between low-level network performance and hardware-conscious GPU serving physics.

---

## 🏗️ System Architecture

The following sequence diagram illustrates the end-to-end lifecycle of a client request under high concurrency, highlighting cache lookups, distributed rate limiting, the dynamic timeout, and the asynchronous race between the primary and hedged requests [1].

```mermaid
sequenceDiagram
    autonumber
    actor Client
    participant Proxy as inferrust-proxy (Rust)
    participant Redis as Redis Sentinel / Cluster
    participant Moka as Local Moka Cache
    participant GPU_A as Inference GPU Replica A
    participant GPU_B as Inference GPU Replica B (Hedge)

    Client->>Proxy: POST /v1/chat/completions (with optional Auth header)
    
    Note over Proxy: 1. Extract Client IP & Check Rate Limits
    Proxy->>Redis: Atomic INCR + EXPIRE pipeline
    Redis-->>Proxy: Return counter (Fail-open on timeout/error)
    
    alt Rate Limit Exceeded
        Proxy-->>Client: 429 Too Many Requests
    else Allowed
        Note over Proxy: 2. Query Memory Cache (Fast Path)
        Proxy->>Moka: Lookup SHA-256 Incremental Key
        alt Cache HIT (Zero-Copy)
            Moka-->>Proxy: Return cached payload
            Proxy-->>Client: 200 OK (Immediate SSE Stream or JSON)
        else Cache MISS
            Note over Proxy: 3. Dynamic p95 Timeout Calculator
            Proxy->>Proxy: Calculate p95 sliding window (latencies)
            
            Note over Proxy: 4. Round-Robin Node Selection
            Proxy->>GPU_A: Dispatch Primary Request A (Idempotency UUID v4)
            
            alt Response A arrives before dynamic p95 timeout
                GPU_A-->>Proxy: 200 OK (Headers/Prefill finished)
                Note over Proxy: Fastest replica wins the race!
            else p95 Timeout Elapsed (Hedge Triggered)
                Note over Proxy: Tail latency detected!
                Proxy->>GPU_B: Dispatch Hedged Request B (Same Idempotency UUID)
                
                par Wait for A
                    GPU_A-->>Proxy: Response A complete
                and Wait for B
                    GPU_B-->>Proxy: Response B complete (Hedge won!)
                end
                
                Note over Proxy: Drop slower request, preserving resources
            end

            alt Stream Mode (is_stream = true)
                Proxy-->>Client: 200 OK (text/event-stream)
                Note over Proxy: Background worker intercepts chunks via MPSC Channel
                Proxy->>Moka: Asynchronously Cache full SSE string (Write-Behind)
            else Non-Stream Mode
                Proxy->>Proxy: Synchronous Tokenization (spawn_blocking)
                Proxy->>Moka: Insert Cache Entry
                Proxy-->>Client: 200 OK (application/json)
            end
        end
    end
```

---

## ✨ Core & Distributed Features

### 1. Dynamic Tail Latency Mitigation (Hedged Requests)
In large-scale distributed systems, tail latency (p95/p99) is highly volatile due to queue delays, scheduling noise, or hardware thermal throttling. Inspired by Google's landmark paper, **"The Tail at Scale"** [1], `inferrust-proxy` dynamically measures historical latencies and calculates a sliding p95 timeout [1].
*   **TTFT-Focused Hedging:** On streaming requests, the hedge timer monitors the **Time to First Token (TTFT)** [1]. Because LLM engines return HTTP headers immediately after the **Prefill** phase is completed, the proxy assesses the computational health of the GPU before the sequential, time-consuming **Decode** phase starts.
*   **Zero-Overhead Resolution:** Whichever backend responds first wins the race; the slower backend's future is dropped, avoiding network and memory buildup [1].

### 2. High-Performance SSE Caching (Write-Behind Pattern)
Caching real-time streams typically introduces overhead or user-facing latency. `inferrust-proxy` solves this with a **Write-Behind** architecture [1]:
*   As chunks arrive from the inference backend, they are sent immediately to the client through a non-blocking `tokio::sync::mpsc` channel [1].
*   Simultaneously, an asynchronous background task (`tokio::spawn`) aggregates these chunks in memory [1].
*   Upon successful stream completion, the fully consolidated response is cached asynchronously with `moka-rs`, avoiding any performance penalty on the critical request path [1].

### 3. Distributed, Fail-Open Rate Limiting
To prevent API abuse and brute-force token exhaustion, the proxy implements an IP-isolated rate limiter [1]:
*   **IP Isolation:** Rates are tracked individually per client IP (`rate_limit:<ip>`) [1]. The proxy safely identifies traffic origins, inspecting standard headers (`X-Forwarded-For` or `CF-Connecting-IP`) with a dynamic fallback to the physical TCP socket (`ConnectInfo`) to prevent global proxy lockouts.
*   **Redis Pipeline Atomicity:** Counter increments (`INCR`) and sliding-window expirations (`EXPIRE`) are grouped atomically using transactional Redis pipelines (`redis::pipe().atomic()`) [1].
*   **Fail-Open Design:** Modeled after Stripe's API scaling practices [1], if the Redis cluster undergoes network partitions, failovers, or downtime, the proxy logs the failure, issues telemetry alerts, and **fails open**, maintaining core service availability [1].

### 4. Idempotency & Connection Pool Optimization
*   **Idempotency Propagation:** Every outgoing request is injected with a unique `Idempotency-Key` (UUID v4) [1]. Inference engines (like vLLM or SGLang) can detect redundant hedged requests and avoid duplicate state modifications or unnecessary GPU compute [1].
*   **Warm TCP Pool:** Optimizes the downstream `reqwest` HTTP client with a keepalive duration of 60 seconds and maintains warm idle connections to avoid the heavy cost of TLS handshakes on every prompt [1].

### 5. Zero-Copy Cache Hits (Local Moka & Remote Redis)
The fast path for cache hits completely avoids CPU compute overhead:
*   **Unified Fast Path:** Both bulk batch completions (Non-Stream) and interactive flows (SSE Stream) are resolved identically during a cache hit.
*   **Zero-Copy:** Serialized JSON and SSE payloads are retrieved as raw data and wrapped directly in `axum::body::Body::from(cached_data)`. This bypasses expensive parsing, conversion into intermediate Rust types (`serde_json::Value`), and re-serialization.

---

## 🛠️ Tech Stack & Key Crates

*   **Framework:** `axum` (extremely fast, routing, and state sharing) [1]
*   **Runtime:** `tokio` (asynchronous event loop, channels, multi-threading) [1]
*   **Caching:** `moka` (concurrent, high-cache-hit-rate local memory cache) [1]
*   **Database Client:** `redis` (multiplexed async connection pooling) [1]
*   **Tokenization:** `tokenizers` (Hugging Face's industrial tokenizer for calculating prompt lengths) [1]
*   **Serialization:** `serde`, `serde_json`, `bincode` [1]
*   **Resilience & Hashing:** `uuid` (idempotency) [1], `sha2` (cache key generation) [1], `hex` [1]

---

## 🧠 LLM Inference Engineering & Trade-offs (LLMOps)

This proxy was designed based on the architectural principles of industry-leading inference platforms (drawing from engineering research by vLLM, Cloudflare, and Ray Serve).

### 1. Prefill vs. Decode (The Physics of Transformers)
Text generation in LLMs has two completely distinct hardware bottlenecks:
*   **Prefill (Prompt Processing):** The model processes all input tokens in parallel using matrix-matrix multiplication (GEMMs). This phase is highly **compute-bound** (limited by the processing power of GPU Tensor Cores) and determines the **TTFT (Time to First Token)**.
*   **Decode (Token Generation):** The model generates new tokens sequentially and autoregressively. Each step requires loading all model weights and the historical **KV Cache** from physical GPU memory (HBM) to the execution units. This phase is highly **memory-bandwidth-bound** and defines the **ITL (Inter-Token Latency)**.

*By focusing our Hedging mechanism strictly on the resolution of the HTTP connection headers (the end of the Prefill phase), the proxy mitigates tail latencies caused by GPU compute queues immediately at the first token.*

### 2. The Impact of Round-Robin on KV Cache (vLLM PagedAttention)
Modern LLM engines like vLLM use the **PagedAttention** algorithm to manage **KV Cache** in physical GPU memory as dynamic, non-contiguous blocks. This enables automatic **Prefix Caching** (instant reuse of token blocks representing system instructions or earlier turns of a chat session).
*   **The Round-Robin Trade-off:** The pure Round-Robin distribution algorithm used in this proxy handles basic load balancing but can break session stickiness for multi-turn chats. Rotational routing among different GPU replicas leads to artificial prefix cache misses, forcing expensive redundant prefill computations on the GPU.
*   **Production Vision:** In large-scale systems, this load balancer should evolve to use **Prefix-Aware Routing** via Consistent Hashing (such as the vLLM Router or Ray Serve Custom Routing). This guarantees that requests sharing the same context prefix are routed to the same physical GPU instance, maximizing cache hits.

### 3. Preventing Thread Blocking with CPU-Bound Tasks (Tokenizer)
The asynchronous Tokio runtime relies on cooperative scheduling among worker threads. Converting raw strings to numerical token vectors (using the Hugging Face Tokenizer) is a highly mathematical, CPU-bound operation.
*   Running synchronous tokenization directly on the main Tokio threads for massive prompts (containing tens of thousands of tokens) temporarily blocks the event loop. Under heavy load, this stalls new network packets and increases tail latency for concurrent active streams.
*   **Recommended Mitigation:** Wrap synchronous tokenizer calls in Tokio's blocking thread pool using `tokio::task::spawn_blocking`:
    ```rust
    let prompt_tokens = tokio::task::spawn_blocking(move || {
        tokenizer.encode(full_text, false)
            .map(|encoded| encoded.get_ids().len())
            .unwrap_or(0)
    }).await?;
    ```

---

## 🚀 Getting Started & Configuration

### Prerequisites
*   **Rust 1.75+** installed on your system [1].
*   An active **Redis** database running locally on `127.0.0.1:6379` (default) [1].
*   The tokenizer config file for your target LLM saved as **`tokenizer.json`** in the project's root directory [1].

### Installation & Setup

1.  Clone the repository [1]:
    ```bash
    git clone https://github.com/your-username/inferrust-proxy.git
    cd inferrust-proxy
    ```

2.  Ensure Redis is running [1]:
    ```bash
    redis-server
    ```

3.  Configure environment variables and run the proxy [1]:
    ```bash
    # Define backend inference engine URLs (e.g., vLLM or Ollama replicas)
    export BACKEND_URLS="http://localhost:11434,http://localhost:11435"
    export RUST_LOG="inferrust_proxy=debug,info"
    export PORT="3000"

    cargo run --release
    ```

4.  Test the gateway using standard OpenAI-compatible cURL commands or SDKs [1]:
    ```bash
    curl -X POST http://localhost:3000/v1/chat/completions \
      -H "Content-Type: application/json" \
      -d '{
        "model": "llama3",
        "messages": [{"role": "user", "content": "Hello! Explain briefly how hedged requests work."}],
        "stream": true
      }'
    ```

---

## 📈 Theoretical Background & MLOps Papers

The architecture of `inferrust-proxy` is heavily grounded in industry-standard research on distributed systems and high-throughput LLM serving:

*   **vLLM & PagedAttention:** *vLLM: Easy, Fast, and Cheap LLM Serving with PagedAttention* (Woosuk Kwon et al., UC Berkeley). Explains physical-to-logical GPU memory block mapping and KV Cache optimization techniques.
*   **The Tail at Scale [1]:** *The Tail at Scale* (Jeffrey Dean and Luiz André Barroso, Google). The foundational systems paper detailing tail latency amplification and request hedging strategies.
*   **Next-Level Inference:** *Next-Level Inference: Why Your Single-Node vLLM Setup Needs Prefill-Decode Disaggregation | vLLM Blog*. Investigates decoupling CPU-intensive prefill servers from memory-bandwidth-limited decode workers.
*   **HyGen Online-Offline Co-location:** *HyGen: Efficient LLM Serving via Elastic Online-Offline Request Co-location* (2501.14808v2.pdf). Details execution and scheduling queues managing priority and Service Level Objectives (SLOs) under multi-tenant workloads.
*   **Pingora Architecture:** *How we built Pingora, the proxy that connects Cloudflare to the Internet*. Demonstrates building memory-safe, ultra-low latency gateways with extremely efficient network resource multiplexing in Rust.

---

This project showcases how modern Rust systems development combined with hardware-conscious GPU inference principles can optimize the operational and cost efficiency of production AI infrastructure at scale.
