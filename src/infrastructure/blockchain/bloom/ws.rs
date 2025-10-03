use crate::{
    ACTIVE_BLOOM_SWAPS, BLOOM_WS_CONNECTION, BloomBuyAck, BloomSwapTracker, PENDING_BLOOM_RESPONSES,
};
use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use serde::Deserialize;
use std::convert::TryFrom;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::time::{Duration, MissedTickBehavior, sleep};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use url::Url;

const BLOOM_WS_HOST: &str = "ws.bloom-ext.app";
const BLOOM_EXTENSION_ORIGIN: &str = "chrome-extension://akdiolpnpplhaoedmednpobkhmkophmg";
const BLOOM_WS_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0";

enum ConnectOutcome {
    Connected,
    Skipped,
}

struct BloomAuthData {
    token: Option<String>,
    expires_at: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct BloomWsMessage {
    id: Option<String>,
    status: Option<u8>,
    side: Option<String>,
    token_name: Option<String>,
    token_address: Option<String>,
    address: Option<String>,
    mint: Option<String>,
    wallet_address: Option<String>,
    tx_hash: Option<String>,
    error: Option<String>,
    error_code: Option<i64>,
}

pub async fn run_bloom_ws_listener() {
    let mut backoff_ms = 1000u64;
    tokio::spawn(async {
        cleanup_stale_swaps().await;
    });
    loop {
        match connect_once().await {
            Ok(ConnectOutcome::Connected) => {
                backoff_ms = 1000;
            }
            Ok(ConnectOutcome::Skipped) => {
                backoff_ms = 30000;
            }
            Err(err) => {
                log::error!("bloom_ws: connection attempt failed err=\"{}\"", err);
                mark_ws_offline();
                backoff_ms = (backoff_ms * 2).min(30000);
            }
        }
        sleep(Duration::from_millis(backoff_ms)).await;
    }
}

async fn connect_once() -> Result<ConnectOutcome> {
    let BloomAuthData { token, expires_at } = load_bloom_auth_data();
    let token = match token.map(|value| value.trim().to_string()) {
        Some(value) if !value.is_empty() => value,
        _ => {
            log::info!("bloom_ws: connection skipped: invalid or expired auth token");
            mark_ws_offline();
            return Ok(ConnectOutcome::Skipped);
        }
    };
    if let Some(expires_at) = expires_at {
        let now_ms = current_time_millis()?;
        if now_ms > expires_at {
            log::info!("bloom_ws: connection skipped: invalid or expired auth token");
            mark_ws_offline();
            return Ok(ConnectOutcome::Skipped);
        }
    }
    let url = match Url::parse(&format!("wss://{}?{}", BLOOM_WS_HOST, token)) {
        Ok(parsed) => parsed,
        Err(err) => {
            mark_ws_offline();
            return Err(err.into());
        }
    };
    log::info!("bloom_ws: Attempting WebSocket connection...");
    let mut key_bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut key_bytes);
    let websocket_key = BASE64_STANDARD.encode(key_bytes);

    let request = Request::builder()
        .method("GET")
        .uri(url.as_str())
        .header("Host", BLOOM_WS_HOST)
        .header("Origin", BLOOM_EXTENSION_ORIGIN)
        .header("User-Agent", BLOOM_WS_USER_AGENT)
        .header("Pragma", "no-cache")
        .header("Cache-Control", "no-cache")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", websocket_key)
        .header(
            "Sec-WebSocket-Extensions",
            "permessage-deflate; client_max_window_bits",
        )
        .body(())?;
    let (mut ws_stream, _) = match connect_async(request).await {
        Ok(parts) => parts,
        Err(err) => {
            mark_ws_offline();
            return Err(err.into());
        }
    };
    log::info!("bloom_ws: connected");
    mark_ws_online();

    let mut keepalive = tokio::time::interval(Duration::from_secs(20));
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        handle_message_text(&text);
                    }
                    Some(Ok(Message::Binary(data))) => {
                        let len = data.len();
                        if let Ok(text) = String::from_utf8(data) {
                            handle_message_text(&text);
                        } else {
                            log::warn!(
                                "bloom_ws: binary message not valid utf8 len={}",
                                len
                            );
                        }
                    }
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Ok(Message::Ping(payload))) => {
                        if let Err(err) = ws_stream.send(Message::Pong(payload)).await {
                            mark_ws_offline();
                            return Err(err.into());
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        log_close_frame(frame);
                        mark_ws_offline();
                        break;
                    }
                    Some(Err(err)) => {
                        mark_ws_offline();
                        return Err(err.into());
                    }
                    None => {
                        log::warn!("bloom_ws: stream ended");
                        mark_ws_offline();
                        break;
                    }
                }
            }
            _ = keepalive.tick() => {
                if let Err(err) = ws_stream.send(Message::Text("keepalive".to_string())).await {
                    mark_ws_offline();
                    return Err(err.into());
                }
            }
        }
    }

    mark_ws_offline();
    Ok(ConnectOutcome::Connected)
}

fn mark_ws_online() {
    let mut state = BLOOM_WS_CONNECTION.lock();
    state.is_connected = true;
    state.last_success_at = Some(SystemTime::now());
}

