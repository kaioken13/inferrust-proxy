pub mod proto {
    tonic::include_proto!("vectordb");
}

pub use proto::vector_service_client::VectorServiceClient;
pub use proto::{SearchRequest, SearchResponse};
