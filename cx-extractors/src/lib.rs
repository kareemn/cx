pub mod database;
pub mod dockerfile;
pub mod envvar;
pub mod grammars;
pub mod grpc;
pub mod grpc_client;
pub mod grpc_server;
pub mod helm;
pub mod k8s;
pub mod messagequeue;
pub mod openapi;
pub mod pipeline;
pub mod proto;
pub mod rest;
pub mod treesitter;
pub mod universal;

pub use pipeline::PipelineError;
pub type Result<T> = std::result::Result<T, PipelineError>;
