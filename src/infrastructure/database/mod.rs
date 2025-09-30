pub mod redis;

pub use redis::{RedisResult, ensure_initialized, get_connection, hmget_strings, hset_multi};
