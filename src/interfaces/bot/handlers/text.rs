use lazy_static::lazy_static;
use parking_lot::Mutex;
use redis::Client as RedisClient;
use regex::Regex;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signer::{Signer, keypair::Keypair};
use std::sync::Arc;
use teloxide::prelude::*;

use crate::application::pricing::SolPriceState;
use crate::infrastructure::blockchain::RpcClients;
use crate::interfaces::bot::user::client::{
    UserClientHandle, get_token_info_from_bloom, search_dialogs,
};
use crate::interfaces::bot::{
    State, Wallet, channel_selection_keyboard, escape_markdown, generate_settings_text,
    generate_task_detail_text, generate_wallets_text, get_user_data, save_user_data,
    settings_menu_keyboard, task_detail_keyboard, token_info_keyboard, wallets_menu_keyboard,
};

type MyDialogue = Dialogue<State, teloxide::dispatching::dialogue::InMemStorage<State>>;
type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

lazy_static! {
    static ref SOLANA_ADDRESS_REGEX: Regex =
        Regex::new(r"^([1-9A-HJ-NP-Za-km-z]{32,44})$").unwrap();
    static ref MINT_REGEX_ROBUST: Regex = Regex::new(r"([1-9A-HJ-NP-Za-km-z]{32,44})").unwrap();
}

#[derive(Clone)]
pub struct TokenInfo {
    pub name: String,
    pub ticker: String,
    pub dex: String,
    pub market_cap: String,
    pub price: String,
    pub liquidity: String,
    pub links: String,
    pub renounced: bool,
    pub freeze_revoked: bool,
}

impl Default for TokenInfo {
    fn default() -> Self {
        Self {
            name: "N/A".to_string(),
            ticker: "N/A".to_string(),
            dex: "N/A".to_string(),
            market_cap: "N/A".to_string(),
            price: "N/A".to_string(),
            liquidity: "N/A".to_string(),
            links: "".to_string(),
            renounced: false,
            freeze_revoked: false,
        }
    }
}

fn parse_bloom_response(text: &str) -> TokenInfo {
    let mut info = TokenInfo::default();
    let mut name_ticker_found = false;

    for line in text.lines() {
        if !name_ticker_found
            && line.contains("•")
            && !line.starts_with("[CA](")
            && !line.contains("Share Token")
        {
            let parts: Vec<&str> = line.split('•').map(|s| s.trim()).collect();
            if parts.len() >= 2 {
                info.name = parts[0]
                    .trim_start_matches(|c: char| !c.is_alphanumeric())
                    .trim()
                    .to_string();
                info.ticker = parts[1].to_string();
                name_ticker_found = true;
            }
        } else if line.starts_with("Dex:") {
            info.dex = line.replace("Dex:", "").trim().to_string();
        } else if line.contains("Market Cap:") {
            if let Some(val) = line.split(':').nth(1) {
                info.market_cap = val.trim().to_string();
            }
        } else if line.contains("Price:") {
            if let Some(val) = line.split(':').nth(1) {
                info.price = val.trim().to_string();
            }
        } else if line.contains("Liquidity:") {
            if let Some(val) = line.split(':').nth(1) {
                info.liquidity = val.trim().to_string();
            }
        } else if line.contains("Renounced") {
            info.renounced = line.contains('🟢');
        } else if line.contains("Freeze") {
            info.freeze_revoked = line.contains('🟢');
        } else if line.starts_with("[CA](") && line.contains("[DEX](") {
            info.links = line.to_string();
        }
    }
    info
}

pub async fn get_parsed_token_info(
    mint: &str,
    user_client_handle: Arc<Mutex<Option<UserClientHandle>>>,
) -> Result<TokenInfo, String> {
    let handle = { user_client_handle.lock().clone() };
    if let Some(client) = handle {
        match get_token_info_from_bloom(&client, mint).await {
            Ok(response) => Ok(parse_bloom_response(&response)),
            Err(e) => Err(format!("❌ Error: {}", e)),
        }
    } else {
        Err("⚠️ Telegram User Client is not logged in. Please ask the admin to log in via the console.".to_string())
    }
}

