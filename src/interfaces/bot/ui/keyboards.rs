use crate::interfaces::bot::data::{BloomWalletInfo, Task};
use crate::interfaces::bot::ui::State;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

pub const ITEMS_PER_PAGE: usize = 5;

pub fn tasks_menu_keyboard(tasks: &Vec<Task>) -> InlineKeyboardMarkup {
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

    for task_chunk in tasks.chunks(2) {
        let row = task_chunk
            .iter()
            .map(|task| {
                let status = if task.active { "🟢" } else { "🔴" };
                let text = format!("{} {}", task.name, status);
                InlineKeyboardButton::callback(text, format!("task_detail_{}", task.name))
            })
            .collect();
        buttons.push(row);
    }

    buttons.push(vec![InlineKeyboardButton::callback(
        "➕ Create New Task",
        "create_task",
    )]);
    InlineKeyboardMarkup::new(buttons)
}

pub fn task_detail_keyboard(task: &Task) -> InlineKeyboardMarkup {
    use crate::interfaces::bot::data::types::Platform;

    let active_status_icon = if task.active { "🟢" } else { "🔴" };
    let inform_only_icon = if task.inform_only { "🟢" } else { "🔴" };

    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

    buttons.push(vec![InlineKeyboardButton::callback(
        format!("📝 Task Name: {}", task.name),
        format!("task_name_{}", task.name),
    )]);

    buttons.push(vec![InlineKeyboardButton::callback(
        "⚙️ Task Settings",
        format!("task_settings_{}", task.name),
    )]);

    let telegram_text = if task.platform == Platform::Telegram {
        "✅ Telegram".to_string()
    } else {
        "Telegram".to_string()
    };
    let discord_text = if task.platform == Platform::Discord {
        "✅ Discord".to_string()
    } else {
        "Discord".to_string()
    };
    buttons.push(vec![
        InlineKeyboardButton::callback(
            telegram_text,
            format!("task_platform_telegram_{}", task.name),
        ),
        InlineKeyboardButton::callback(
            discord_text,
            format!("task_platform_discord_{}", task.name),
        ),
    ]);

    buttons.push(vec![
        InlineKeyboardButton::callback(
            format!("Buy: {} SOL", task.buy_amount_sol),
            format!("task_buy_amount_{}", task.name),
        ),
        InlineKeyboardButton::callback(
            format!("Fee: {} SOL", task.buy_priority_fee_sol),
            format!("task_buy_fee_{}", task.name),
        ),
        InlineKeyboardButton::callback(
            format!("Slippage: {}%", task.buy_slippage_percent),
            format!("task_slippage_{}", task.name),
        ),
    ]);

    if task.platform == Platform::Telegram {
        let has_session = task.has_telegram_user_session();
        buttons.push(vec![InlineKeyboardButton::callback(
            "👥 Telegram Users to Monitor",
            format!("task_users_{}", task.name),
        )]);
        let has_channel = has_session
            && task
                .listen_channel_name
                .as_ref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or_else(|| !task.listen_channels.is_empty());
        let channel_button_text = if has_session {
            if has_channel {
                "📢 Change Channel".to_string()
            } else {
                "📢 Set Channel".to_string()
            }
        } else {
            "📢 Set Channel".to_string()
        };
        buttons.push(vec![
            InlineKeyboardButton::callback(
                channel_button_text,
                format!("task_channels_{}", task.name),
            ),
            InlineKeyboardButton::callback(
                format!("{} Inform Only", inform_only_icon),
                format!("task_toggle_inform_{}", task.name),
            ),
        ]);
    } else {
        buttons.push(vec![InlineKeyboardButton::callback(
            "👥 Discord Users to Monitor",
            format!("task_discord_users_{}", task.name),
        )]);
        let has_discord_channel = task
            .discord_channel_id
            .as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        let discord_channel_button = if has_discord_channel {
            "📢 Change Channel ID"
        } else {
            "📢 Set Channel ID"
        };
        buttons.push(vec![
            InlineKeyboardButton::callback(
                discord_channel_button,
                format!("task_discord_channel_{}", task.name),
            ),
            InlineKeyboardButton::callback(
                format!("{} Inform Only", inform_only_icon),
                format!("task_toggle_inform_{}", task.name),
            ),
        ]);
    }

    buttons.push(vec![InlineKeyboardButton::callback(
        format!("🚫 Blacklist Words ({})", task.blacklist_words.len()),
        format!("task_blacklist_{}", task.name),
    )]);

    buttons.push(vec![
        InlineKeyboardButton::callback(
            format!("{} Active", active_status_icon),
            format!("task_toggle_{}", task.name),
        ),
        InlineKeyboardButton::callback("🗑️ Delete", format!("task_delete_{}", task.name)),
    ]);

    buttons.push(vec![InlineKeyboardButton::callback("← Back", "view_tasks")]);

    InlineKeyboardMarkup::new(buttons)
}

