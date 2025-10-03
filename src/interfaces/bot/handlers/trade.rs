// File: src/telegram/handlers/trade.rs
use parking_lot::Mutex;
use redis::Client as RedisClient;
use solana_sdk::pubkey::Pubkey;
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::Account as TokenAccount;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use teloxide::prelude::*;

use super::start::Command;
use super::text::{format_token_info_message, get_parsed_token_info, parse_mint_from_text_robust};
use crate::application::pricing::SolPriceState;
use crate::infrastructure::blockchain::{RpcClients, bloom_buy, bloom_sell};
use crate::interfaces::bot::user::client::UserClientHandle;
use crate::interfaces::bot::{
    State, UserData, get_user_data, log_buffer_to_tx, token_info_keyboard,
};
use crate::{BloomBuyAck, PENDING_BLOOM_RESPONSES};
use tokio::sync::oneshot;
use tokio::time::Duration;

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
type MyDialogue = Dialogue<State, teloxide::dispatching::dialogue::InMemStorage<State>>;

pub async fn get_token_balance(
    rpc_client: &solana_rpc_client::nonblocking::rpc_client::RpcClient,
    owner: &Pubkey,
    mint: &Pubkey,
) -> Result<u64, anyhow::Error> {
    let ata_pubkey = spl_associated_token_account::get_associated_token_address(owner, mint);
    match rpc_client.get_account(&ata_pubkey).await {
        Ok(account) => {
            let token_account = <TokenAccount as Pack>::unpack(&account.data)?;
            Ok(token_account.amount)
        }
        Err(_) => Ok(0),
    }
}

pub async fn handle_trade_callback(
    q: CallbackQuery,
    bot: Bot,
    redis_client: RedisClient,
    dialogue: MyDialogue,
    sol_price_state: SolPriceState,
    user_client_handle: Arc<Mutex<Option<UserClientHandle>>>,
    rpc_clients: RpcClients,
) -> HandlerResult {
    if let Some(message) = q.message.clone() {
        let chat_id = message.chat.id;
        let data = q.data.clone().unwrap_or_default();
        let message_text = message.text().unwrap_or_default();

        let maybe_mint = parse_mint_from_text_robust(message_text);
        log::info!("[TRADE_CALLBACK] Parsed Mint Address: {:?}", maybe_mint);

        if data == "r" {
            if let Some(mint) = maybe_mint {
                match get_parsed_token_info(&mint, user_client_handle).await {
                    Ok(token_info) => {
                        let new_text = format_token_info_message(
                            &mint,
                            &token_info,
                            chat_id.0,
                            redis_client.clone(),
                            sol_price_state.clone(),
                            rpc_clients.clone(),
                        )
                        .await;
                        let mut con = redis_client.get_multiplexed_async_connection().await?;
                        let user_data = get_user_data(&mut con, chat_id.0).await?.unwrap();
                        let keyboard = token_info_keyboard(&user_data.config, &mint);

                        match bot
                            .edit_message_text(chat_id, message.id, new_text)
                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                            .disable_web_page_preview(true)
                            .reply_markup(keyboard)
                            .await
                        {
                            Ok(_) => {}
                            Err(teloxide::RequestError::Api(
                                teloxide::ApiError::MessageNotModified,
                            )) => {}
                            Err(e) => return Err(Box::new(e)),
                        }
                    }
                    Err(e) => {
                        bot.answer_callback_query(q.id.clone())
                            .text(&e.to_string())
                            .show_alert(true)
                            .await?;
                    }
                }
            } else {
                bot.answer_callback_query(q.id.clone())
                    .text("Could not find mint address in the message.")
                    .show_alert(true)
                    .await?;
            }
        } else if data.starts_with("b_") || data.starts_with("s_") {
            let mint = match maybe_mint {
                Some(m) => m,
                None => {
                    bot.send_message(chat_id, "Could not find mint address in the message.")
                        .await?;
                    return Ok(());
                }
            };

            if data == "b_custom" {
                let prompt_message = bot
                    .send_message(chat_id, "Please enter the amount of SOL to buy.")
                    .await?;
                dialogue
                    .update(State::ReceiveCustomBuyAmount {
                        prompt_message_id: prompt_message.id,
                        mint,
                    })
                    .await?;
                return Ok(());
            }

            if data == "s_custom" {
                let prompt_message = bot
                    .send_message(
                        chat_id,
                        "Please enter the percentage of tokens to sell (1-100).",
                    )
                    .await?;
                dialogue
                    .update(State::ReceiveCustomSellPercentage {
                        prompt_message_id: prompt_message.id,
                        mint,
                    })
                    .await?;
                return Ok(());
            }

            let mut con = redis_client.get_multiplexed_async_connection().await?;
            let user_data = match get_user_data(&mut con, chat_id.0).await? {
                Some(data) => data,
                None => {
                    bot.send_message(chat_id, "Please /start the bot first.")
                        .await?;
                    return Ok(());
                }
            };

            let cmd = if let Some(amount_part) = data.strip_prefix("b_") {
                if let Ok(amount) = amount_part.parse::<f64>() {
                    let log_msg =
                        format!("🟡 Attempting to BUY {} SOL of {} via Bloom", amount, mint);
                    bot.send_message(chat_id, &log_msg).await?;
                    crate::info!(chat_id.0, "{}", log_msg);
                    Command::Buy(amount, mint)
                } else {
                    bot.send_message(chat_id, "Invalid amount.").await?;
                    return Ok(());
                }
            } else if let Some(percent_part) = data.strip_prefix("s_") {
                if let Ok(percentage) = percent_part.parse::<u64>() {
                    let rpc_client = &rpc_clients.helius_client;
                    let wallet_pubkey =
                        Pubkey::from_str(&user_data.get_default_wallet().unwrap().public_key)
                            .unwrap();
                    let mint_pubkey = Pubkey::from_str(&mint).unwrap();

                    let total_balance =
                        match get_token_balance(rpc_client, &wallet_pubkey, &mint_pubkey).await {
                            Ok(balance) => balance,
                            Err(e) => {
                                bot.send_message(
                                    chat_id,
                                    format!("Failed to get token balance: {}", e),
                                )
                                .await?;
                                return Ok(());
                            }
                        };

                    if total_balance == 0 {
                        bot.send_message(chat_id, "You don't have any of this token to sell.")
                            .await?;
                        return Ok(());
                    }

                    let amount_to_sell =
                        (total_balance as f64 * (percentage as f64 / 100.0)) as u64;

                    let log_msg = format!(
                        "🟡 Attempting to SELL {}% ({} tokens) of {} via Bloom",
                        percentage, amount_to_sell, mint
                    );
                    bot.send_message(chat_id, &log_msg).await?;
                    crate::info!(chat_id.0, "{}", log_msg);
                    Command::Sell(amount_to_sell, mint)
                } else {
                    bot.send_message(chat_id, "Invalid percentage.").await?;
                    return Ok(());
                }
            } else {
                bot.send_message(chat_id, "Invalid action.").await?;
                return Ok(());
            };

            let result_message = run_trade_in_blocking_task(
                user_data,
                cmd,
                redis_client.clone(),
                chat_id.0,
                rpc_clients,
            )
            .await;
            bot.send_message(chat_id, result_message).await?;
            return Ok(());
        }
    }
    Ok(())
}