pub async fn format_token_info_message(
    mint: &str,
    token_info: &TokenInfo,
    chat_id: i64,
    redis_client: RedisClient,
    sol_price_state: SolPriceState,
    rpc_clients: RpcClients,
) -> String {
    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let user_data = get_user_data(&mut con, chat_id).await.unwrap().unwrap();

    let wallet_balance_str = if let Some(wallet) = user_data.get_default_wallet() {
        let rpc_client = &rpc_clients.helius_client;
        let pubkey = Pubkey::try_from(wallet.public_key.as_str()).unwrap();
        match rpc_client.get_balance(&pubkey).await {
            Ok(lamports) => {
                let sol_balance = lamports as f64 / 1_000_000_000.0;
                let sol_balance_formatted = format!("{:.4}", sol_balance);

                let price_guard = sol_price_state.read().await;
                let usd_value_str = match *price_guard {
                    Some(price) => {
                        let usd_value = sol_balance * price as f64;
                        let usd_value_formatted = format!("{:.2}", usd_value);
                        format!(" \\(${}\\)", escape_markdown(&usd_value_formatted))
                    }
                    None => "".to_string(),
                };
                format!(
                    "*{} SOL{}*",
                    escape_markdown(&sol_balance_formatted),
                    usd_value_str
                )
            }
            Err(_) => "*Error*".to_string(),
        }
    } else {
        "*No Wallet*".to_string()
    };

    format!(
        "Buy {} — {} — From *{}*\n`{}`\n\n💳 Wallet: {}\n\n💰 Price: *{}*\n📊 Market Cap: *{}*\n💧 Liquidity: *{}*\n\n{}\n\nRenounced: {}  Freeze Revoked: {}",
        escape_markdown(&token_info.name),
        escape_markdown(&token_info.ticker),
        escape_markdown(&token_info.dex),
        mint,
        wallet_balance_str,
        escape_markdown(&token_info.price),
        escape_markdown(&token_info.market_cap),
        escape_markdown(&token_info.liquidity),
        token_info.links,
        if token_info.renounced { "✅" } else { "❌" },
        if token_info.freeze_revoked {
            "✅"
        } else {
            "❌"
        }
    )
}

pub fn parse_mint_from_text_robust(text: &str) -> Option<String> {
    MINT_REGEX_ROBUST.find(text).map(|m| m.as_str().to_string())
}

