use crate::application::indexer::config::{HASH_KEY, MAX_PER_SHARD};
use crate::infrastructure::database::{self as redis_infra, RedisResult};
use dashmap::DashMap;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

static SHARDS: OnceLock<DashMap<String, SmallVec<[Arc<str>; 8]>>> = OnceLock::new();

pub(crate) fn shards_map() -> &'static DashMap<String, SmallVec<[Arc<str>; 8]>> {
    SHARDS.get_or_init(DashMap::new)
}

fn split_mint_into_parts(mint: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = mint.as_bytes();
    let mut i = 0usize;
    while i + 7 <= bytes.len() {
        out.push(&mint[i..i + 7]);
        i += 7;
    }
    out
}

pub async fn preload_from_redis(redis_url: &str) -> RedisResult<usize> {
    let t0 = Instant::now();
    redis_infra::ensure_initialized(redis_url).await?;

    let mut conn = redis_infra::get_connection(redis_url).await?;
    let mut cursor: u64 = 0;
    let mut fields_loaded = 0usize;

    loop {
        let res: (u64, Vec<String>) = redis::cmd("HSCAN")
            .arg(HASH_KEY)
            .arg(cursor)
            .arg("COUNT")
            .arg(1000)
            .query_async(&mut conn)
            .await?;

        cursor = res.0;
        let mut it = res.1.into_iter();

        while let Some(field) = it.next() {
            let mint = match it.next() {
                Some(v) => v,
                None => break,
            };

            let arc_mint: Arc<str> = Arc::from(mint);
            let map = shards_map();
            let mut entry = map.entry(field).or_insert_with(SmallVec::new);

            if entry.len() >= MAX_PER_SHARD {
                entry.remove(0);
            }
            entry.push(arc_mint);
            fields_loaded += 1;
        }

        if cursor == 0 {
            break;
        }
    }

    let us = t0.elapsed().as_micros();
    log::info!("pp.preload fields={} perf.us={}", fields_loaded, us);

    Ok(fields_loaded)
}

pub async fn index_mint_shards(redis_url: &str, mint: &str) -> RedisResult<()> {
    let t0 = Instant::now();
    let parts = split_mint_into_parts(mint);

    if parts.is_empty() {
        return Ok(());
    }

    let arc_mint: Arc<str> = Arc::from(mint.to_string());
    let map = shards_map();

    for part in &parts {
        let key = part.to_string();
        let mut entry = map.entry(key).or_insert_with(SmallVec::new);

        let already_exists = entry
            .iter()
            .any(|m| Arc::ptr_eq(m, &arc_mint) || m.as_ref() == arc_mint.as_ref());

        if !already_exists {
            if entry.len() >= MAX_PER_SHARD {
                entry.remove(0);
            }
            entry.push(arc_mint.clone());
        }
    }

    let us = t0.elapsed().as_micros();
    log::info!("pp.index shards={} perf.us={}", parts.len(), us);

    redis_infra::ensure_initialized(redis_url).await?;

    let fields: Vec<(String, String)> = parts
        .iter()
        .map(|p| (p.to_string(), mint.to_string()))
        .collect();

    redis_infra::hset_multi(HASH_KEY, &fields).await?;

    Ok(())
}

fn is_base58_byte(b: u8) -> bool {
    matches!(b,
        b'1'..=b'9' |
        b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' |
        b'a'..=b'k' | b'm'..=b'z'
    )
}

fn extract_unique_windows_7(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let mut out: Vec<&str> = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if is_base58_byte(b[i]) {
            let run_start = i;
            while i < b.len() && is_base58_byte(b[i]) {
                i += 1;
            }
            if i > run_start {
                let run = &text[run_start..i];
                let rb = run.as_bytes();
                if rb.len() >= 7 {
                    let mut j = 0usize;
                    while j + 7 <= rb.len() {
                        out.push(&run[j..j + 7]);
                        j += 1;
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    out.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    out.dedup_by(|a, b| a.as_bytes() == b.as_bytes());
    out
}

pub async fn threshold_detect_from_text(
    redis_url: &str,
    text: &str,
    threshold: usize,
) -> RedisResult<Option<String>> {
    let t0 = Instant::now();
    let windows = extract_unique_windows_7(text);

    if windows.is_empty() {
        return Ok(None);
    }

    let map = shards_map();
    let mut counts: HashMap<Arc<str>, usize> = HashMap::new();
    let mut total_hits = 0usize;

    for window in &windows {
        if let Some(entry) = map.get(*window) {
            for mint in entry.iter() {
                total_hits += 1;
                *counts.entry(mint.clone()).or_insert(0) += 1;
            }
        }
    }

    if total_hits == 0 {
        redis_infra::ensure_initialized(redis_url).await?;
        let fields: Vec<String> = windows.iter().map(|s| s.to_string()).collect();
        let values = redis_infra::hmget_strings(HASH_KEY, &fields).await?;

        let mut hit_counts: HashMap<String, usize> = HashMap::new();
        let mut redis_hits = 0usize;

        for value in values.into_iter().flatten() {
            redis_hits += 1;
            *hit_counts.entry(value).or_insert(0) += 1;
        }

        let best = hit_counts.into_iter().max_by_key(|(_, count)| *count);

        let us = t0.elapsed().as_micros();

        if let Some((mint, cnt)) = best {
            log::info!(
                "pp.detect_redis hits={} uniq_windows={} best_cnt={} perf.us={}",
                redis_hits,
                windows.len(),
                cnt,
                us
            );

            if cnt >= threshold {
                return Ok(Some(mint));
            }
        } else {
            log::info!(
                "pp.detect_redis hits=0 uniq_windows={} perf.us={}",
                windows.len(),
                us
            );
        }

        return Ok(None);
    }

    let best = counts.into_iter().max_by_key(|(_, count)| *count);

    let us = t0.elapsed().as_micros();

    if let Some((mint, cnt)) = best {
        log::info!(
            "pp.detect_inmem hits={} uniq_windows={} best_cnt={} perf.us={}",
            total_hits,
            windows.len(),
            cnt,
            us
        );

        if cnt >= threshold {
            return Ok(Some(mint.to_string()));
        }
    } else {
        log::info!(
            "pp.detect_inmem hits=0 uniq_windows={} perf.us={}",
            windows.len(),
            us
        );
    }

    Ok(None)
}
