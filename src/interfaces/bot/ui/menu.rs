use crate::application::pricing::SolPriceState;
use crate::interfaces::bot::data::{Task, get_user_data};
use crate::{BLOOM_WS_CONNECTION, BloomWsConnectionStatus};
use redis::Client as RedisClient;

#[derive(Clone, Debug)]
pub struct WalletDisplayInfo {
    pub label: Option<String>,
    pub address: String,
    pub balance_sol: Option<f64>,
}

pub fn escape_markdown(text: &str) -> String {
    text.replace("\\", "\\\\")
        .replace("_", "\\_")
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

pub fn telegram_linking_intro_text() -> String {
    let path = escape_markdown("Telegram App > Settings > Devices > Link Desktop Device");
    [
        "📱 *Telegram User Linking*".to_string(),
        format!(r"You need to scan this QR code in the {}\.", path),
        r"⏳ The QR code is valid for *60 seconds*\. After that it expires\.".to_string(),
        r"Tap *Generate QR Code* when you are ready\.".to_string(),
    ]
    .join("\n\n")
}

pub fn telegram_linking_scan_text() -> String {
    let path = escape_markdown("Telegram App > Settings > Devices > Link Desktop Device");
    [
        "🕒 *Scan within 60 seconds*".to_string(),
        format!(
            r"Open the {} and scan the QR code that was just sent\.",
            path
        ),
        r"If it expires you can tap *Generate QR Code* again\.".to_string(),
    ]
    .join("\n\n")
}

pub fn telegram_linking_expired_text() -> String {
    [
        "⚠️ *QR code expired*".to_string(),
        r"Generate a new QR code when you are ready to try again\.".to_string(),
    ]
    .join("\n\n")
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
    sol_price_state: SolPriceState,
    selected_wallet: Option<&WalletDisplayInfo>,
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

    let price_guard = sol_price_state.read().await;
    let sol_price_value = (*price_guard).map(|value| value as f64);
    drop(price_guard);

    let (wallet_label, wallet_balance) =
        format_task_bloom_wallet(task, selected_wallet);
    let wallet_block = match wallet_balance {
        Some(balance) => format!("🏦 *Bloom Wallet:* `{}` `{}`", wallet_label, balance),
        None => format!("🏦 *Bloom Wallet:* `{}`", wallet_label),
    };
    let heading = format!(
        "🎯 *{}*",
        escape_markdown(&format!("Task Configuration - {}", task.name))
    );

    let platform_details = match task.platform {
        Platform::Telegram => {
            let has_user_session = task.has_telegram_user_session();
            let channel_name_str = if has_user_session {
                task.listen_channel_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "Not Set".to_string())
            } else {
                "Not Set".to_string()
            };
            let monitoring_str = if has_user_session {
                if task.listen_users.is_empty() && task.listen_usernames.is_empty() {
                    if task.telegram_channel_is_broadcast {
                        "Channel posts (no specific users)".to_string()
                    } else {
                        "Not Set".to_string()
                    }
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
                }
            } else {
                "Not Set".to_string()
            };
            let username_display = task.telegram_username_display().unwrap_or("N/A");
            format!(
                concat!(
                    "👤 *Telegram Username:* `{}`\n",
                    "📢 *Telegram Channel Name:* `{}`\n",
                    "👥 *Monitoring:* `{}`"
                ),
                escape_markdown(username_display),
                escape_markdown(&channel_name_str),
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

    let buy_amount_display = format_sol_with_usd(task.buy_amount_sol, sol_price_value);
    let buy_fee_display = format_sol_with_usd(task.buy_priority_fee_sol, sol_price_value);

    let ws_status = {
        let state = BLOOM_WS_CONNECTION.lock();
        state.status
    };

    let bloom_notice_line = if ws_status == BloomWsConnectionStatus::Connected {
        String::new()
    } else {
        format!(
            "{}\n\n",
            escape_markdown(
                "⚠️ Bloom WS connection unavailable. Buy success notifications may be delayed. Please verify via @BloomSolana_bot.",
            )
        )
    };

    format!(
        concat!(
            "{}\n",
            "\n",
            "📊 *Platform:* `{}`\n",
            "\n",
            "{}\n",
            "\n",
            "{}\n",
            "\n",
            "🚫 *Blacklist Words:* `{}`\n",
            "\n",
            "💰 *Fees & Slippage*\n",
            "• *Buy Amount:* `{}`\n",
            "• *Buy Fee:* `{}`\n",
            "• *Buy Slippage:* `{}%`\n",
            "\n",
            "{}{}"
        ),
        heading,
        escape_markdown(platform_str),
        wallet_block,
        platform_details,
        escape_markdown(&blacklist_str),
        escape_markdown(&buy_amount_display),
        escape_markdown(&buy_fee_display),
        escape_markdown(&task.buy_slippage_percent.to_string()),
        inform_only_line,
        bloom_notice_line
    )
}

pub async fn generate_task_settings_text(
    _redis_client: RedisClient,
    _chat_id: i64,
    task: &Task,
    _sol_price_state: SolPriceState,
    selected_wallet: Option<&WalletDisplayInfo>,
) -> String {
    let (wallet_label, wallet_balance) =
        format_task_bloom_wallet(task, selected_wallet);
    let wallet_line = match wallet_balance {
        Some(balance) => format!("🏦 *Bloom Wallet:* `{}` `{}`", wallet_label, balance),
        None => format!("🏦 *Bloom Wallet:* `{}`", wallet_label),
    };
    let mut sections = vec![format!(
        "⚙️ *Task Settings: {}*",
        escape_markdown(&task.name)
    )];
    sections.push(wallet_line);
    let has_telegram_session = task.has_telegram_user_session();
    let telegram_status = if has_telegram_session {
        task.telegram_username_display()
            .map(|value| format!("Configured {}", value))
            .unwrap_or_else(|| "Configured".to_string())
    } else {
        "Not set".to_string()
    };
    sections.push(format!(
        "🤖 *Telegram User:* `{}`",
        escape_markdown(&telegram_status)
    ));
    let has_token = task
        .discord_token
        .as_ref()
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false);
    let token_status = if has_token {
        let username_display = task
            .discord_username
            .as_deref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| format!("Configured (@{})", value))
            .unwrap_or_else(|| "Configured".to_string());
        username_display
    } else {
        "Not set".to_string()
    };
    sections.push(format!(
        "🔑 *Discord Token:* `{}`",
        escape_markdown(&token_status)
    ));
    sections.push(escape_markdown(
        "Choose an option below to configure this task.",
    ));
    sections.join("\n\n")
}

pub fn generate_task_wallets_text(
    task: &Task,
    selected_wallet: Option<&WalletDisplayInfo>,
    displayed_wallets: &[WalletDisplayInfo],
    sol_price: Option<f64>,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "👛 *Bloom Wallets · {}*",
        escape_markdown(&task.name)
    ));
    lines.push(String::new());

    let selected_in_page = selected_wallet
        .map(|wallet| {
            displayed_wallets
                .iter()
                .any(|entry| entry.address == wallet.address)
        })
        .unwrap_or(false);

    match (selected_wallet, selected_in_page) {
        (Some(wallet), false) => {
            lines.extend(build_wallet_entry_lines(wallet, "▪️", sol_price));
            lines.push(String::new());
        }
        (None, _) => {
            lines.push(escape_markdown("No wallet selected."));
            lines.push(String::new());
        }
        _ => {}
    }

    for (index, wallet) in displayed_wallets.iter().enumerate() {
        let is_selected = selected_wallet
            .map(|selected| selected.address == wallet.address)
            .unwrap_or(false);
        let marker = if is_selected { "▪️" } else { "▫️" };
        lines.extend(build_wallet_entry_lines(wallet, marker, sol_price));
        if index + 1 < displayed_wallets.len() {
            lines.push(String::new());
        }
    }

    lines.push(String::new());
    lines.push(escape_markdown(
        "Select a wallet below to assign it to this task.",
    ));
    lines.join(
        "
",
    )
}

