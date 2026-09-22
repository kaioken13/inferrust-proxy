// src/vectordb_client.rs
#[allow(clippy::result_large_err)]
pub mod proto {
    tonic::include_proto!("vectordb");
}

pub use proto::vector_service_client::VectorServiceClient;
pub use proto::{SearchRequest, SearchResponse};