fn build_bloom_trade_message(
    action: &str,
    amount: f64,
    mint: &str,
    priority_fee: f64,
    api_ms: u128,
    ack: Option<&BloomBuyAck>,
) -> String {
    let action_title = match action {
        "buy" => "Buy",
        "sell" => "Sell",
        _ => "Trade",
    };
    let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
    let (amount_str, unit_label) = match action {
        "sell" => (format!("{:.0}", amount), "tokens"),
        _ => (format!("{:.4}", amount), "SOL"),
    };
    let header = format!("🟢 {} Successful | {}", action_title, timestamp);
    let mut message = format!(
        "{}\nRoute: Bloom\nMint: {}\nAmount: {} {}\nAPI: {} ms\nPriority Fee: {:.6} SOL",
        header, mint, amount_str, unit_label, api_ms, priority_fee
    );
    if let Some(ack) = ack {
        let confirmation_ms = ack
            .success_time
            .duration_since(ack.pending_time)
            .as_millis();
        message.push_str(&format!("\nConfirmation: {} ms", confirmation_ms));
        if let Some(token_name) = &ack.token_name {
            if !token_name.is_empty() {
                message.push_str(&format!("\nToken: {}", token_name));
            }
        }
        if let Some(signature) = &ack.signature {
            message.push_str(&format!("\nSignature: {}", signature));
        }
    } else {
        message.push_str("\nConfirmation pending");
    }
    message
}