fn mark_ws_offline() {
    let mut state = BLOOM_WS_CONNECTION.lock();
    state.is_connected = false;
}

fn handle_message_text(text: &str) {
    if text.trim().is_empty() || text.trim().eq_ignore_ascii_case("keepalive") {
        return;
    }
    match serde_json::from_str::<BloomWsMessage>(text) {
        Ok(message) => process_ws_message(message),
        Err(err) => {
            log::warn!(
                "bloom_ws: failed parsing message err=\"{}\" text={} ",
                err,
                text
            );
        }
    }
}

fn process_ws_message(mut message: BloomWsMessage) {
    let id = match message.id.clone() {
        Some(id) => id,
        None => {
            log::warn!("bloom_ws: missing id message={:?}", message);
            return;
        }
    };
    let status = match message.status {
        Some(status) => status,
        None => {
            log::warn!("bloom_ws: missing status id={}", id);
            return;
        }
    };
    message.status = Some(status);

    let token_name = message.token_name.clone();
    let tx_hash = message.tx_hash.clone();
    let error = message.error.clone();

    if status == 0 {
        let exists = ACTIVE_BLOOM_SWAPS.lock().contains_key(&id);
        if exists {
            log::debug!("bloom_ws: pending swap id={} side={:?}", id, message.side);
        } else {
            log::debug!(
                "bloom_ws: pending swap ignored id={} side={:?}",
                id,
                message.side
            );
        }
        return;
    }

    let tracker_option = {
        let mut swaps = ACTIVE_BLOOM_SWAPS.lock();
        swaps.remove(&id)
    };

    let tracker = match tracker_option {
        Some(tracker) => tracker,
        None => {
            log::warn!("bloom_ws: no tracker for id={} status={}", id, status);
            return;
        }
    };

    match status {
        1 => handle_success(&id, &tracker, token_name, tx_hash),
        _ if status >= 2 => handle_failure(&id, &tracker, status, error),
        _ => {
            log::warn!("bloom_ws: unhandled status={} id={}", status, id);
        }
    }
}

fn current_time_millis() -> Result<i64> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(i64::try_from(duration.as_millis())?)
}

fn load_bloom_auth_data() -> BloomAuthData {
    let token = std::env::var("BLOOM_AUTH_TOKEN").ok();
    let expires_at = std::env::var("BLOOM_AUTH_TOKEN_EXPIRES_AT")
        .ok()
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return None;
            }
            trimmed.parse::<i64>().ok()
        });
    BloomAuthData { token, expires_at }
}

fn handle_success(
    id: &str,
    tracker: &BloomSwapTracker,
    token_name: Option<String>,
    tx_hash: Option<String>,
) {
    let mut responses = PENDING_BLOOM_RESPONSES.lock();
    if let Some(sender) = responses.remove(&tracker.mint) {
        let ack = BloomBuyAck {
            pending_time: tracker.started_at,
            success_time: Instant::now(),
            token_name,
            signature: tx_hash.clone(),
        };
        if sender.send(ack).is_err() {
            log::warn!(
                "bloom_ws: ack channel closed id={} mint={}",
                id,
                tracker.mint
            );
        } else {
            log::info!(
                "bloom_ws: success id={} mint={} side={} tx_hash={:?}",
                id,
                tracker.mint,
                tracker.side,
                tx_hash
            );
        }
    } else {
        log::warn!(
            "bloom_ws: success but no pending sender id={} mint={}",
            id,
            tracker.mint
        );
    }
}

fn handle_failure(id: &str, tracker: &BloomSwapTracker, status: u8, error: Option<String>) {
    let mut responses = PENDING_BLOOM_RESPONSES.lock();
    if responses.remove(&tracker.mint).is_some() {
        log::warn!(
            "bloom_ws: failure id={} mint={} side={} status={} error={:?}",
            id,
            tracker.mint,
            tracker.side,
            status,
            error
        );
    } else {
        log::warn!(
            "bloom_ws: failure without pending sender id={} mint={} status={} error={:?}",
            id,
            tracker.mint,
            status,
            error
        );
    }
}

fn log_close_frame(frame: Option<CloseFrame<'_>>) {
    if let Some(close_frame) = frame {
        log::warn!(
            "bloom_ws: connection closed code={} reason={}",
            close_frame.code,
            close_frame.reason
        );
    } else {
        log::warn!("bloom_ws: connection closed without frame");
    }
}

async fn cleanup_stale_swaps() {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let now = Instant::now();
        let mut stale_entries = Vec::new();
        {
            let swaps = ACTIVE_BLOOM_SWAPS.lock();
            for (id, tracker) in swaps.iter() {
                if now.duration_since(tracker.started_at) > Duration::from_secs(300) {
                    stale_entries.push((id.clone(), tracker.mint.clone()));
                }
            }
        }
        if stale_entries.is_empty() {
            continue;
        }
        {
            let mut swaps = ACTIVE_BLOOM_SWAPS.lock();
            for (id, _) in &stale_entries {
                swaps.remove(id);
            }
        }
        {
            let mut responses = PENDING_BLOOM_RESPONSES.lock();
            for (_, mint) in &stale_entries {
                responses.remove(mint);
            }
        }
        log::warn!("bloom_ws: removed {} stale swaps", stale_entries.len());
    }
}
