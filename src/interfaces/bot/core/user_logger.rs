use lazy_static::lazy_static;
use redis::{AsyncCommands, Client as RedisClient, pipe};
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;

lazy_static! {
    static ref REDIS_CLIENT: Arc<Mutex<RedisClient>> = {
        let redis_url = env::var("REDIS_URL").expect("REDIS_URL must be set for user logger");
        Arc::new(Mutex::new(
            RedisClient::open(redis_url).expect("Failed to create Redis client for logger"),
        ))
    };
}

fn get_global_log_key(chat_id: i64) -> String {
    format!("logs:global:{}", chat_id)
}

fn get_tx_log_hash_key(chat_id: i64) -> String {
    format!("logs:txs:{}", chat_id)
}

fn get_ca_log_hash_key(chat_id: i64) -> String {
    format!("logs:ca_detection:{}", chat_id)
}

const LOG_MAXLEN: usize = 2000;

pub async fn log_to_user(chat_id: i64, level: &str, message: String) {
    let client = REDIS_CLIENT.lock().await;
    let mut con = match client.get_multiplexed_async_connection().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to get Redis connection for logging: {}", e);
            return;
        }
    };

    let key = get_global_log_key(chat_id);
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    let timestamped_message = format!("[{}] [{}] {}", timestamp, level, message);

    let console_log = format!("[USER_LOG:{}] {}", chat_id, timestamped_message);
    match level {
        "ERROR" => log::error!("{}", console_log),
        "WARN" => log::warn!("{}", console_log),
        "INFO" => log::info!("{}", console_log),
        _ => log::debug!("{}", console_log),
    }

    let mut p = pipe();
    p.cmd("LPUSH").arg(&key).arg(&timestamped_message).ignore();
    p.cmd("LTRIM")
        .arg(&key)
        .arg(0)
        .arg((LOG_MAXLEN as isize) - 1)
        .ignore();
    let _: Result<(), _> = p.query_async(&mut con).await;
}

pub async fn log_buffer_to_tx(chat_id: i64, signature: String, buffer: Vec<String>) {
    if signature.is_empty() || buffer.is_empty() {
        return;
    }
    let client = REDIS_CLIENT.lock().await;
    let mut con = match client.get_multiplexed_async_connection().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to get Redis connection for tx logging: {}", e);
            return;
        }
    };

    let key = get_tx_log_hash_key(chat_id);
    let field = signature;
    let new_logs_json = serde_json::to_string(&buffer).unwrap();
    let _: Result<(), _> = con.hset(key, field, new_logs_json).await;
}

pub async fn log_buffer_to_ca_detection(chat_id: i64, mint: String, buffer: Vec<String>) {
    if mint.is_empty() || buffer.is_empty() {
        return;
    }
    let client = REDIS_CLIENT.lock().await;
    let mut con = match client.get_multiplexed_async_connection().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "Failed to get Redis connection for CA detection logging: {}",
                e
            );
            return;
        }
    };

    let key = get_ca_log_hash_key(chat_id);
    let field = mint;
    let new_logs_json = serde_json::to_string(&buffer).unwrap();
    let _: Result<(), _> = con.hset(key, field, new_logs_json).await;
}

#[macro_export]
macro_rules! user_log {
    ($level:ident, $chat_id:expr, $($arg:tt)*) => {
        {
            let message = format!($($arg)*);
            let level_str = stringify!($level);
            tokio::spawn(crate::interfaces::bot::user_logger::log_to_user($chat_id, level_str, message));
        }
    };
}

#[macro_export]
macro_rules! info {
    ($chat_id:expr, $($arg:tt)*) => {
        crate::user_log!(INFO, $chat_id, $($arg)*)
    };
}

#[macro_export]
macro_rules! warn {
    ($chat_id:expr, $($arg:tt)*) => {
        crate::user_log!(WARN, $chat_id, $($arg)*)
    };
}

#[macro_export]
macro_rules! error {
    ($chat_id:expr, $($arg:tt)*) => {
        crate::user_log!(ERROR, $chat_id, $($arg)*)
    };
}

#[macro_export]
macro_rules! debug {
    ($chat_id:expr, $($arg:tt)*) => {
        crate::user_log!(DEBUG, $chat_id, $($arg)*)
    };
}
