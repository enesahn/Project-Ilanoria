use crate::application::pricing::SolPriceState;
use crate::infrastructure::blockchain::RpcClients;
use crate::interfaces::bot::data::{Task, get_user_data};
use redis::Client as RedisClient;

pub fn escape_markdown(text: &str) -> String {
    text.replace("_", "\\_")
        .replace("*", "\\*")
        .replace("[", "\\[")
        .replace("]", "\\]")
        .replace("(", "\\(")
        .replace(")", "\\)")
        .replace("~", "\\~")
        .replace("`", "\\`")
        .replace(">", "\\>")
        .replace("#", "\\#")
        .replace("+", "\\+")
        .replace("-", "\\-")
        .replace("=", "\\=")
        .replace("|", "\\|")
        .replace("{", "\\{")
        .replace("}", "\\}")
        .replace(".", "\\.")
        .replace("!", "\\!")
}

pub async fn generate_main_menu_text(
    redis_client: RedisClient,
    chat_id: i64,
    sol_price_state: SolPriceState,
    rpc_clients: RpcClients,
) -> String {
    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    match get_user_data(&mut con, chat_id).await {
        Ok(Some(user_data)) => {
            if let Some(default_wallet) = user_data.get_default_wallet() {
                let pubkey_str = &default_wallet.public_key;
                let rpc_client = &rpc_clients.quicknode_client;
                let pubkey = solana_sdk::pubkey::Pubkey::try_from(pubkey_str.as_str()).unwrap();

                let (balance_lamports, balance_str) = match rpc_client.get_balance(&pubkey).await {
                    Ok(lamports) => {
                        let sol_balance = lamports as f64 / 1_000_000_000.0;
                        (lamports, format!("{:.4} SOL", sol_balance))
                    }
                    Err(_) => (0, "Error".to_string()),
                };

                let price_guard = sol_price_state.read().await;
                let usd_value_str = match *price_guard {
                    Some(price) => {
                        let sol_balance = balance_lamports as f64 / 1_000_000_000.0;
                        let usd_value = sol_balance * price as f64;
                        let formatted_usd = format!("{:.2}", usd_value);
                        let escaped_usd = escape_markdown(&formatted_usd);
                        format!(" \\(${}\\)", escaped_usd)
                    }
                    None => "".to_string(),
                };

                format!(
                    "🏠 *Main Menu*\n\n*Default Wallet: {}*\n`{}`\n\n*Balance:* `{}`{}",
                    escape_markdown(&default_wallet.name),
                    escape_markdown(pubkey_str),
                    balance_str,
                    usd_value_str
                )
            } else {
                "⚠️ No default wallet found. Please create or import a wallet.".to_string()
            }
        }
        _ => "⚠️ Could not load data. Please run /start again.".to_string(),
    }
}

pub async fn generate_tasks_text(redis_client: RedisClient, chat_id: i64) -> String {
    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    match get_user_data(&mut con, chat_id).await {
        Ok(Some(user_data)) => {
            let mut text = "📋 *Your Tasks*\n\n".to_string();
            if user_data.tasks.is_empty() {
                text.push_str(&escape_markdown("No tasks created yet."));
            } else {
                text.push_str(&escape_markdown("Select a task to view details or edit."));
            }
            text
        }
        _ => escape_markdown("⚠️ Could not load tasks."),
    }
}

pub async fn generate_task_detail_text(
    _redis_client: RedisClient,
    _chat_id: i64,
    task: &Task,
) -> String {
    use crate::interfaces::bot::data::types::Platform;

    let blacklist_str = if task.blacklist_words.is_empty() {
        "Not Set".to_string()
    } else {
        task.blacklist_words.join(", ")
    };

    let inform_only_line = if task.inform_only {
        "🔔 *Inform Only Mode Active*\n\n".to_string()
    } else {
        "".to_string()
    };

    let platform_str = match task.platform {
        Platform::Telegram => "Telegram",
        Platform::Discord => "Discord",
    };

    let bloom_wallet_display = format_task_bloom_wallet(task);

    let platform_details = match task.platform {
        Platform::Telegram => {
            let channel_id_str = task
                .listen_channels
                .first()
                .map_or("Not Set".to_string(), |id| id.to_string());
            let channel_name_str = task.listen_channel_name.as_deref().unwrap_or("N/A");
            let monitoring_str = if task.listen_users.is_empty() && task.listen_usernames.is_empty()
            {
                "Not Set".to_string()
            } else if !task.listen_usernames.is_empty() {
                format!(
                    "{} users: {}",
                    task.listen_usernames.len(),
                    task.listen_usernames.join(", ")
                )
            } else {
                let users = task
                    .listen_users
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} users: {}", task.listen_users.len(), users)
            };
            format!(
                concat!(
                    "📢 *Telegram Channel:* `{}`\n",
                    "📌 *Channel Name:* `{}`\n",
                    "👥 *Monitoring:* `{}`"
                ),
                escape_markdown(&channel_id_str),
                escape_markdown(channel_name_str),
                escape_markdown(&monitoring_str)
            )
        }
        Platform::Discord => {
            let channel_id_str = task.discord_channel_id.as_deref().unwrap_or("Not Set");
            let username_str = match task.discord_username.as_deref() {
                Some(username) if !username.trim().is_empty() => username.to_string(),
                _ => "N/A".to_string(),
            };
            let users_count = task.discord_users.len();
            let users_str = if users_count == 0 {
                "Not Set".to_string()
            } else {
                format!("{} users: {}", users_count, task.discord_users.join(", "))
            };
            format!(
                concat!(
                    "👤 *Discord Username:* `{}`\n",
                    "📢 *Discord Channel ID:* `{}`\n",
                    "👥 *Monitoring:* `{}`"
                ),
                escape_markdown(&username_str),
                escape_markdown(channel_id_str),
                escape_markdown(&users_str)
            )
        }
    };

    format!(
        "🎯 *Task Configuration \\- {}*\n
\
        📊 *Platform:* `{}`\n
\
        💼 *Bloom Wallet:* `{}`\n
\
        {}\n
\
        📝 *Blacklist Words:* `{}`\n
\
        💰 *Fees & Slippage*
\
        • *Buy Amount:* `{} SOL`
\
        • *Buy Fee:* `{} SOL`
\
        • *Buy Slippage:* `{}%`\n
\
        {}\
        *🟢: The feature/mode is turned ON*
\
        *🔴: The feature/mode is turned OFF*",
        escape_markdown(&task.name),
        escape_markdown(platform_str),
        escape_markdown(&bloom_wallet_display),
        platform_details,
        escape_markdown(&blacklist_str),
        escape_markdown(&task.buy_amount_sol.to_string()),
        escape_markdown(&task.buy_priority_fee_sol.to_string()),
        escape_markdown(&task.buy_slippage_percent.to_string()),
        inform_only_line
    )
}

