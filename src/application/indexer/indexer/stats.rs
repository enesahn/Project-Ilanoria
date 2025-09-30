use super::shards::shards_map;
use crate::application::indexer::config::HASH_KEY;
use crate::infrastructure::database::{self as redis_infra, RedisResult};
use std::collections::HashSet;
use std::sync::Arc;

pub fn ram_index_stats() -> (usize, usize, usize, usize) {
    let map = shards_map();
    let mut unique_mints: HashSet<Arc<str>> = HashSet::new();
    let mut refs = 0usize;
    let mut bytes_keys = 0usize;
    let mut bytes_mints = 0usize;
    for e in map.iter() {
        bytes_keys += e.key().len();
        for m in e.value().iter() {
            refs += 1;
            bytes_mints += m.len();
            unique_mints.insert(m.clone());
        }
    }
    let shards = map.len();
    let uniq_mints = unique_mints.len();
    let approx = bytes_keys + bytes_mints;
    (shards, refs, uniq_mints, approx)
}

pub async fn redis_index_stats(redis_url: &str) -> RedisResult<(u64, usize, u64)> {
    redis_infra::ensure_initialized(redis_url).await?;

    let mut conn = redis_infra::get_connection(redis_url).await?;

    let hlen: u64 = redis::cmd("HLEN")
        .arg(HASH_KEY)
        .query_async(&mut conn)
        .await
        .unwrap_or(0);

    let mut cursor: u64 = 0;
    let mut unique_values = HashSet::new();

    loop {
        let res: (u64, Vec<String>) = redis::cmd("HSCAN")
            .arg(HASH_KEY)
            .arg(cursor)
            .arg("COUNT")
            .arg(2000)
            .query_async(&mut conn)
            .await?;

        cursor = res.0;
        let mut it = res.1.into_iter();

        while let Some(_field) = it.next() {
            if let Some(value) = it.next() {
                unique_values.insert(value);
            } else {
                break;
            }
        }

        if cursor == 0 {
            break;
        }
    }

    let mem_bytes: u64 = redis::cmd("MEMORY")
        .arg("USAGE")
        .arg(HASH_KEY)
        .query_async(&mut conn)
        .await
        .unwrap_or(0);

    Ok((hlen, unique_values.len(), mem_bytes))
}
