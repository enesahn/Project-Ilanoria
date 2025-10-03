use redis::RedisResult;

fn get_global_log_key(chat_id: i64) -> String {
    format!("logs:global:{}", chat_id)
}

pub async fn get_user_logs(redis_url: &str, chat_id: i64) -> RedisResult<Vec<String>> {
    let client = redis::Client::open(redis_url)?;
    let mut con = client.get_multiplexed_async_connection().await?;
    let key = get_global_log_key(chat_id);
    let logs: Vec<String> = redis::cmd("LRANGE")
        .arg(key)
        .arg(0)
        .arg(-1)
        .query_async(&mut con)
        .await?;
    Ok(logs)
}

pub async fn clear_user_logs(redis_url: &str, chat_id: i64) -> RedisResult<()> {
    let client = redis::Client::open(redis_url)?;
    let mut con = client.get_multiplexed_async_connection().await?;
    let key = get_global_log_key(chat_id);
    let _: () = redis::cmd("UNLINK").arg(key).query_async(&mut con).await?;
    Ok(())
}
