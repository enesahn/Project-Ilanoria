pub mod client;
pub mod commands;
pub mod types;

pub use client::{ensure_initialized, get_connection};
pub use commands::{hmget_strings, hset_multi};
pub use types::RedisResult;
