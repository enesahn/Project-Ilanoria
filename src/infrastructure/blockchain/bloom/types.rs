use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct BloomWallet<'a> {
    pub address: &'a str,
    pub label: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BloomSwapPayload<'a> {
    pub id: String,
    pub auth_token: String,
    pub address: &'a str,
    pub amount: f64,
    pub priority_fee: f64,
    pub processor_tip: f64,
    pub slippage: u32,
    pub side: &'a str,
    pub skip_if_bought: bool,
    pub anti_mev: bool,
    pub auto_tip: bool,
    pub dev_sell: Option<String>,
    pub amount_type: &'a str,
    pub wallets: Vec<BloomWallet<'a>>,
}

#[derive(Deserialize)]
pub(super) struct BloomSwapResponse {
    pub success: bool,
    pub error: Option<String>,
}
