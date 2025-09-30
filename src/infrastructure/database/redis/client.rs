use super::types::RedisResult;
use redis::Client;
use redis::aio::MultiplexedConnection;
use std::sync::OnceLock;

static CLIENT: OnceLock<Client> = OnceLock::new();

pub async fn init(redis_url: &str) -> RedisResult<()> {
    let client = Client::open(redis_url)?;
    CLIENT.set(client).map_err(|_| {
        redis::RedisError::from((
            redis::ErrorKind::IoError,
            "Redis client already initialized",
        ))
    })?;
    Ok(())
}

pub async fn ensure_initialized(redis_url: &str) -> RedisResult<()> {
    if CLIENT.get().is_none() {
        init(redis_url).await?;
    }
    Ok(())
}

pub(super) async fn get_conn() -> RedisResult<MultiplexedConnection> {
    let client = CLIENT.get().ok_or_else(|| {
        redis::RedisError::from((redis::ErrorKind::IoError, "Redis client not initialized"))
    })?;
    client.get_multiplexed_async_connection().await
}

pub async fn get_connection(redis_url: &str) -> RedisResult<MultiplexedConnection> {
    ensure_initialized(redis_url).await?;
    get_conn().await
}
