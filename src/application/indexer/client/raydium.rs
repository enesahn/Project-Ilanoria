use crate::application::indexer::config::{
    INITIAL_BACKOFF_MS, MAX_BACKOFF_MS, RAYDIUM_STREAM_METHOD, RAYDIUM_WS_URL, SOL_MINT_ADDRESS,
};
use crate::application::indexer::indexer::index_mint_shards;
use crate::application::indexer::types::{RaydiumEvent, RaydiumPool};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::{Duration, Instant};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::Message;

pub async fn run_raydium_pool_ingest() {
    let redis_url = match std::env::var("REDIS_URL") {
        Ok(value) => value,
        Err(_) => {
            log::error!("REDIS_URL environment variable not set");
            return;
        }
    };

    let auth_token = match std::env::var("BLOXROUTE_AUTH_TOKEN") {
        Ok(value) => value,
        Err(_) => {
            log::error!("BLOXROUTE_AUTH_TOKEN environment variable not set");
            return;
        }
    };

    let mut backoff_ms = INITIAL_BACKOFF_MS;

    loop {
        if let Err(error) = connect_and_process(&redis_url, &auth_token).await {
            log::warn!("ray.ws connection failed: {}", error);
        }

        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
    }
}

async fn connect_and_process(
    redis_url: &str,
    auth_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut request = RAYDIUM_WS_URL.into_client_request()?;
    let header_value = HeaderValue::from_str(auth_token)?;
    request.headers_mut().insert("Authorization", header_value);

    let connect_started = Instant::now();
    let (mut ws, _) = connect_async(request).await?;
    let connect_us = connect_started.elapsed().as_micros();
    log::info!("ray.ws connected perf.us={}", connect_us);

    let subscription = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "subscribe",
        "params": [
            RAYDIUM_STREAM_METHOD,
            { "includeCPMM": true }
        ]
    });

    ws.send(Message::Text(subscription.to_string())).await?;

    while let Some(message) = ws.next().await {
        match message? {
            Message::Text(text) => {
                if let Ok(event) = serde_json::from_str::<RaydiumEvent>(&text) {
                    if let Some(mint) = extract_candidate_mint(event.pool()) {
                        if let Err(error) = index_mint_shards(redis_url, &mint, "raydium.ws").await
                        {
                            log::error!("Failed to index Raydium mint {}: {}", mint, error);
                        }
                    }
                }
            }
            Message::Ping(payload) => {
                ws.send(Message::Pong(payload)).await?;
            }
            Message::Close(_) => break,
            Message::Binary(_) => {}
            _ => {}
        }
    }

    Ok(())
}

fn extract_candidate_mint(pool: Option<&RaydiumPool>) -> Option<String> {
    let pool = pool?;
    let token1 = pool.token1_mint_address.as_deref();
    let token2 = pool.token2_mint_address.as_deref();

    match (token1, token2) {
        (Some(a), Some(b)) => {
            let a_is_sol = a == SOL_MINT_ADDRESS;
            let b_is_sol = b == SOL_MINT_ADDRESS;

            if a_is_sol && !b_is_sol {
                return Some(b.to_string());
            }

            if b_is_sol && !a_is_sol {
                return Some(a.to_string());
            }

            if !a_is_sol && !b_is_sol {
                return Some(a.to_string());
            }
        }
        (Some(a), None) => {
            if a != SOL_MINT_ADDRESS {
                return Some(a.to_string());
            }
        }
        (None, Some(b)) => {
            if b != SOL_MINT_ADDRESS {
                return Some(b.to_string());
            }
        }
        _ => {}
    }

    None
}