pub async fn run_trade_in_blocking_task(
    user_data: UserData,
    cmd: Command,
    _redis_client: RedisClient,
    chat_id: i64,
    _rpc_clients: RpcClients,
) -> String {
    let log_buffer = Arc::new(Mutex::new(Vec::new()));

    let default_wallet = match user_data.get_default_wallet() {
        Some(w) => w.clone(),
        None => return "No default wallet configured for trading.".to_string(),
    };

    let wallet_public_key = default_wallet.public_key.clone();
    let wallet_label = default_wallet.name.trim().to_string();
    let wallet_label = if wallet_label.is_empty() {
        "Default Wallet".to_string()
    } else {
        wallet_label
    };

    let priority_fee = match &cmd {
        Command::Buy(..) => user_data.config.buy_priority_fee_sol,
        Command::Sell(..) => user_data.config.sell_priority_fee_sol,
        _ => user_data.config.buy_priority_fee_sol,
    };

    match cmd {
        Command::Buy(amount, mint) => {
            if amount <= 0.0 {
                return "Amount must be greater than zero.".to_string();
            }

            let slippage_percent = user_data.config.slippage_percent;

            let (tx, rx) = oneshot::channel();
            PENDING_BLOOM_RESPONSES.lock().insert(mint.clone(), tx);

            let api_start = Instant::now();
            log_buffer.lock().push(format!(
                "[{}] DETAILS: Bloom buy {:.4} SOL",
                chrono::Utc::now().to_rfc3339(),
                amount
            ));

            let response = bloom_buy(
                &mint,
                amount,
                slippage_percent,
                priority_fee,
                wallet_public_key.as_str(),
                wallet_label.as_str(),
            )
            .await;

            match response {
                Ok(_) => {
                    let api_duration = api_start.elapsed();
                    log_buffer.lock().push(format!(
                        "[{}] PERF_METRIC::{{ \"stage\": \"bloom_request\", \"duration_ms\": {:.4} }}",
                        chrono::Utc::now().to_rfc3339(),
                        api_duration.as_micros() as f64 / 1000.0
                    ));

                    let ack_wait_start = Instant::now();
                    let ack_result = tokio::time::timeout(Duration::from_secs(20), rx).await;
                    let mut ack_option: Option<BloomBuyAck> = None;
                    let ack_status: &str;
                    match ack_result {
                        Ok(Ok(ack)) => {
                            let confirmation_ms = ack
                                .success_time
                                .duration_since(ack.pending_time)
                                .as_millis();
                            log_buffer.lock().push(format!(
                                "[{}] PERF_METRIC::{{ \"stage\": \"bloom_confirmation\", \"duration_ms\": {:.4} }}",
                                chrono::Utc::now().to_rfc3339(),
                                confirmation_ms as f64
                            ));
                            ack_option = Some(ack);
                            ack_status = "confirmed";
                        }
                        Ok(Err(_)) => {
                            PENDING_BLOOM_RESPONSES.lock().remove(&mint);
                            ack_status = "channel_closed";
                        }
                        Err(_) => {
                            PENDING_BLOOM_RESPONSES.lock().remove(&mint);
                            ack_status = "timeout";
                        }
                    }

                    let ack_wait_ms = ack_wait_start.elapsed().as_millis();
                    log_buffer.lock().push(format!(
                        "[{}] PERF_METRIC::{{ \"stage\": \"ack_wait\", \"duration_ms\": {:.4}, \"status\": \"{}\" }}",
                        chrono::Utc::now().to_rfc3339(),
                        ack_wait_ms as f64,
                        ack_status
                    ));
                    log_buffer.lock().push(format!(
                        "[{}] FEE: {:.6} SOL",
                        chrono::Utc::now().to_rfc3339(),
                        priority_fee
                    ));

                    let final_message = build_bloom_trade_message(
                        "buy",
                        amount,
                        &mint,
                        priority_fee,
                        api_duration.as_millis(),
                        ack_option.as_ref(),
                    );

                    let signature_opt = ack_option.as_ref().and_then(|ack| ack.signature.clone());
                    if let Some(signature) = signature_opt.clone() {
                        let buffer_snapshot = log_buffer.lock().clone();
                        tokio::spawn(log_buffer_to_tx(
                            chat_id,
                            signature.clone(),
                            buffer_snapshot,
                        ));
                        crate::info!(chat_id, "Bloom buy successful. Signature: {}", signature);
                    } else {
                        crate::info!(
                            chat_id,
                            "Bloom buy completed without signature. Status: {}",
                            ack_status
                        );
                    }

                    final_message
                }
                Err(error) => {
                    PENDING_BLOOM_RESPONSES.lock().remove(&mint);
                    let error_message = format!("❌ Bloom buy failed: {}", error);
                    crate::error!(chat_id, "{}", error_message);
                    error_message
                }
            }
        }
        Command::Sell(token_amount, mint) => {
            if token_amount == 0 {
                return "Token amount must be greater than zero.".to_string();
            }

            let slippage_percent = user_data.config.slippage_percent;

            let (tx, rx) = oneshot::channel();
            PENDING_BLOOM_RESPONSES.lock().insert(mint.clone(), tx);

            let api_start = Instant::now();
            log_buffer.lock().push(format!(
                "[{}] DETAILS: Bloom sell {} tokens",
                chrono::Utc::now().to_rfc3339(),
                token_amount
            ));

            let response = bloom_sell(
                &mint,
                token_amount,
                slippage_percent,
                priority_fee,
                wallet_public_key.as_str(),
                wallet_label.as_str(),
            )
            .await;

            match response {
                Ok(_) => {
                    let api_duration = api_start.elapsed();
                    log_buffer.lock().push(format!(
                        "[{}] PERF_METRIC::{{ \"stage\": \"bloom_request\", \"duration_ms\": {:.4} }}",
                        chrono::Utc::now().to_rfc3339(),
                        api_duration.as_micros() as f64 / 1000.0
                    ));

                    let ack_wait_start = Instant::now();
                    let ack_result = tokio::time::timeout(Duration::from_secs(20), rx).await;
                    let mut ack_option: Option<BloomBuyAck> = None;
                    let ack_status: &str;
                    match ack_result {
                        Ok(Ok(ack)) => {
                            let confirmation_ms = ack
                                .success_time
                                .duration_since(ack.pending_time)
                                .as_millis();
                            log_buffer.lock().push(format!(
                                "[{}] PERF_METRIC::{{ \"stage\": \"bloom_confirmation\", \"duration_ms\": {:.4} }}",
                                chrono::Utc::now().to_rfc3339(),
                                confirmation_ms as f64
                            ));
                            ack_option = Some(ack);
                            ack_status = "confirmed";
                        }
                        Ok(Err(_)) => {
                            PENDING_BLOOM_RESPONSES.lock().remove(&mint);
                            ack_status = "channel_closed";
                        }
                        Err(_) => {
                            PENDING_BLOOM_RESPONSES.lock().remove(&mint);
                            ack_status = "timeout";
                        }
                    }

                    let ack_wait_ms = ack_wait_start.elapsed().as_millis();
                    log_buffer.lock().push(format!(
                        "[{}] PERF_METRIC::{{ \"stage\": \"ack_wait\", \"duration_ms\": {:.4}, \"status\": \"{}\" }}",
                        chrono::Utc::now().to_rfc3339(),
                        ack_wait_ms as f64,
                        ack_status
                    ));
                    log_buffer.lock().push(format!(
                        "[{}] FEE: {:.6} SOL",
                        chrono::Utc::now().to_rfc3339(),
                        priority_fee
                    ));

                    let final_message = build_bloom_trade_message(
                        "sell",
                        token_amount as f64,
                        &mint,
                        priority_fee,
                        api_duration.as_millis(),
                        ack_option.as_ref(),
                    );

                    let signature_opt = ack_option.as_ref().and_then(|ack| ack.signature.clone());
                    if let Some(signature) = signature_opt.clone() {
                        let buffer_snapshot = log_buffer.lock().clone();
                        tokio::spawn(log_buffer_to_tx(
                            chat_id,
                            signature.clone(),
                            buffer_snapshot,
                        ));
                        crate::info!(chat_id, "Bloom sell successful. Signature: {}", signature);
                    } else {
                        crate::info!(
                            chat_id,
                            "Bloom sell completed without signature. Status: {}",
                            ack_status
                        );
                    }

                    final_message
                }
                Err(error) => {
                    PENDING_BLOOM_RESPONSES.lock().remove(&mint);
                    let error_message = format!("❌ Bloom sell failed: {}", error);
                    crate::error!(chat_id, "{}", error_message);
                    error_message
                }
            }
        }
        _ => "Invalid trade command".to_string(),
    }
}

pub async fn trade_handler(
    bot: Bot,
    msg: Message,
    cmd: Command,
    redis_client: RedisClient,
    rpc_clients: RpcClients,
) -> HandlerResult {
    let chat_id = msg.chat.id.0;
    let mut con = redis_client.get_multiplexed_async_connection().await?;

    let user_data = match get_user_data(&mut con, chat_id).await? {
        Some(s) => s,
        None => {
            bot.send_message(
                msg.chat.id,
                "User data not found. Please /start the bot first.",
            )
            .await?;
            return Ok(());
        }
    };

    let log_msg = match &cmd {
        Command::Buy(amount, mint) => {
            format!("🟡 Attempting to BUY {} SOL of {} via Bloom", amount, mint)
        }
        Command::Sell(amount, mint) => format!(
            "🟡 Attempting to SELL {} tokens of {} via Bloom",
            amount, mint
        ),
        _ => "Invalid command".to_string(),
    };

    bot.send_message(msg.chat.id, &log_msg).await?;
    crate::info!(chat_id, "{}", log_msg);

    let result_message =
        run_trade_in_blocking_task(user_data, cmd, redis_client, chat_id, rpc_clients).await;

    bot.send_message(msg.chat.id, result_message).await?;

    Ok(())
}
