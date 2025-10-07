use crate::{
    ACTIVE_BLOOM_SWAPS, BLOOM_WS_CONNECTION, BloomBuyAck, BloomSwapTracker,
    BloomWsConnectionStatus, PENDING_BLOOM_RESPONSES,
};
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::convert::TryFrom;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::time::{Duration, MissedTickBehavior, sleep};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue, Request, header};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use url::Url;

const BLOOM_WS_HOST: &str = "ws.bloom-ext.app";
const BLOOM_EXTENSION_ORIGIN: &str = "chrome-extension://akdiolpnpplhaoedmednpobkhmkophmg";
const BLOOM_WS_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0";
const BLOOM_WS_ACCEPT_ENCODING: &str = "gzip, deflate, br, zstd";
const BLOOM_WS_ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";
const BLOOM_WS_SEC_CH_UA: &str =
    "\"Not;A=Brand\";v=\"99\", \"Microsoft Edge\";v=\"140\", \"Chromium\";v=\"140\"";
const BLOOM_WS_SEC_CH_UA_MOBILE: &str = "?0";
const BLOOM_WS_SEC_CH_UA_PLATFORM: &str = "\"Windows\"";
const BLOOM_WS_SEC_FETCH_SITE: &str = "none";
const BLOOM_WS_SEC_FETCH_MODE: &str = "websocket";
const BLOOM_WS_SEC_FETCH_DEST: &str = "empty";
const BLOOM_WS_SEC_WEBSOCKET_EXTENSIONS: &str = "permessage-deflate; client_max_window_bits";

enum ConnectOutcome {
    SessionClosed { reason: String },
    Skipped { reason: String },
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

    update_ws_status(
        BloomWsConnectionStatus::Connecting,
        "Bloom WebSocket connection is starting.",
    );

    tokio::spawn(async {
        cleanup_stale_swaps().await;
    });

    loop {
        match connect_once().await {
            Ok(ConnectOutcome::SessionClosed { reason }) => {
                backoff_ms = 1000;
                let delay_secs = ((backoff_ms + 999) / 1000).max(1);
                let unit = if delay_secs == 1 { "second" } else { "seconds" };
                let message = format!(
                    "Bloom WS: {}. Retrying in {} {}.",
                    normalize_reason(&reason),
                    delay_secs,
                    unit
                );
                update_ws_status(BloomWsConnectionStatus::Disconnected, message);
            }
            Ok(ConnectOutcome::Skipped { reason }) => {
                backoff_ms = 30000;
                let delay_secs = ((backoff_ms + 999) / 1000).max(1);
                let unit = if delay_secs == 1 { "second" } else { "seconds" };
                let message = format!(
                    "Bloom WS: {}. Next attempt in {} {}.",
                    normalize_reason(&reason),
                    delay_secs,
                    unit
                );
                update_ws_status(BloomWsConnectionStatus::Unavailable, message);
            }
            Err(err) => {
                backoff_ms = (backoff_ms * 2).min(30000);
                let delay_secs = ((backoff_ms + 999) / 1000).max(1);
                let unit = if delay_secs == 1 { "second" } else { "seconds" };
                let message = format!(
                    "Bloom WS: {}. Retrying in {} {}.",
                    normalize_reason(&err.to_string()),
                    delay_secs,
                    unit
                );
                update_ws_status(BloomWsConnectionStatus::Disconnected, message);
            }
        }
        sleep(Duration::from_millis(backoff_ms)).await;
    }
}

fn update_ws_status(status: BloomWsConnectionStatus, message: impl Into<String>) {
    let mut state = BLOOM_WS_CONNECTION.lock();
    state.status = status;
    state.message = message.into();
}

fn normalize_reason(reason: &str) -> String {
    let mut text = reason.trim();

    loop {
        let trimmed = text.trim_end_matches('.');
        if trimmed.len() == text.len() {
            break;
        }
        text = trimmed.trim();
    }

    for prefix in [
        "Bloom WebSocket connection ",
        "Bloom WebSocket ",
        "Bloom WS: ",
        "WebSocket protocol error: ",
    ] {
        if let Some(stripped) = text.strip_prefix(prefix) {
            return normalize_reason(stripped);
        }
    }

    text.to_string()
}

fn build_bloom_ws_request(url: Url) -> Result<Request<()>> {
    let mut request = url.into_client_request()?;
    let headers = request.headers_mut();
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static(BLOOM_EXTENSION_ORIGIN),
    );
    headers.insert(
        header::USER_AGENT,
        HeaderValue::from_static(BLOOM_WS_USER_AGENT),
    );
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
    headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    headers.insert(
        HeaderName::from_static("sec-websocket-version"),
        HeaderValue::from_static("13"),
    );
    headers.insert(
        HeaderName::from_static("sec-websocket-extensions"),
        HeaderValue::from_static(BLOOM_WS_SEC_WEBSOCKET_EXTENSIONS),
    );
    headers.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static(BLOOM_WS_ACCEPT_ENCODING),
    );
    headers.insert(
        header::ACCEPT_LANGUAGE,
        HeaderValue::from_static(BLOOM_WS_ACCEPT_LANGUAGE),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static(BLOOM_WS_SEC_FETCH_SITE),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-mode"),
        HeaderValue::from_static(BLOOM_WS_SEC_FETCH_MODE),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-dest"),
        HeaderValue::from_static(BLOOM_WS_SEC_FETCH_DEST),
    );
    headers.insert(
        HeaderName::from_static("sec-ch-ua"),
        HeaderValue::from_static(BLOOM_WS_SEC_CH_UA),
    );
    headers.insert(
        HeaderName::from_static("sec-ch-ua-mobile"),
        HeaderValue::from_static(BLOOM_WS_SEC_CH_UA_MOBILE),
    );
    headers.insert(
        HeaderName::from_static("sec-ch-ua-platform"),
        HeaderValue::from_static(BLOOM_WS_SEC_CH_UA_PLATFORM),
    );
    headers.insert(header::HOST, HeaderValue::from_static(BLOOM_WS_HOST));
    Ok(request)
}

