use crate::HTTP_CLIENT;
use crate::interfaces::bot::data::BloomWalletInfo;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct WalletFetchRequest<'a> {
    auth: &'a str,
}

#[derive(Deserialize)]
struct WalletFetchResponse {
    #[serde(default)]
    wallets: Vec<RemoteWallet>,
}

#[derive(Deserialize)]
struct RemoteWallet {
    address: String,
    #[serde(default)]
    label: Option<String>,
}

fn resolve_region(region_hint: Option<&str>) -> String {
    if let Some(region) = region_hint {
        return region.to_lowercase();
    }
    if let Ok(region) = std::env::var("BLOOM_REGION") {
        if !region.is_empty() {
            return region.to_lowercase();
        }
    }
    "eu1".to_string()
}

pub async fn fetch_bloom_wallets(region_hint: Option<&str>) -> Result<Vec<BloomWalletInfo>> {
    let auth_token = std::env::var("BLOOM_AUTH_TOKEN")
        .map_err(|_| anyhow!("BLOOM_AUTH_TOKEN environment variable not set"))?;
    let region = resolve_region(region_hint);
    let url = format!("https://{}-tg.bloom-ext.app/get-wallets", region);
    let response = HTTP_CLIENT
        .post(url)
        .json(&WalletFetchRequest { auth: &auth_token })
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "Bloom get-wallets request failed with status {}: {}",
            status,
            body
        ));
    }
    let payload: WalletFetchResponse = response.json().await?;
    Ok(payload
        .wallets
        .into_iter()
        .map(|wallet| BloomWalletInfo {
            address: wallet.address,
            label: wallet.label,
        })
        .collect())
}
