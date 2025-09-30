#![allow(dead_code)]

use super::types::*;
use crate::HTTP_CLIENT;
use crate::infrastructure::blockchain::config::SHYFT_GRAPHQL_URL;
use anyhow::{Result, anyhow};

async fn execute_query<T: for<'de> serde::Deserialize<'de>>(
    query: String,
    error_context: &str,
) -> Result<T> {
    let api_key = std::env::var("SHYFT_API_KEY").map_err(|_| anyhow!("SHYFT_API_KEY not set"))?;
    let url = format!(
        "{}?api_key={}&network=mainnet-beta",
        SHYFT_GRAPHQL_URL, api_key
    );

    let variables = serde_json::json!({});
    let request_body = GraphQLQuery { query, variables };

    let response = HTTP_CLIENT.post(&url).json(&request_body).send().await?;

    if !response.status().is_success() {
        return Err(anyhow!("GraphQL request failed: {}", response.status()));
    }

    let resp_text = response.text().await?;
    serde_json::from_str(&resp_text).map_err(|e| {
        anyhow!(
            "Failed to parse {} JSON: {}, Body: {}",
            error_context,
            e,
            resp_text
        )
    })
}

pub async fn fetch_pump_swap_params(mint: &str) -> Result<PumpFunAmmPool> {
    let query = format!(
        r#"
        query MyQuery {{
          pump_fun_amm_Pool(
            where: {{_or: [{{base_mint: {{_eq: "{}"}}}}, {{quote_mint: {{_eq: "{}"}}}}]}}
          ) {{
            pubkey
            pool_base_token_account
            pool_quote_token_account
            coin_creator
            creator
            lp_mint
            base_mint
            quote_mint
          }}
        }}
        "#,
        mint, mint
    );

    let resp: GraphQLResponse<PumpFunAmmPoolResponse> =
        execute_query(query, "pump_fun_amm_Pool").await?;

    resp.data
        .pump_fun_amm_pool
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("Pool not found for mint: {}", mint))
}

pub async fn fetch_raydium_v4_params(mint: &str) -> Result<RaydiumV4Pool> {
    let query = format!(
        r#"
        query MyQuery {{
          Raydium_LiquidityPoolv4(
            where: {{_or: [{{baseMint: {{_eq: "{}"}}}}, {{quoteMint: {{_eq: "{}"}}}}]}}
          ) {{
            pubkey
            baseVault
            quoteVault
            lpMint
            baseMint
            quoteMint
            marketId
            openOrders
            targetOrders
            nonce
            status
            state
            depth
            baseDecimal
            quoteDecimal
            minSize
            lpReserve
          }}
        }}
        "#,
        mint, mint
    );

    let resp: GraphQLResponse<RaydiumV4PoolResponse> =
        execute_query(query, "Raydium_LiquidityPoolv4").await?;

    resp.data
        .raydium_liquidity_pool_v4
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("Pool not found for mint: {}", mint))
}

pub async fn fetch_raydium_cpmm_params(mint: &str) -> Result<RaydiumCpmmPool> {
    let query = format!(
        r#"
        query MyQuery {{
          raydium_cp_swap_PoolState(
            where: {{_or: [{{token0Mint: {{_eq: "{}"}}}}, {{token1Mint: {{_eq: "{}"}}}}]}}
          ) {{
            token0Vault
            token1Vault
            lpMint
            token0Mint
            token1Mint
            pubkey
            ammConfig
            status
            mint0Decimals
            mint1Decimals
            lpMintDecimals
            lpSupply
            observationKey
          }}
        }}
        "#,
        mint, mint
    );

    let resp: GraphQLResponse<RaydiumCpmmPoolResponse> =
        execute_query(query, "raydium_cp_swap_PoolState").await?;

    resp.data
        .raydium_cp_swap_pool_state
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("Pool not found for mint: {}", mint))
}
