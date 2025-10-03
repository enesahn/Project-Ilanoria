use serde::Deserialize;

#[derive(Deserialize)]
pub struct WsEvent {
    #[serde(default)]
    pub mint: Option<String>,
    #[serde(rename = "txType", default)]
    pub tx_type: Option<String>,
}

#[derive(Deserialize)]
pub struct RaydiumEvent {
    #[serde(default)]
    pub params: Option<RaydiumEventParams>,
}

impl RaydiumEvent {
    pub fn pool(&self) -> Option<&RaydiumPool> {
        self.params.as_ref()?.result.as_ref()?.pool.as_ref()
    }
}

#[derive(Deserialize)]
pub struct RaydiumEventParams {
    #[serde(default)]
    pub result: Option<RaydiumEventResult>,
}

#[derive(Deserialize)]
pub struct RaydiumEventResult {
    #[serde(default)]
    pub pool: Option<RaydiumPool>,
}

#[derive(Deserialize)]
pub struct RaydiumPool {
    #[serde(rename = "token1MintAddress", default)]
    pub token1_mint_address: Option<String>,
    #[serde(rename = "token2MintAddress", default)]
    pub token2_mint_address: Option<String>,
}
