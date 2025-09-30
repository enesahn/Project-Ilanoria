use crate::infrastructure::blockchain::bloom::types::{BloomSwapPayload, BloomWallet};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use reqwest::Client;
use std::sync::Arc;
use std::time::Instant;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub enum WarmupStatus {
    Success,
    Failed,
    Pending,
}

#[derive(Clone, Debug)]
pub struct WarmupResult {
    pub url: String,
    pub status: WarmupStatus,
    pub latency_ms: Option<f64>,
    pub last_checked: Option<DateTime<Utc>>,
}

pub type WarmerState = Arc<Mutex<Vec<WarmupResult>>>;

pub async fn run_warmer(state_arc: WarmerState) {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    loop {
        let urls_to_warm: Vec<String> = {
            let state = state_arc.lock();
            state.iter().map(|r| r.url.clone()).collect()
        };

        for url in urls_to_warm {
            let start_time = Instant::now();

            let (status, latency) = if url.contains("node1.me") {
                let ping_url = format!("{}/ping", url.trim_end_matches('/'));
                let response = client.get(&ping_url).send().await;
                let duration = start_time.elapsed();
                let latency_ms = Some(duration.as_micros() as f64 / 1000.0);

                match response {
                    Ok(res) if res.status().is_success() => match res.text().await {
                        Ok(text) if text.trim() == "pong" => (WarmupStatus::Success, latency_ms),
                        _ => (WarmupStatus::Failed, latency_ms),
                    },
                    _ => (WarmupStatus::Failed, latency_ms),
                }
            } else if url.contains("0slot.trade") {
                let health_url = format!("{}/health", url.trim_end_matches('/'));
                let response = client.get(&health_url).send().await;
                let duration = start_time.elapsed();
                let latency_ms = Some(duration.as_micros() as f64 / 1000.0);

                match response {
                    Ok(res) if res.status().is_success() => match res.text().await {
                        Ok(text) if text.trim() == "OK" => (WarmupStatus::Success, latency_ms),
                        _ => (WarmupStatus::Failed, latency_ms),
                    },
                    _ => (WarmupStatus::Failed, latency_ms),
                }
            } else if url.contains("bloom-ext.app") {
                let auth_token = match std::env::var("BLOOM_AUTH_TOKEN") {
                    Ok(token) => token,
                    Err(_) => {
                        log::error!("BLOOM_AUTH_TOKEN not set, cannot warm up Bloom API.");
                        String::new()
                    }
                };

                if auth_token.is_empty() {
                    (WarmupStatus::Failed, Some(0.0))
                } else {
                    let payload = BloomSwapPayload {
                        id: format!("QT-WARMER-{}", Uuid::new_v4()),
                        auth_token,
                        address: "DLzvNdYN4GKPBtY6yJZrsZt2wxhLkJEV4ZkyLidipump",
                        amount: 10000.0,
                        priority_fee: 0.1,
                        processor_tip: 0.1,
                        slippage: 20,
                        side: "Buy",
                        skip_if_bought: false,
                        anti_mev: false,
                        auto_tip: false,
                        dev_sell: None,
                        amount_type: "exact_in",
                        wallets: vec![BloomWallet {
                            address: "5zsdFixcN3D67AXBrNeHjW49tgkQ3VmdtTKA8DMxLuub",
                            label: "yoomain",
                        }],
                    };

                    let api_url = "http://eu1.bloom-ext.app/api/extension-swap";
                    let response = client.post(api_url).json(&payload).send().await;
                    let duration = start_time.elapsed();
                    let latency_ms = Some(duration.as_micros() as f64 / 1000.0);

                    match response {
                        Ok(res) if !res.status().is_server_error() => {
                            (WarmupStatus::Success, latency_ms)
                        }
                        _ => (WarmupStatus::Failed, latency_ms),
                    }
                }
            } else if url.contains("quiknode")
                || url.contains("shyft.to")
                || url.contains("helius-rpc")
            {
                let body = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "getHealth"
                });
                let response = client.post(&url).json(&body).send().await;
                let duration = start_time.elapsed();
                let latency_ms = Some(duration.as_micros() as f64 / 1000.0);
                match response {
                    Ok(res) if res.status().is_success() => {
                        match res.json::<serde_json::Value>().await {
                            Ok(json) if json.get("result").is_some() => {
                                (WarmupStatus::Success, latency_ms)
                            }
                            _ => (WarmupStatus::Failed, latency_ms),
                        }
                    }
                    _ => (WarmupStatus::Failed, latency_ms),
                }
            } else {
                let response = client.head(&url).send().await;
                let duration = start_time.elapsed();
                let latency_ms = Some(duration.as_micros() as f64 / 1000.0);
                match response {
                    Ok(res) if res.status().is_success() => (WarmupStatus::Success, latency_ms),
                    _ => (WarmupStatus::Failed, latency_ms),
                }
            };

            let mut state = state_arc.lock();
            if let Some(result) = state.iter_mut().find(|r| r.url == url) {
                result.status = status;
                result.latency_ms = latency;
                result.last_checked = Some(Utc::now());
            }
        }

        sleep(Duration::from_secs(15)).await;
    }
}
