use super::types::{BloomSwapPayload, BloomSwapResponse, BloomWallet};
use crate::infrastructure::blockchain::config::BLOOM_SWAP_URL;
use crate::{ACTIVE_BLOOM_SWAPS, BloomSwapTracker, HTTP_CLIENT};
use anyhow::{Result, anyhow};
use std::time::Instant;
use uuid::Uuid;

async fn execute_swap(
    mint_address: &str,
    amount: f64,
    slippage_percent: u32,
    priority_fee: f64,
    wallet_address: &str,
    wallet_label: &str,
    side: &str,
) -> Result<()> {
    let auth_token = std::env::var("BLOOM_AUTH_TOKEN")
        .map_err(|_| anyhow!("BLOOM_AUTH_TOKEN environment variable not set"))?;

    let swap_id = format!("QT-{}", Uuid::new_v4());
    {
        let mut swaps = ACTIVE_BLOOM_SWAPS.lock();
        swaps.insert(
            swap_id.clone(),
            BloomSwapTracker {
                mint: mint_address.to_string(),
                side: side.to_string(),
                started_at: Instant::now(),
            },
        );
    }

    let payload = BloomSwapPayload {
        id: swap_id.clone(),
        auth_token,
        address: mint_address,
        amount,
        priority_fee,
        processor_tip: priority_fee,
        slippage: slippage_percent,
        side,
        skip_if_bought: false,
        anti_mev: false,
        auto_tip: false,
        dev_sell: None,
        amount_type: "exact_in",
        wallets: vec![BloomWallet {
            address: wallet_address,
            label: wallet_label,
        }],
    };

    let response = HTTP_CLIENT
        .post(BLOOM_SWAP_URL)
        .json(&payload)
        .send()
        .await?;

    if !response.status().is_success() {
        ACTIVE_BLOOM_SWAPS.lock().remove(&swap_id);
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "Bloom API request failed with status {}: {}",
            status,
            text
        ));
    }

    let swap_response: BloomSwapResponse = response.json().await?;

    if swap_response.success {
        Ok(())
    } else {
        ACTIVE_BLOOM_SWAPS.lock().remove(&swap_id);
        Err(anyhow!(swap_response.error.unwrap_or_else(|| {
            "Unknown error from Bloom API".to_string()
        })))
    }
}

pub async fn buy(
    mint_address: &str,
    sol_amount: f64,
    slippage_percent: u32,
    priority_fee: f64,
    wallet_address: &str,
    wallet_label: &str,
) -> Result<()> {
    execute_swap(
        mint_address,
        sol_amount,
        slippage_percent,
        priority_fee,
        wallet_address,
        wallet_label,
        "Buy",
    )
    .await
}
