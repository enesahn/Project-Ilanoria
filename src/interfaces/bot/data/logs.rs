use redis::{AsyncCommands, RedisResult};

fn get_global_log_key(chat_id: i64) -> String {
    format!("logs:global:{}", chat_id)
}

fn get_tx_log_hash_key(chat_id: i64) -> String {
    format!("logs:txs:{}", chat_id)
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

pub async fn get_user_tx_signatures(redis_url: &str, chat_id: i64) -> RedisResult<Vec<String>> {
    let client = redis::Client::open(redis_url)?;
    let mut con = client.get_multiplexed_async_connection().await?;
    let key = get_tx_log_hash_key(chat_id);
    let mut cursor: u64 = 0;
    let mut out = Vec::new();
    loop {
        let res: (u64, Vec<String>) = redis::cmd("HSCAN")
            .arg(&key)
            .arg(cursor)
            .arg("COUNT")
            .arg(1000)
            .query_async(&mut con)
            .await?;
        cursor = res.0;
        let fields = res.1;
        let mut it = fields.into_iter();
        while let Some(field) = it.next() {
            let _val = it.next();
            out.push(field);
        }
        if cursor == 0 {
            break;
        }
    }
    Ok(out)
}

pub async fn get_tx_logs(
    redis_url: &str,
    chat_id: i64,
    signature: &str,
) -> RedisResult<Vec<String>> {
    let client = redis::Client::open(redis_url)?;
    let mut con = client.get_multiplexed_async_connection().await?;
    let key = get_tx_log_hash_key(chat_id);
    let logs_json: Option<String> = con.hget(key, signature).await?;
    match logs_json {
        Some(json_str) => {
            let logs: Vec<String> = serde_json::from_str(&json_str).unwrap_or_else(|_| vec![]);
            Ok(logs)
        }
        None => Ok(vec![]),
    }
}

pub async fn clear_user_logs(redis_url: &str, chat_id: i64) -> RedisResult<()> {
    let client = redis::Client::open(redis_url)?;
    let mut con = client.get_multiplexed_async_connection().await?;
    let key = get_global_log_key(chat_id);
    let _: () = redis::cmd("UNLINK").arg(key).query_async(&mut con).await?;
    Ok(())
}

pub async fn clear_user_tx_log(redis_url: &str, chat_id: i64, signature: &str) -> RedisResult<()> {
    let client = redis::Client::open(redis_url)?;
    let mut con = client.get_multiplexed_async_connection().await?;
    let key = get_tx_log_hash_key(chat_id);
    let _: () = con.hdel(key, signature).await?;
    Ok(())
}
