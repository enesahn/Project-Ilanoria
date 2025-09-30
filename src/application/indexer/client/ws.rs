use crate::application::indexer::config::{INITIAL_BACKOFF_MS, MAX_BACKOFF_MS, WS_URL};
use crate::application::indexer::indexer::index_mint_shards;
use crate::application::indexer::types::WsEvent;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::{Duration, Instant};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;

pub async fn run_ws_ingest() {
    let redis_url = match std::env::var("REDIS_URL") {
        Ok(v) => v,
        Err(_) => {
            log::error!("REDIS_URL environment variable not set");
            return;
        }
    };

    let mut backoff_ms = INITIAL_BACKOFF_MS;

    loop {
        if let Err(e) = connect_and_process(&redis_url).await {
            log::warn!("pp.ws connection failed: {}", e);
        }

        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
    }
}

async fn connect_and_process(redis_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let t_conn = Instant::now();
    let (mut ws, _) = connect_async(WS_URL).await?;

    let conn_us = t_conn.elapsed().as_micros();
    log::info!("pp.ws connected perf.us={}", conn_us);

    let subscription = json!({"method": "subscribeNewToken"});
    ws.send(Message::Text(subscription.to_string())).await?;

    while let Some(msg) = ws.next().await {
        match msg? {
            Message::Text(text) => {
                if let Ok(event) = serde_json::from_str::<WsEvent>(&text) {
                    if event.tx_type.as_deref() == Some("create") {
                        if let Some(mint) = event.mint {
                            if let Err(e) = index_mint_shards(redis_url, &mint).await {
                                log::error!("Failed to index mint {}: {}", mint, e);
                            }
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
