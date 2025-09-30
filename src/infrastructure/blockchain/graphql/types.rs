#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub(super) struct GraphQLQuery {
    pub query: String,
    pub variables: serde_json::Value,
}

#[derive(Deserialize)]
pub(super) struct GraphQLResponse<T> {
    pub data: T,
}

#[derive(Deserialize)]
pub(super) struct PumpFunAmmPoolResponse {
    #[serde(rename = "pump_fun_amm_Pool")]
    pub pump_fun_amm_pool: Vec<PumpFunAmmPool>,
}

#[derive(Deserialize)]
pub(super) struct RaydiumV4PoolResponse {
    #[serde(rename = "Raydium_LiquidityPoolv4")]
    pub raydium_liquidity_pool_v4: Vec<RaydiumV4Pool>,
}

#[derive(Deserialize)]
pub(super) struct RaydiumCpmmPoolResponse {
    #[serde(rename = "raydium_cp_swap_PoolState")]
    pub raydium_cp_swap_pool_state: Vec<RaydiumCpmmPool>,
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
pub struct PumpFunAmmPool {
    pub pubkey: String,
    pub pool_base_token_account: String,
    pub pool_quote_token_account: String,
    pub coin_creator: String,
    pub creator: String,
    pub lp_mint: String,
    pub base_mint: String,
    pub quote_mint: String,
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
pub struct RaydiumV4Pool {
    pub pubkey: String,
    #[serde(rename = "baseVault")]
    pub base_vault: String,
    #[serde(rename = "quoteVault")]
    pub quote_vault: String,
    #[serde(rename = "lpMint")]
    pub lp_mint: String,
    #[serde(rename = "baseMint")]
    pub base_mint: String,
    #[serde(rename = "quoteMint")]
    pub quote_mint: String,
    #[serde(rename = "marketId")]
    pub market_id: String,
    #[serde(rename = "openOrders")]
    pub open_orders: String,
    #[serde(rename = "targetOrders")]
    pub target_orders: String,
    pub nonce: i64,
    pub status: i64,
    pub state: i64,
    pub depth: i64,
    #[serde(rename = "baseDecimal")]
    pub base_decimal: i64,
    #[serde(rename = "quoteDecimal")]
    pub quote_decimal: i64,
    #[serde(rename = "minSize")]
    pub min_size: u64,
    #[serde(rename = "lpReserve")]
    pub lp_reserve: u64,
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
pub struct RaydiumCpmmPool {
    #[serde(rename = "token0Vault")]
    pub token0_vault: String,
    #[serde(rename = "token1Vault")]
    pub token1_vault: String,
    #[serde(rename = "lpMint")]
    pub lp_mint: String,
    #[serde(rename = "token0Mint")]
    pub token0_mint: String,
    #[serde(rename = "token1Mint")]
    pub token1_mint: String,
    pub pubkey: String,
    #[serde(rename = "ammConfig")]
    pub amm_config: String,
    pub status: i64,
    #[serde(rename = "mint0Decimals")]
    pub mint0_decimals: i64,
    #[serde(rename = "mint1Decimals")]
    pub mint1_decimals: i64,
    #[serde(rename = "lpMintDecimals")]
    pub lp_mint_decimals: i64,
    #[serde(rename = "lpSupply")]
    pub lp_supply: u64,
    #[serde(rename = "observationKey")]
    pub observation_key: String,
}