fn build_wallet_entry_lines(
    wallet: &WalletDisplayInfo,
    marker: &str,
    sol_price: Option<f64>,
) -> Vec<String> {
    let mut lines = Vec::new();
    let label_text = escape_markdown(&format_wallet_label(
        wallet.label.as_deref(),
        &wallet.address,
    ));
    let address_text = escape_markdown(&wallet.address);
    let balance_text = escape_markdown(&format_wallet_balance_text(wallet.balance_sol, sol_price));
    lines.push(format!("{} {}", marker, label_text));
    lines.push(format!("   🔑 {}", address_text));
    lines.push(format!("   💰 {}", balance_text));
    lines
}

fn format_task_bloom_wallet(
    task: &Task,
    selected_wallet: Option<&WalletDisplayInfo>,
) -> (String, Option<String>) {
    if let Some(wallet) = selected_wallet {
        let label = format_wallet_label(wallet.label.as_deref(), &wallet.address);
        let label = escape_markdown(&label);
        let balance = wallet.balance_sol.map(|amount| {
            let sol_text = format_trimmed_sol(amount);
            escape_markdown(&format!("{} SOL", sol_text))
        });
        return (label, balance);
    }
    if let Some(wallet) = task.bloom_wallet.as_ref() {
        let label = format_wallet_label(wallet.label.as_deref(), &wallet.address);
        return (escape_markdown(&label), None);
    }
    (escape_markdown("Not set"), None)
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

fn format_sol_with_usd(amount: f64, price_per_sol: Option<f64>) -> String {
    let sol_text = format_trimmed_sol(amount);
    if let Some(price) = price_per_sol {
        let usd_value = amount * price;
        let usd_text = format!("{:.2}", usd_value);
        format!("{} SOL (${usd_text})", sol_text)
    } else {
        format!("{} SOL", sol_text)
    }
}

fn format_wallet_balance_text(balance_sol: Option<f64>, price_per_sol: Option<f64>) -> String {
    match balance_sol {
        Some(amount) => format_sol_with_usd(amount, price_per_sol),
        None => "Balance unavailable".to_string(),
    }
}

fn format_trimmed_sol(amount: f64) -> String {
    if amount == 0.0 {
        return "0".to_string();
    }
    let mut text = format!("{:.6}", amount);
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text.is_empty() {
        "0".to_string()
    } else {
        text
    }
}