pub async fn text_handler(
    bot: Bot,
    msg: Message,
    dialogue: MyDialogue,
    redis_client: RedisClient,
    user_client_handle: Arc<Mutex<Option<UserClientHandle>>>,
    sol_price_state: SolPriceState,
    rpc_clients: RpcClients,
) -> HandlerResult {
    let chat_id = msg.chat.id;
    let text = match msg.text() {
        Some(text) => text,
        None => return Ok(()),
    };

    if let Some(captures) = SOLANA_ADDRESS_REGEX.captures(text) {
        if let Some(mint_address) = captures.get(1) {
            let mint = mint_address.as_str();

            match get_parsed_token_info(mint, user_client_handle.clone()).await {
                Ok(token_info) => {
                    let final_message = format_token_info_message(
                        mint,
                        &token_info,
                        chat_id.0,
                        redis_client.clone(),
                        sol_price_state.clone(),
                        rpc_clients.clone(),
                    )
                    .await;

                    let mut con = redis_client.get_multiplexed_async_connection().await?;
                    let user_data = get_user_data(&mut con, chat_id.0).await?.unwrap();
                    let keyboard = token_info_keyboard(&user_data.config, mint);

                    bot.send_message(chat_id, final_message)
                        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                        .disable_web_page_preview(true)
                        .reply_markup(keyboard)
                        .await?;
                }
                Err(error_message) => {
                    bot.send_message(chat_id, error_message).await?;
                }
            }
            return Ok(());
        }
    }

    if let Some(state) = dialogue.get().await? {
        match state.clone() {
            State::Start | State::SettingsMenu | State::WalletsMenu | State::TasksMenu => {
                return Ok(());
            }
            _ => {}
        }

        let mut con = redis_client.get_multiplexed_async_connection().await?;
        let mut user_data = get_user_data(&mut con, chat_id.0).await?.unwrap();

        bot.delete_message(chat_id, msg.id).await.ok();

        match state {
            State::TaskSelectChannelSearch {
                task_name,
                menu_message_id,
                prompt_message_id,
            } => {
                let handle = user_client_handle.lock().clone();
                if let Some(client) = handle {
                    match search_dialogs(&client, text).await {
                        Ok(channels) if !channels.is_empty() => {
                            let new_state = State::TaskSelectChannelFromList {
                                task_name,
                                menu_message_id,
                                prompt_message_id,
                                all_channels: channels,
                                page: 0,
                            };
                            dialogue.update(new_state.clone()).await?;
                            let keyboard = channel_selection_keyboard(&new_state).await.unwrap();
                            bot.edit_message_text(
                                chat_id,
                                prompt_message_id,
                                "Found channels/groups. Please select one:",
                            )
                            .reply_markup(keyboard)
                            .await?;
                        }
                        _ => {
                            bot.edit_message_text(
                                chat_id,
                                prompt_message_id,
                                "No channels found with that title. Please try again",
                            )
                            .await?;
                        }
                    }
                } else {
                    bot.edit_message_text(
                        chat_id,
                        prompt_message_id,
                        "User client is not logged in. Cannot search.",
                    )
                    .await?;
                    dialogue.update(State::TasksMenu).await?;
                }
            }
            State::TaskReceiveName {
                task_name,
                menu_message_id,
                prompt_message_id,
            } => {
                bot.delete_message(chat_id, prompt_message_id).await.ok();
                if let Some(task_index) = user_data.tasks.iter().position(|t| t.name == task_name) {
                    user_data.tasks[task_index].name = text.to_string();
                    save_user_data(&mut con, chat_id.0, &user_data).await?;

                    let task = &user_data.tasks[task_index];
                    let task_text = generate_task_detail_text(
                        redis_client.clone(),
                        chat_id.0,
                        task,
                        sol_price_state.clone(),
                    )
                    .await;
                    bot.edit_message_text(chat_id, menu_message_id, task_text)
                        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                        .reply_markup(task_detail_keyboard(task))
                        .await?;
                }
                dialogue.update(State::TasksMenu).await?;
            }
            State::TaskReceiveBuyAmount {
                task_name,
                menu_message_id,
                prompt_message_id,
            } => {
                bot.delete_message(chat_id, prompt_message_id).await.ok();
                if let Ok(amount) = text.parse::<f64>() {
                    if let Some(task_index) =
                        user_data.tasks.iter().position(|t| t.name == task_name)
                    {
                        user_data.tasks[task_index].buy_amount_sol = amount;
                        save_user_data(&mut con, chat_id.0, &user_data).await?;

                        let task = &user_data.tasks[task_index];
                        let task_text = generate_task_detail_text(
                            redis_client.clone(),
                            chat_id.0,
                            task,
                            sol_price_state.clone(),
                        )
                        .await;
                        bot.edit_message_text(chat_id, menu_message_id, task_text)
                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                            .reply_markup(task_detail_keyboard(task))
                            .await?;
                    }
                }
                dialogue.update(State::TasksMenu).await?;
            }
            State::TaskReceiveBuyFee {
                task_name,
                menu_message_id,
                prompt_message_id,
            } => {
                bot.delete_message(chat_id, prompt_message_id).await.ok();
                if let Ok(fee) = text.parse::<f64>() {
                    if let Some(task_index) =
                        user_data.tasks.iter().position(|t| t.name == task_name)
                    {
                        user_data.tasks[task_index].buy_priority_fee_sol = fee;
                        save_user_data(&mut con, chat_id.0, &user_data).await?;

                        let task = &user_data.tasks[task_index];
                        let task_text = generate_task_detail_text(
                            redis_client.clone(),
                            chat_id.0,
                            task,
                            sol_price_state.clone(),
                        )
                        .await;
                        bot.edit_message_text(chat_id, menu_message_id, task_text)
                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                            .reply_markup(task_detail_keyboard(task))
                            .await?;
                    }
                }
                dialogue.update(State::TasksMenu).await?;
            }
            State::TaskReceiveBuySlippage {
                task_name,
                menu_message_id,
                prompt_message_id,
            } => {
                bot.delete_message(chat_id, prompt_message_id).await.ok();
                if let Ok(slippage) = text.parse::<u32>() {
                    if let Some(task_index) =
                        user_data.tasks.iter().position(|t| t.name == task_name)
                    {
                        user_data.tasks[task_index].buy_slippage_percent = slippage;
                        save_user_data(&mut con, chat_id.0, &user_data).await?;

                        let task = &user_data.tasks[task_index];
                        let task_text = generate_task_detail_text(
                            redis_client.clone(),
                            chat_id.0,
                            task,
                            sol_price_state.clone(),
                        )
                        .await;
                        bot.edit_message_text(chat_id, menu_message_id, task_text)
                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                            .reply_markup(task_detail_keyboard(task))
                            .await?;
                    }
                }
                dialogue.update(State::TasksMenu).await?;
            }
            State::TaskReceiveBlacklist {
                task_name,
                menu_message_id,
                prompt_message_id,
            } => {
                bot.delete_message(chat_id, prompt_message_id).await.ok();
                let words: Vec<String> = text.split(',').map(|s| s.trim().to_lowercase()).collect();
                if let Some(task_index) = user_data.tasks.iter().position(|t| t.name == task_name) {
                    user_data.tasks[task_index].blacklist_words = words;
                    save_user_data(&mut con, chat_id.0, &user_data).await?;

                    let task = &user_data.tasks[task_index];
                    let task_text = generate_task_detail_text(
                        redis_client.clone(),
                        chat_id.0,
                        task,
                        sol_price_state.clone(),
                    )
                    .await;
                    bot.edit_message_text(chat_id, menu_message_id, task_text)
                        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                        .reply_markup(task_detail_keyboard(task))
                        .await?;
                }
                dialogue.update(State::TasksMenu).await?;
            }
            State::TaskReceiveDiscordToken {
                task_name,
                menu_message_id,
                prompt_message_id,
            } => {
                bot.delete_message(chat_id, prompt_message_id).await.ok();
                let token = text.trim().to_string();
                let token_slice = token.as_str();

                let duplicate_task_name = user_data
                    .tasks
                    .iter()
                    .find(|t| {
                        t.name != task_name
                            && t.discord_token
                                .as_deref()
                                .map(|existing| existing == token_slice)
                                .unwrap_or(false)
                    })
                    .map(|t| t.name.clone());

                if let Some(conflict_task_name) = duplicate_task_name {
                    let warning_message = bot
                        .send_message(
                            chat_id,
                            format!(
                                "⚠️ This Discord token is already assigned to task '{}'. Remove it there before reusing.",
                                conflict_task_name
                            ),
                        )
                        .await?;
                    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                    bot.delete_message(chat_id, warning_message.id).await.ok();
                } else {
                    let client = reqwest::Client::new();
                    let validate_result = client
                        .get("https://discord.com/api/v10/users/@me")
                        .header("Authorization", token.clone())
                        .send()
                        .await;

                    match validate_result {
                        Ok(response) if response.status().is_success() => {
                            let json = response.json::<serde_json::Value>().await;
                            let username = if let Ok(data) = json {
                                data["username"].as_str().unwrap_or("Unknown").to_string()
                            } else {
                                "Unknown".to_string()
                            };

                            if let Some(task_index) =
                                user_data.tasks.iter().position(|t| t.name == task_name)
                            {
                                user_data.tasks[task_index].discord_token = Some(token);
                                user_data.tasks[task_index].discord_username = Some(username);
                                save_user_data(&mut con, chat_id.0, &user_data).await?;

                                let task = &user_data.tasks[task_index];
                                let task_text = generate_task_detail_text(
                                    redis_client.clone(),
                                    chat_id.0,
                                    task,
                                    sol_price_state.clone(),
                                )
                                .await;
                                bot.edit_message_text(chat_id, menu_message_id, task_text)
                                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                                    .reply_markup(task_detail_keyboard(task))
                                    .await?;
                            }
                        }
                        _ => {
                            let error_msg = bot
                                .send_message(
                                    chat_id,
                                    "❌ Invalid Discord token. Please check and try again.",
                                )
                                .await?;
                            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                            bot.delete_message(chat_id, error_msg.id).await.ok();
                        }
                    }
                }
                dialogue.update(State::TasksMenu).await?;
            }
            State::TaskReceiveDiscordChannelId {
                task_name,
                menu_message_id,
                prompt_message_id,
            } => {
                bot.delete_message(chat_id, prompt_message_id).await.ok();
                let channel_id = text.trim().to_string();
                if let Some(task_index) = user_data.tasks.iter().position(|t| t.name == task_name) {
                    user_data.tasks[task_index].discord_channel_id = Some(channel_id);
                    save_user_data(&mut con, chat_id.0, &user_data).await?;

                    let task = &user_data.tasks[task_index];
                    let task_text = generate_task_detail_text(
                        redis_client.clone(),
                        chat_id.0,
                        task,
                        sol_price_state.clone(),
                    )
                    .await;
                    bot.edit_message_text(chat_id, menu_message_id, task_text)
                        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                        .reply_markup(task_detail_keyboard(task))
                        .await?;
                }
                dialogue.update(State::TasksMenu).await?;
            }
            State::TaskReceiveDiscordUsers {
                task_name,
                menu_message_id,
                prompt_message_id,
            } => {
                bot.delete_message(chat_id, prompt_message_id).await.ok();
                let usernames: Vec<String> = text
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if let Some(task_index) = user_data.tasks.iter().position(|t| t.name == task_name) {
                    user_data.tasks[task_index].discord_users = usernames;
                    save_user_data(&mut con, chat_id.0, &user_data).await?;

                    let task = &user_data.tasks[task_index];
                    let task_text = generate_task_detail_text(
                        redis_client.clone(),
                        chat_id.0,
                        task,
                        sol_price_state.clone(),
                    )
                    .await;
                    bot.edit_message_text(chat_id, menu_message_id, task_text)
                        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                        .reply_markup(task_detail_keyboard(task))
                        .await?;
                }
                dialogue.update(State::TasksMenu).await?;
            }
            State::ReceiveImportKey {
                menu_message_id,
                prompt_message_id,
            } => {
                let key_validation_result = bs58::decode(text)
                    .into_vec()
                    .map_err(anyhow::Error::new)
                    .and_then(|bytes| {
                        Keypair::try_from(bytes.as_slice()).map_err(anyhow::Error::new)
                    });

                match key_validation_result {
                    Ok(_) => {
                        bot.delete_message(chat_id, prompt_message_id).await.ok();
                        let new_prompt = bot
                            .send_message(
                                chat_id,
                                "Private key is valid. Please enter a name for this wallet.",
                            )
                            .await?;
                        dialogue
                            .update(State::ReceiveWalletName {
                                menu_message_id,
                                prompt_message_id: new_prompt.id,
                                private_key: text.to_string(),
                            })
                            .await?;
                    }
                    Err(_) => {
                        bot.delete_message(chat_id, prompt_message_id).await.ok();
                        let error_msg = bot
                            .send_message(chat_id, "Invalid private key. Please try again.")
                            .await?;
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                        bot.delete_message(chat_id, error_msg.id).await.ok();
                        dialogue.update(State::WalletsMenu).await?;
                    }
                }
            }
            State::ReceiveWalletName {
                menu_message_id,
                prompt_message_id,
                private_key,
            } => {
                let keypair = Keypair::from_base58_string(&private_key);
                let new_wallet = Wallet {
                    name: text.to_string(),
                    public_key: keypair.pubkey().to_string(),
                    private_key,
                };
                user_data.wallets.push(new_wallet);
                save_user_data(&mut con, chat_id.0, &user_data).await?;

                bot.delete_message(chat_id, prompt_message_id).await.ok();
                let wallets_text = generate_wallets_text(redis_client.clone(), chat_id.0).await;
                bot.edit_message_text(chat_id, menu_message_id, wallets_text)
                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                    .reply_markup(wallets_menu_keyboard(
                        &user_data.wallets,
                        user_data.default_wallet_index,
                    ))
                    .await?;
                dialogue.update(State::WalletsMenu).await?;
            }
            State::ReceiveSlippage {
                menu_message_id,
                prompt_message_id,
            } => {
                bot.delete_message(chat_id, prompt_message_id).await.ok();
                let mut success = false;
                if let Ok(slippage) = text.parse::<u32>() {
                    user_data.config.slippage_percent = slippage;
                    success = true;
                }
                if success {
                    save_user_data(&mut con, chat_id.0, &user_data).await?;
                    let new_settings_text =
                        generate_settings_text(redis_client.clone(), chat_id.0).await;
                    bot.edit_message_text(chat_id, menu_message_id, new_settings_text)
                        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                        .reply_markup(settings_menu_keyboard())
                        .await?;
                } else {
                    let error_msg = bot
                        .send_message(chat_id, "Invalid value. Please try again.")
                        .await?;
                    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                    bot.delete_message(chat_id, error_msg.id).await.ok();
                }
                dialogue.update(State::SettingsMenu).await?;
            }
            _ => {}
        }
    }
    Ok(())
}
