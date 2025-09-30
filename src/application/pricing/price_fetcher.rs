use super::config::{FETCH_INTERVAL, JUPITER_PRICE_API};
use super::types::SolPriceState;
use crate::HTTP_CLIENT;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct PriceResponse {
    prices: Prices,
}

#[derive(Deserialize, Debug)]
struct Prices {
    #[serde(rename = "So11111111111111111111111111111111111111112")]
    sol_price: f64,
}

pub async fn run_price_fetcher(state: SolPriceState) {
    loop {
        if let Err(e) = fetch_and_update_price(&state).await {
            log::error!("Failed to fetch SOL price: {}", e);
        }
        tokio::time::sleep(FETCH_INTERVAL).await;
    }
}

async fn fetch_and_update_price(state: &SolPriceState) -> Result<(), Box<dyn std::error::Error>> {
    let response = HTTP_CLIENT.get(JUPITER_PRICE_API).send().await?;

    if !response.status().is_success() {
        return Err(format!("API request failed with status: {}", response.status()).into());
    }

    let data = response.json::<PriceResponse>().await?;
    let price_as_int = data.prices.sol_price as u32;

    let mut price_guard = state.write().await;
    *price_guard = Some(price_as_int);

    Ok(())
}