pub async fn generate_task_settings_text(
    _redis_client: RedisClient,
    _chat_id: i64,
    task: &Task,
) -> String {
    let bloom_wallet_display = format_task_bloom_wallet(task);
    format!(
        "⚙️ *Task Settings: {}*\n\n💼 *Bloom Wallet:* `{}`\n\nChoose an option below to configure this task\\.",
        escape_markdown(&task.name),
        escape_markdown(&bloom_wallet_display)
    )
}

pub fn generate_task_wallets_text(task: &Task, current_display: &str) -> String {
    format!(
        "👛 *Bloom Wallets: {}*\n\nCurrent selection: `{}`\n\nSelect a wallet below to assign it to this task\\.",
        escape_markdown(&task.name),
        escape_markdown(current_display)
    )
}

pub async fn generate_wallets_text(redis_client: RedisClient, chat_id: i64) -> String {
    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    match get_user_data(&mut con, chat_id).await {
        Ok(Some(user_data)) => {
            let mut text = "👛 *Your Wallets*\n\n".to_string();
            for (i, wallet) in user_data.wallets.iter().enumerate() {
                let icon = if i == user_data.default_wallet_index {
                    "✅"
                } else {
                    "☑️"
                };
                text.push_str(&format!(
                    "*{} {}*\n`{}`\n\n",
                    icon,
                    escape_markdown(&wallet.name),
                    escape_markdown(&wallet.public_key)
                ));
            }
            text
        }
        _ => "⚠️ Could not load wallets.".to_string(),
    }
}

pub async fn generate_settings_text(redis_client: RedisClient, chat_id: i64) -> String {
    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    match get_user_data(&mut con, chat_id).await {
        Ok(Some(user_data)) => {
            let config = user_data.config;
            let slippage = escape_markdown(&config.slippage_percent.to_string());
            let buy_priority_fee = escape_markdown(&config.buy_priority_fee_sol.to_string());
            let sell_priority_fee = escape_markdown(&config.sell_priority_fee_sol.to_string());

            format!(
                "⚙️ *Current Settings:*\n\n*Slippage:* `{}%`\n*Buy Priority Fee:* `{}` SOL\n*Sell Priority Fee:* `{}` SOL",
                slippage, buy_priority_fee, sell_priority_fee
            )
        }
        _ => "⚠️ Could not load settings. Please run /start again.".to_string(),
    }
}

fn format_task_bloom_wallet(task: &Task) -> String {
    if let Some(wallet) = task.bloom_wallet.as_ref() {
        return format_wallet_label(wallet.label.as_deref(), &wallet.address);
    }
    "Not set".to_string()
}

fn format_wallet_label(label: Option<&str>, address: &str) -> String {
    let trimmed_label = label.map(|value| value.trim()).unwrap_or("");
    let short_address = shorten_pubkey(address);
    if trimmed_label.is_empty() {
        short_address
    } else {
        format!("{} ({})", trimmed_label, short_address)
    }
}

fn shorten_pubkey(pubkey: &str) -> String {
    const PREFIX: usize = 6;
    const SUFFIX: usize = 4;
    if pubkey.len() <= PREFIX + SUFFIX {
        pubkey.to_string()
    } else {
        let prefix = &pubkey[..PREFIX];
        let suffix = &pubkey[pubkey.len() - SUFFIX..];
        format!("{}...{}", prefix, suffix)
    }
}