pub fn task_delete_confirmation_keyboard(task_name: &str) -> InlineKeyboardMarkup {
    let clean_task_name = task_name
        .strip_prefix("task_delete_confirm_")
        .unwrap_or(task_name);
    let buttons = vec![
        vec![InlineKeyboardButton::callback(
            "✅ Yes, delete",
            format!("task_delete_confirm_{}", clean_task_name),
        )],
        vec![InlineKeyboardButton::callback(
            "❌ Cancel",
            format!("task_detail_{}", clean_task_name),
        )],
    ];
    InlineKeyboardMarkup::new(buttons)
}

pub async fn channel_selection_keyboard(state: &State) -> Option<InlineKeyboardMarkup> {
    if let State::TaskSelectChannelFromList {
        task_name,
        all_channels,
        page,
        ..
    } = state
    {
        let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
        let start = page * ITEMS_PER_PAGE;
        let end = (start + ITEMS_PER_PAGE).min(all_channels.len());

        for (name, id) in &all_channels[start..end] {
            buttons.push(vec![InlineKeyboardButton::callback(
                name,
                format!("task_chan_select_{}_{}", task_name, id),
            )]);
        }

        let mut nav_row = Vec::new();
        if *page > 0 {
            nav_row.push(InlineKeyboardButton::callback(
                "< Prev",
                format!("task_chan_page_{}_{}", task_name, page - 1),
            ));
        }
        if end < all_channels.len() {
            nav_row.push(InlineKeyboardButton::callback(
                "Next >",
                format!("task_chan_page_{}_{}", task_name, page + 1),
            ));
        }
        if !nav_row.is_empty() {
            buttons.push(nav_row);
        }

        buttons.push(vec![InlineKeyboardButton::callback(
            "← Cancel",
            format!("task_chan_cancel_{}", task_name),
        )]);
        Some(InlineKeyboardMarkup::new(buttons))
    } else {
        None
    }
}

pub async fn user_selection_keyboard(state: &State) -> Option<InlineKeyboardMarkup> {
    if let State::TaskSelectUsersFromList {
        task_name,
        all_users,
        selected_users,
        page,
        ..
    } = state
    {
        let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
        let start = page * ITEMS_PER_PAGE;
        let end = (start + ITEMS_PER_PAGE).min(all_users.len());

        for (name, id, role) in &all_users[start..end] {
            let check = if selected_users.contains(id) {
                "✅"
            } else {
                " "
            };
            let text = format!("{} {} - {}", check, name, role);
            buttons.push(vec![InlineKeyboardButton::callback(
                text,
                format!("task_user_toggle_{}_{}", task_name, id),
            )]);
        }

        let mut nav_row = Vec::new();
        if *page > 0 {
            nav_row.push(InlineKeyboardButton::callback(
                "< Prev",
                format!("task_user_page_{}_{}", task_name, page - 1),
            ));
        }
        if end < all_users.len() {
            nav_row.push(InlineKeyboardButton::callback(
                "Next >",
                format!("task_user_page_{}_{}", task_name, page + 1),
            ));
        }
        if !nav_row.is_empty() {
            buttons.push(nav_row);
        }

        buttons.push(vec![InlineKeyboardButton::callback(
            "← Back",
            format!("task_detail_{}", task_name),
        )]);
        Some(InlineKeyboardMarkup::new(buttons))
    } else {
        None
    }
}

