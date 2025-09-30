use super::types::UserData;
use crate::interfaces::bot::tasks::state;
use redis::aio::MultiplexedConnection;
use redis::{AsyncCommands, RedisResult};

fn get_user_key(chat_id: i64) -> String {
    format!("user_data:{}", chat_id)
}

pub async fn get_user_data(
    con: &mut MultiplexedConnection,
    chat_id: i64,
) -> RedisResult<Option<UserData>> {
    let key = get_user_key(chat_id);
    let data: Option<String> = con.get(key).await?;
    let parsed = data.and_then(|s| serde_json::from_str(&s).ok());
    if let Some(ref user_data) = parsed {
        state::sync_user_data(chat_id, user_data).await;
    }
    Ok(parsed)
}

pub async fn save_user_data(
    con: &mut MultiplexedConnection,
    chat_id: i64,
    user_data: &UserData,
) -> RedisResult<()> {
    let key = get_user_key(chat_id);
    let data = serde_json::to_string(user_data).unwrap();
    let _: () = con.set(key, data).await?;
    state::sync_user_data(chat_id, user_data).await;
    Ok(())
}

pub async fn get_all_user_ids(redis_url: &str) -> RedisResult<Vec<i64>> {
    let client = redis::Client::open(redis_url)?;
    let mut con = client.get_multiplexed_async_connection().await?;
    let mut cursor: u64 = 0;
    let mut ids = Vec::new();
    loop {
        let res: (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("user_data:*")
            .arg("COUNT")
            .arg(1000)
            .query_async(&mut con)
            .await?;
        cursor = res.0;
        for k in res.1 {
            if let Some(s) = k.strip_prefix("user_data:") {
                if let Ok(id) = s.parse::<i64>() {
                    ids.push(id);
                }
            }
        }
        if cursor == 0 {
            break;
        }
    }
    Ok(ids)
}