async fn connect_once() -> Result<ConnectOutcome> {
    let BloomAuthData { token, expires_at } = load_bloom_auth_data();
    let token = match token.map(|value| value.trim().to_string()) {
        Some(value) if !value.is_empty() => value,
        _ => {
            log::info!("bloom_ws: connection skipped: invalid or expired auth token");
            update_ws_status(
                BloomWsConnectionStatus::Unavailable,
                "Bloom WS: Authentication token missing or expired.",
            );
            return Ok(ConnectOutcome::Skipped {
                reason: "Authentication token missing or expired".to_string(),
            });
        }
    };
    if let Some(expires_at) = expires_at {
        let now_ms = current_time_millis()?;
        if now_ms > expires_at {
            log::info!("bloom_ws: connection skipped: invalid or expired auth token");
            update_ws_status(
                BloomWsConnectionStatus::Unavailable,
                "Bloom WS: Authentication token missing or expired.",
            );
            return Ok(ConnectOutcome::Skipped {
                reason: "Authentication token missing or expired".to_string(),
            });
        }
    }
    let url = match Url::parse(&format!("wss://{}?{}", BLOOM_WS_HOST, token)) {
        Ok(parsed) => parsed,
        Err(err) => {
            update_ws_status(
                BloomWsConnectionStatus::Disconnected,
                format!("Bloom WS: URL parsing failed ({})", err),
            );
            return Err(err.into());
        }
    };
    log::info!("bloom_ws: Attempting WebSocket connection...");
    let request = match build_bloom_ws_request(url) {
        Ok(value) => value,
        Err(err) => {
            update_ws_status(
                BloomWsConnectionStatus::Disconnected,
                format!("Bloom WS: Request construction failed ({})", err),
            );
            return Err(err);
        }
    };
    let (mut ws_stream, response) = match connect_async(request).await {
        Err(err) => {
            update_ws_status(
                BloomWsConnectionStatus::Disconnected,
                format!("Bloom WS: Handshake failed ({})", err),
            );
            return Err(err.into());
        }
        Ok(parts) => parts,
    };
    log::debug!(
        "bloom_ws: handshake response status={} headers={:?}",
        response.status(),
        response.headers()
    );
    log::info!("bloom_ws: connected");
    update_ws_status(
        BloomWsConnectionStatus::Connected,
        "Bloom WebSocket connection established.",
    );

    let mut keepalive = tokio::time::interval(Duration::from_secs(20));
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let close_reason = loop {
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
                            update_ws_status(
                                BloomWsConnectionStatus::Disconnected,
                                format!("Bloom WebSocket ping response failed: {}", err),
                            );
                            return Err(err.into());
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        break describe_close_frame(frame);
                    }
                    Some(Err(err)) => {
                        update_ws_status(
                            BloomWsConnectionStatus::Disconnected,
                            format!("Bloom WebSocket stream error: {}", err),
                        );
                        return Err(err.into());
                    }
                    None => {
                        log::warn!("bloom_ws: stream ended");
                        break String::from("Bloom WebSocket stream ended");
                    }
                }
            }
            _ = keepalive.tick() => {
                if let Err(err) = ws_stream.send(Message::Text("keepalive".to_string())).await {
                    update_ws_status(
                        BloomWsConnectionStatus::Disconnected,
                        format!("Bloom WebSocket keepalive failed: {}", err),
                    );
                    return Err(err.into());
                }
            }
        }
    };

    let resolved_reason = if close_reason.trim().is_empty() {
        String::from("Bloom WebSocket session ended unexpectedly")
    } else {
        close_reason
    };
    update_ws_status(
        BloomWsConnectionStatus::Disconnected,
        resolved_reason.clone(),
    );
    Ok(ConnectOutcome::SessionClosed {
        reason: resolved_reason,
    })
}

fn describe_close_frame(frame: Option<CloseFrame<'_>>) -> String {
    if let Some(close_frame) = frame {
        log::warn!(
            "bloom_ws: connection closed code={} reason={}",
            close_frame.code,
            close_frame.reason
        );
        format!(
            "Connection closed (code {} - {})",
            close_frame.code, close_frame.reason
        )
    } else {
        log::warn!("bloom_ws: connection closed without frame");
        String::from("Connection closed without close frame")
    }
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
