use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast;

const HISTORY_CAPACITY: usize = 500;

#[derive(Clone, Debug)]
pub struct IndexerMintLogEntry {
    pub timestamp: DateTime<Utc>,
    pub source: String,
    pub mint: String,
    pub shards: usize,
    pub perf_us: u128,
    pub windows: Vec<String>,
    pub was_inserted: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct IndexerMintLogCounters {
    pub total: u64,
    pub pumpfun: u64,
    pub raydium: u64,
    pub other: u64,
}

static LOG_BUFFER: Lazy<RwLock<VecDeque<IndexerMintLogEntry>>> =
    Lazy::new(|| RwLock::new(VecDeque::with_capacity(HISTORY_CAPACITY)));
static LOG_CHANNEL: Lazy<broadcast::Sender<IndexerMintLogEntry>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(1024);
    tx
});

static TOTAL_EVENTS: AtomicU64 = AtomicU64::new(0);
static PUMPFUN_EVENTS: AtomicU64 = AtomicU64::new(0);
static RAYDIUM_EVENTS: AtomicU64 = AtomicU64::new(0);
static OTHER_EVENTS: AtomicU64 = AtomicU64::new(0);

pub fn record_indexer_mint_log(entry: IndexerMintLogEntry) {
    match entry.source.as_str() {
        "pumpfun.ws" => {
            PUMPFUN_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
        "raydium.ws" => {
            RAYDIUM_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            OTHER_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
    }
    TOTAL_EVENTS.fetch_add(1, Ordering::Relaxed);

    {
        let mut guard = LOG_BUFFER.write();
        if guard.len() == HISTORY_CAPACITY {
            guard.pop_front();
        }
        guard.push_back(entry.clone());
    }

    let _ = LOG_CHANNEL.send(entry);
}

pub fn subscribe_indexer_mint_logs() -> (
    Vec<IndexerMintLogEntry>,
    broadcast::Receiver<IndexerMintLogEntry>,
) {
    let history = {
        let guard = LOG_BUFFER.read();
        guard.iter().cloned().collect::<Vec<_>>()
    };
    (history, LOG_CHANNEL.subscribe())
}

pub fn indexer_mint_log_counters() -> IndexerMintLogCounters {
    IndexerMintLogCounters {
        total: TOTAL_EVENTS.load(Ordering::Relaxed),
        pumpfun: PUMPFUN_EVENTS.load(Ordering::Relaxed),
        raydium: RAYDIUM_EVENTS.load(Ordering::Relaxed),
        other: OTHER_EVENTS.load(Ordering::Relaxed),
    }
}
