use super::client::get_conn;
use super::types::RedisResult;

pub async fn hset_multi(key: &str, fields: &[(String, String)]) -> RedisResult<()> {
    if fields.is_empty() {
        return Ok(());
    }

    let mut conn = get_conn().await?;
    let mut args = Vec::with_capacity(1 + fields.len() * 2);
    args.push(key.to_string());

    for (field, value) in fields {
        args.push(field.clone());
        args.push(value.clone());
    }

    redis::cmd("HSET")
        .arg(&args)
        .query_async::<()>(&mut conn)
        .await?;

    Ok(())
}

pub async fn hmget_strings(key: &str, fields: &[String]) -> RedisResult<Vec<Option<String>>> {
    if fields.is_empty() {
        return Ok(Vec::new());
    }

    let mut conn = get_conn().await?;
    let mut args = Vec::with_capacity(1 + fields.len());
    args.push(key.to_string());
    args.extend(fields.iter().cloned());

    redis::cmd("HMGET").arg(&args).query_async(&mut conn).await
}