pub fn task_settings_keyboard(task: &Task) -> InlineKeyboardMarkup {
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    buttons.push(vec![InlineKeyboardButton::callback(
        "🌸 Bloom Wallets",
        format!("task_settings_wallets_{}", task.name),
    )]);
    let has_user = task.has_telegram_user_session();
    let telegram_label = if has_user {
        "🤖 Update Telegram User"
    } else {
        "🤖 Set Telegram User"
    };
    buttons.push(vec![InlineKeyboardButton::callback(
        telegram_label,
        format!("task_telegram_user_{}", task.name),
    )]);

    let has_token = task
        .discord_token
        .as_ref()
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false);
    let discord_label = if has_token {
        "🔑 Update Discord Token"
    } else {
        "🔑 Set Discord Token"
    };
    buttons.push(vec![InlineKeyboardButton::callback(
        discord_label,
        format!("task_discord_token_{}", task.name),
    )]);
    buttons.push(vec![InlineKeyboardButton::callback(
        "← Back to Task",
        format!("task_detail_{}", task.name),
    )]);
    InlineKeyboardMarkup::new(buttons)
}

pub fn task_wallets_keyboard(
    task_name: &str,
    wallets: &[BloomWalletInfo],
    selected_address: Option<&str>,
    page: usize,
) -> InlineKeyboardMarkup {
    let start = page * ITEMS_PER_PAGE;
    let end = (start + ITEMS_PER_PAGE).min(wallets.len());
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

    for (index, wallet) in wallets[start..end].iter().enumerate() {
        let absolute_index = start + index;
        let is_selected = selected_address
            .map(|address| address == wallet.address.as_str())
            .unwrap_or(false);
        let icon = if is_selected { "✅" } else { "☑️" };
        let short_address = shorten_wallet_address(&wallet.address);
        let label = wallet
            .label
            .as_deref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .unwrap_or_else(|| short_address.clone());
        let display_text = if label == short_address {
            label
        } else {
            format!("{} ({})", label, short_address)
        };
        let button_text = format!("{} {}", icon, display_text);
        buttons.push(vec![InlineKeyboardButton::callback(
            button_text,
            format!(
                "task_wallet_select:{}:{}:{}",
                task_name, page, absolute_index
            ),
        )]);
    }

    if selected_address.is_some() {
        buttons.push(vec![InlineKeyboardButton::callback(
            "Clear Selection",
            format!("task_wallet_clear:{}", task_name),
        )]);
    }

    if wallets.len() > ITEMS_PER_PAGE {
        let mut nav_row = Vec::new();
        if page > 0 {
            nav_row.push(InlineKeyboardButton::callback(
                "< Prev",
                format!("task_wallet_page:{}:{}", task_name, page - 1),
            ));
        }
        if end < wallets.len() {
            nav_row.push(InlineKeyboardButton::callback(
                "Next >",
                format!("task_wallet_page:{}:{}", task_name, page + 1),
            ));
        }
        if !nav_row.is_empty() {
            buttons.push(nav_row);
        }
    }

    buttons.push(vec![InlineKeyboardButton::callback(
        "← Back",
        format!("task_settings_{}", task_name),
    )]);

    InlineKeyboardMarkup::new(buttons)
}

pub fn task_telegram_linking_keyboard(task_name: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "🔄 Generate QR Code",
            format!("task_telegram_link_generate_{}", task_name),
        )],
        vec![InlineKeyboardButton::callback(
            "✖️ Cancel Telegram Linking",
            format!("task_telegram_link_cancel_{}", task_name),
        )],
    ])
}

pub fn task_telegram_confirm_keyboard(task_name: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "✅ Yes, this is my account",
            format!("task_telegram_link_confirm_yes_{}", task_name),
        )],
        vec![InlineKeyboardButton::callback(
            "♻️ No, try another account",
            format!("task_telegram_link_confirm_no_{}", task_name),
        )],
        vec![InlineKeyboardButton::callback(
            "✖️ Cancel Telegram Linking",
            format!("task_telegram_link_cancel_{}", task_name),
        )],
    ])
}

fn shorten_wallet_address(address: &str) -> String {
    const PREFIX: usize = 6;
    const SUFFIX: usize = 4;
    if address.len() <= PREFIX + SUFFIX {
        address.to_string()
    } else {
        let prefix = &address[..PREFIX];
        let suffix = &address[address.len() - SUFFIX..];
        format!("{}...{}", prefix, suffix)
    }
}

pub fn token_info_keyboard(_mint: &str) -> InlineKeyboardMarkup {
    let buttons = vec![vec![InlineKeyboardButton::callback("↻ Refresh", "r")]];
    InlineKeyboardMarkup::new(buttons)
}
