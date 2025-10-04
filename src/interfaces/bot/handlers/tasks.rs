use crate::USER_CLIENT_HANDLE;
use crate::application::pricing::SolPriceState;
use crate::infrastructure::blockchain::RpcClients;
use crate::interfaces::bot::WalletDisplayInfo;
use crate::interfaces::bot::data::BloomWalletInfo;
use crate::interfaces::bot::data::types::Platform;
use crate::interfaces::bot::user::client::{
    UserClientHandle, authenticate_task_user_via_qr, get_chat_admins, is_channel_member,
    take_active_qr_message, take_pending_session,
};
use crate::interfaces::bot::utils::fetch_bloom_wallets;
use crate::interfaces::bot::{
    ITEMS_PER_PAGE, State, Task, channel_selection_keyboard, generate_task_detail_text,
    generate_task_settings_text, generate_task_wallets_text, generate_tasks_text, get_user_data,
    save_user_data, send_cleanup_msg, task_delete_confirmation_keyboard, task_detail_keyboard,
    task_settings_keyboard, task_telegram_linking_keyboard, task_wallets_keyboard,
    tasks_menu_keyboard, telegram_linking_intro_text, user_selection_keyboard,
};
use grammers_client::Client as TelegramClient;
use parking_lot::Mutex;
use rand::Rng;
use redis::Client as RedisClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::MessageId;
use tokio::sync::mpsc::Sender;

type MyDialogue = Dialogue<State, teloxide::dispatching::dialogue::InMemStorage<State>>;
type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

fn generate_random_task_name() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::rng();
    (0..6)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

pub async fn handle_task_callbacks(
    q: CallbackQuery,
    bot: Bot,
    redis_client: RedisClient,
    dialogue: MyDialogue,
    sol_price_state: SolPriceState,
    user_client_handle: Arc<Mutex<Option<UserClientHandle>>>,
    rpc_clients: RpcClients,
    client_sender: Sender<TelegramClient>,
) -> HandlerResult {
    if let Some(message) = q.message.clone() {
        let chat_id = message.chat.id;
        let data = q.data.clone().unwrap_or_default();

        log::info!(
            "[TASK_CALLBACK] Received data: '{}' from ChatID: {}",
            data,
            chat_id
        );

        if data == "view_tasks" {
            let tasks_text = generate_tasks_text(redis_client.clone(), chat_id.0).await;
            let tasks = get_tasks(redis_client.clone(), chat_id.0).await;
            bot.edit_message_text(chat_id, message.id, tasks_text)
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                .reply_markup(tasks_menu_keyboard(&tasks))
                .await?;
            dialogue.update(State::TasksMenu).await?;
        } else if data == "create_task" {
            let mut con = redis_client.get_multiplexed_async_connection().await?;
            if let Some(mut user_data) = get_user_data(&mut con, chat_id.0).await? {
                let task_name = generate_random_task_name();
                let new_task = Task {
                    name: task_name.clone(),
                    platform: crate::interfaces::bot::data::types::Platform::Telegram,
                    listen_channels: vec![],
                    listen_channel_name: None,
                    listen_users: vec![],
                    listen_usernames: vec![],
                    telegram_channel_is_broadcast: false,
                    grammers_session_data: None,
                    telegram_username: None,
                    discord_token: None,
                    discord_channel_id: None,
                    discord_username: None,
                    discord_users: vec![],
                    active: false,
                    buy_amount_sol: 0.001,
                    buy_priority_fee_sol: 0.001,
                    buy_slippage_percent: 20,
                    blacklist_words: vec![],
                    inform_only: false,
                    bloom_wallet: None,
                };
                user_data.tasks.push(new_task);
                save_user_data(&mut con, chat_id.0, &user_data).await?;
                bot.answer_callback_query(q.id)
                    .text(&format!("✅ New task created: {}", task_name))
                    .await?;
            }
            let tasks_text = generate_tasks_text(redis_client.clone(), chat_id.0).await;
            let tasks = get_tasks(redis_client.clone(), chat_id.0).await;
            bot.edit_message_text(chat_id, message.id, tasks_text)
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                .reply_markup(tasks_menu_keyboard(&tasks))
                .await?;
            dialogue.update(State::TasksMenu).await?;
        } else if let Some(payload) = data.strip_prefix("task_wallet_select:") {
            if let Some(state) = dialogue.get().await?.clone() {
                if let State::TaskSelectBloomWallet {
                    task_name,
                    menu_message_id,
                    wallets,
                    page,
                } = state
                {
                    let mut segments = payload.split(':');
                    let payload_task = segments.next().unwrap_or_default();
                    let requested_page = segments
                        .next()
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(page);
                    let selected_index = segments
                        .next()
                        .and_then(|value| value.parse::<usize>().ok());
                    if payload_task == task_name {
                        if let Some(index) = selected_index {
                            if let Some(wallet) = wallets.get(index) {
                                if persist_task_wallet_selection(
                                    &redis_client,
                                    chat_id.0,
                                    &task_name,
                                    Some(wallet),
                                )
                                .await?
                                .is_some()
                                {
                                    bot.answer_callback_query(q.id.clone())
                                        .text("✅ Bloom wallet updated.")
                                        .await?;
                                }
                            }
                        }
                        render_task_wallets_view(
                            &bot,
                            redis_client.clone(),
                            chat_id,
                            menu_message_id,
                            &task_name,
                            &wallets,
                            requested_page,
                            sol_price_state.clone(),
                            rpc_clients.clone(),
                        )
                        .await?;
                        dialogue
                            .update(State::TaskSelectBloomWallet {
                                task_name,
                                menu_message_id,
                                wallets,
                                page: requested_page,
                            })
                            .await?;
                    }
                }
            }
        } else if let Some(payload) = data.strip_prefix("task_wallet_page:") {
            if let Some(state) = dialogue.get().await?.clone() {
                if let State::TaskSelectBloomWallet {
                    task_name,
                    menu_message_id,
                    wallets,
                    ..
                } = state
                {
                    let mut segments = payload.split(':');
                    let payload_task = segments.next().unwrap_or_default();
                    if payload_task == task_name {
                        if let Some(new_page) = segments
                            .next()
                            .and_then(|value| value.parse::<usize>().ok())
                        {
                            render_task_wallets_view(
                                &bot,
                                redis_client.clone(),
                                chat_id,
                                menu_message_id,
                                &task_name,
                                &wallets,
                                new_page,
                                sol_price_state.clone(),
                                rpc_clients.clone(),
                            )
                            .await?;
                            dialogue
                                .update(State::TaskSelectBloomWallet {
                                    task_name,
                                    menu_message_id,
                                    wallets,
                                    page: new_page,
                                })
                                .await?;
                        }
                    }
                }
            }
        } else if let Some(task_name_payload) = data.strip_prefix("task_wallet_clear:") {
            if let Some(state) = dialogue.get().await?.clone() {
                if let State::TaskSelectBloomWallet {
                    task_name,
                    menu_message_id,
                    wallets,
                    page,
                } = state
                {
                    if task_name_payload == task_name {
                        let cleared = persist_task_wallet_selection(
                            &redis_client,
                            chat_id.0,
                            &task_name,
                            None,
                        )
                        .await?
                        .is_some();
                        if cleared {
                            bot.answer_callback_query(q.id.clone())
                                .text("✅ Bloom wallet cleared.")
                                .await?;
                        }
                        render_task_wallets_view(
                            &bot,
                            redis_client.clone(),
                            chat_id,
                            menu_message_id,
                            &task_name,
                            &wallets,
                            page,
                            sol_price_state.clone(),
                            rpc_clients.clone(),
                        )
                        .await?;
                        dialogue
                            .update(State::TaskSelectBloomWallet {
                                task_name,
                                menu_message_id,
                                wallets,
                                page,
                            })
                            .await?;
                    }
                }
            }
        } else if let Some(task_name) = data.strip_prefix("task_settings_wallets_") {
            match fetch_bloom_wallets(None).await {
                Ok(wallets) => {
                    render_task_wallets_view(
                        &bot,
                        redis_client.clone(),
                        chat_id,
                        message.id,
                        task_name,
                        &wallets,
                        0,
                        sol_price_state.clone(),
                        rpc_clients.clone(),
                    )
                    .await?;
                    dialogue
                        .update(State::TaskSelectBloomWallet {
                            task_name: task_name.to_string(),
                            menu_message_id: message.id,
                            wallets,
                            page: 0,
                        })
                        .await?;
                }
                Err(err) => {
                    bot.answer_callback_query(q.id).await?;
                    let err_msg = format!("Failed to fetch Bloom wallets: {}", err);
                    let _ = send_cleanup_msg(&bot, chat_id, &err_msg, 5).await;
                }
            }
        } else if let Some(task_name) = data.strip_prefix("task_settings_") {
            render_task_settings_view(
                &bot,
                redis_client.clone(),
                chat_id,
                message.id,
                task_name,
                sol_price_state.clone(),
                rpc_clients.clone(),
            )
            .await?;
            dialogue
                .update(State::TaskSettingsMenu {
                    _task_name: task_name.to_string(),
                    _menu_message_id: message.id,
                })
                .await?;
        } else if let Some(task_name) = data.strip_prefix("task_detail_") {
            if let Some(task) = get_task_by_name(redis_client.clone(), chat_id.0, task_name).await {
                let task_text = build_task_detail_text(
                    redis_client.clone(),
                    chat_id.0,
                    &task,
                    sol_price_state.clone(),
                    rpc_clients.clone(),
                )
                .await;
                bot.edit_message_text(chat_id, message.id, task_text)
                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                    .reply_markup(task_detail_keyboard(&task))
                    .await?;
            } else {
                bot.edit_message_text(chat_id, message.id, "Task not found.")
                    .await?;
            }
        } else if let Some(task_name) = data.strip_prefix("task_toggle_inform_") {
            toggle_task_inform_only(redis_client.clone(), chat_id.0, task_name).await?;
            if let Some(task) = get_task_by_name(redis_client.clone(), chat_id.0, task_name).await {
                let task_text = build_task_detail_text(
                    redis_client.clone(),
                    chat_id.0,
                    &task,
                    sol_price_state.clone(),
                    rpc_clients.clone(),
                )
                .await;
                bot.edit_message_text(chat_id, message.id, task_text)
                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                    .reply_markup(task_detail_keyboard(&task))
                    .await?;
            }
        } else if let Some(task_name) = data.strip_prefix("task_toggle_") {
            if let Some(task) = get_task_by_name(redis_client.clone(), chat_id.0, task_name).await {
                if !task.active {
                    if let Some(error_msg) = activation_requirement_error(&task) {
                        bot.answer_callback_query(q.id.clone()).await?;
                        let _ = send_cleanup_msg(&bot, chat_id, error_msg, 5).await;
                        let task_text = build_task_detail_text(
                            redis_client.clone(),
                            chat_id.0,
                            &task,
                            sol_price_state.clone(),
                            rpc_clients.clone(),
                        )
                        .await;
                        bot.edit_message_text(chat_id, message.id, task_text)
                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                            .reply_markup(task_detail_keyboard(&task))
                            .await?;
                        return Ok(());
                    }

                    if matches!(task.platform, Platform::Telegram) {
                        let Some(channel_id) = task.listen_channels.first().copied() else {
                            bot.answer_callback_query(q.id.clone()).await?;
                            let _ = send_cleanup_msg(
                                &bot,
                                chat_id,
                                "⚠️ Channel information is missing. Please choose the channel again.",
                                6,
                            )
                            .await;
                            return Ok(());
                        };

                        let handle = user_client_handle.lock().clone();
                        let Some(client) = handle else {
                            bot.answer_callback_query(q.id.clone()).await?;
                            let _ = send_cleanup_msg(
                                &bot,
                                chat_id,
                                "⚠️ Telegram user client is disconnected. Please restart the session.",
                                6,
                            )
                            .await;
                            return Ok(());
                        };

                        match is_channel_member(&client, channel_id).await {
                            Ok(true) => {}
                            Ok(false) => {
                                bot.answer_callback_query(q.id.clone()).await?;
                                log::warn!(
                                    "Membership check failed chat_id={} task={} channel={}",
                                    chat_id.0,
                                    task_name,
                                    channel_id
                                );
                                let cleared_task_result =
                                    clear_telegram_task_channel_configuration(
                                        redis_client.clone(),
                                        chat_id.0,
                                        task_name,
                                    )
                                    .await;
                                let task_for_view = match cleared_task_result {
                                    Ok(Some(updated_task)) => updated_task,
                                    Ok(None) => {
                                        log::warn!(
                                            "No task found during channel reset chat_id={} task={}",
                                            chat_id.0,
                                            task_name
                                        );
                                        task.clone()
                                    }
                                    Err(err) => {
                                        log::error!(
                                            "Failed to clear Telegram channel configuration chat_id={} task={} err={}",
                                            chat_id.0,
                                            task_name,
                                            err
                                        );
                                        let msg =
                                            format!("⚠️ Failed to reset channel settings: {}", err);
                                        let _ = send_cleanup_msg(&bot, chat_id, &msg, 8).await;
                                        return Ok(());
                                    }
                                };

                                let task_text = build_task_detail_text(
                                    redis_client.clone(),
                                    chat_id.0,
                                    &task_for_view,
                                    sol_price_state.clone(),
                                    rpc_clients.clone(),
                                )
                                .await;
                                bot.edit_message_text(chat_id, message.id, task_text)
                                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                                    .reply_markup(task_detail_keyboard(&task_for_view))
                                    .await?;

                                let warning_msg = "⚠️ The Telegram user client is no longer a member of this channel. Channel and monitoring settings were reset. Please select another channel or rejoin.";
                                let _ = send_cleanup_msg(&bot, chat_id, warning_msg, 8).await;
                                return Ok(());
                            }
                            Err(err) => {
                                bot.answer_callback_query(q.id.clone()).await?;
                                log::warn!(
                                    "Failed to verify membership chat_id={} task={} err={}",
                                    chat_id.0,
                                    task_name,
                                    err
                                );
                                let msg =
                                    format!("⚠️ Unable to verify channel membership: {}", err);
                                let _ = send_cleanup_msg(&bot, chat_id, &msg, 8).await;
                                return Ok(());
                            }
                        }
                    }
                }

                toggle_task_active(redis_client.clone(), chat_id.0, task_name).await?;
                if let Some(updated_task) =
                    get_task_by_name(redis_client.clone(), chat_id.0, task_name).await
                {
                    let task_text = build_task_detail_text(
                        redis_client.clone(),
                        chat_id.0,
                        &updated_task,
                        sol_price_state.clone(),
                        rpc_clients.clone(),
                    )
                    .await;
                    bot.edit_message_text(chat_id, message.id, task_text)
                        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                        .reply_markup(task_detail_keyboard(&updated_task))
                        .await?;
                }
            }
        } else if let Some(task_name) = data.strip_prefix("task_delete_confirm_") {
            if let Some(task) = get_task_by_name(redis_client.clone(), chat_id.0, task_name).await {
                if task.active {
                    let _ = send_cleanup_msg(
                        &bot,
                        chat_id,
                        "⚠️ Task is active. Please deactivate it before deleting.",
                        5,
                    )
                    .await;
                } else {
                    delete_task(redis_client.clone(), chat_id.0, task_name).await?;
                    bot.answer_callback_query(q.id)
                        .text("🗑️ Task deleted.")
                        .await?;
                    let tasks_text = generate_tasks_text(redis_client.clone(), chat_id.0).await;
                    let tasks = get_tasks(redis_client.clone(), chat_id.0).await;
                    bot.edit_message_text(chat_id, message.id, tasks_text)
                        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                        .reply_markup(tasks_menu_keyboard(&tasks))
                        .await?;
                    dialogue.update(State::TasksMenu).await?;
                }
            }
        } else if let Some(task_name) = data.strip_prefix("task_delete_") {
            if let Some(task) = get_task_by_name(redis_client.clone(), chat_id.0, task_name).await {
                if task.active {
                    let _ = send_cleanup_msg(
                        &bot,
                        chat_id,
                        "⚠️ This task is currently running. Stop it first, then delete.",
                        5,
                    )
                    .await;
                } else {
                    let confirmation_text = "⚠️ *Are you sure?*\n\nThis action cannot be undone\\.\nDo you want to permanently delete this task?";
                    bot.edit_message_text(chat_id, message.id, confirmation_text)
                        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                        .reply_markup(task_delete_confirmation_keyboard(task_name))
                        .await?;
                }
            }
        } else if let Some(task_name) = data.strip_prefix("task_name_") {
            let prompt = bot
                .send_message(chat_id, "Please enter the new name for this task:")
                .await?;
            dialogue
                .update(State::TaskReceiveName {
                    task_name: task_name.to_string(),
                    menu_message_id: message.id,
                    prompt_message_id: prompt.id,
                })
                .await?;
        } else if let Some(task_name) = data.strip_prefix("task_buy_amount_") {
            let prompt = bot
                .send_message(
                    chat_id,
                    "Please enter the new buy amount in SOL (e.g., 0.01):",
                )
                .await?;
            dialogue
                .update(State::TaskReceiveBuyAmount {
                    task_name: task_name.to_string(),
                    menu_message_id: message.id,
                    prompt_message_id: prompt.id,
                })
                .await?;
        } else if let Some(task_name) = data.strip_prefix("task_buy_fee_") {
            let prompt = bot
                .send_message(
                    chat_id,
                    "Please enter the new buy priority fee in SOL (e.g., 0.001):",
                )
                .await?;
            dialogue
                .update(State::TaskReceiveBuyFee {
                    task_name: task_name.to_string(),
                    menu_message_id: message.id,
                    prompt_message_id: prompt.id,
                })
                .await?;
        } else if let Some(task_name) = data.strip_prefix("task_slippage_") {
            let prompt = bot
                .send_message(
                    chat_id,
                    "Please enter the new buy slippage percentage (e.g., 25):",
                )
                .await?;
            dialogue
                .update(State::TaskReceiveBuySlippage {
                    task_name: task_name.to_string(),
                    menu_message_id: message.id,
                    prompt_message_id: prompt.id,
                })
                .await?;
        } else if let Some(task_name) = data.strip_prefix("task_blacklist_") {
            let prompt = bot
                .send_message(
                    chat_id,
                    "Enter blacklist words, separated by commas (e.g., rug,scam,honeypot):",
                )
                .await?;
            dialogue
                .update(State::TaskReceiveBlacklist {
                    task_name: task_name.to_string(),
                    menu_message_id: message.id,
                    prompt_message_id: prompt.id,
                })
                .await?;
        } else if let Some(task_name) = data.strip_prefix("task_platform_telegram_") {
            let mut con = redis_client.get_multiplexed_async_connection().await?;
            if let Some(mut user_data) = get_user_data(&mut con, chat_id.0).await? {
                if let Some(task) = user_data.tasks.iter_mut().find(|t| t.name == *task_name) {
                    if task.active {
                        bot.answer_callback_query(q.id)
                            .text("⚠️ Task is active. Please deactivate before changing platform.")
                            .await?;
                        return Ok(());
                    }
                    task.platform = crate::interfaces::bot::data::types::Platform::Telegram;
                    save_user_data(&mut con, chat_id.0, &user_data).await?;
                    if let Some(updated_task) =
                        user_data.tasks.iter().find(|t| t.name == *task_name)
                    {
                        let task_text = build_task_detail_text(
                            redis_client.clone(),
                            chat_id.0,
                            updated_task,
                            sol_price_state.clone(),
                            rpc_clients.clone(),
                        )
                        .await;
                        bot.edit_message_text(chat_id, message.id, task_text)
                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                            .reply_markup(task_detail_keyboard(updated_task))
                            .await?;
                    }
                    bot.answer_callback_query(q.id)
                        .text("✅ Platform changed to Telegram")
                        .await?;
                }
            }
        } else if let Some(task_name) = data.strip_prefix("task_platform_discord_") {
            let mut con = redis_client.get_multiplexed_async_connection().await?;
            if let Some(mut user_data) = get_user_data(&mut con, chat_id.0).await? {
                if let Some(task) = user_data.tasks.iter_mut().find(|t| t.name == *task_name) {
                    if task.active {
                        bot.answer_callback_query(q.id)
                            .text("⚠️ Task is active. Please deactivate before changing platform.")
                            .await?;
                        return Ok(());
                    }
                    task.platform = crate::interfaces::bot::data::types::Platform::Discord;
                    save_user_data(&mut con, chat_id.0, &user_data).await?;
                    if let Some(updated_task) =
                        user_data.tasks.iter().find(|t| t.name == *task_name)
                    {
                        let task_text = build_task_detail_text(
                            redis_client.clone(),
                            chat_id.0,
                            updated_task,
                            sol_price_state.clone(),
                            rpc_clients.clone(),
                        )
                        .await;
                        bot.edit_message_text(chat_id, message.id, task_text)
                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                            .reply_markup(task_detail_keyboard(updated_task))
                            .await?;
                    }
                    bot.answer_callback_query(q.id)
                        .text("✅ Platform changed to Discord")
                        .await?;
                }
            }
        } else if let Some(task_name) = data.strip_prefix("task_telegram_user_") {
            let task_name = task_name.to_string();
            bot.answer_callback_query(q.id.clone()).await?;
            let intro_text = telegram_linking_intro_text();
            bot.edit_message_text(chat_id, message.id, intro_text)
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                .reply_markup(task_telegram_linking_keyboard(&task_name))
                .await?;
            dialogue
                .update(State::TaskTelegramLinking {
                    task_name,
                    menu_message_id: message.id,
                    qr_active: false,
                })
                .await?;
        } else if let Some(task_name) = data.strip_prefix("task_telegram_link_cancel_") {
            let task_name = task_name.to_string();
            bot.answer_callback_query(q.id.clone()).await?;

            if let Some(qr_message_id) = take_active_qr_message(chat_id.0, &task_name) {
                if let Err(err) = bot.delete_message(chat_id, qr_message_id).await {
                    log::warn!(
                        "Failed to delete active QR message during cancel chat_id={} task={} err={}",
                        chat_id.0,
                        task_name,
                        err
                    );
                }
            }

            if let Some(pending) = take_pending_session(chat_id.0, &task_name) {
                drop(pending);
            }

            render_task_settings_view(
                &bot,
                redis_client.clone(),
                chat_id,
                message.id,
                &task_name,
                sol_price_state.clone(),
                rpc_clients.clone(),
            )
            .await?;
            dialogue
                .update(State::TaskSettingsMenu {
                    _task_name: task_name,
                    _menu_message_id: message.id,
                })
                .await?;
        } else if let Some(task_name) = data.strip_prefix("task_telegram_link_generate_") {
            let task_name = task_name.to_string();
            if let Some(state) = dialogue.get().await?.clone() {
                if let State::TaskTelegramLinking {
                    task_name: active_task,
                    menu_message_id,
                    qr_active,
                } = state
                {
                    if active_task == task_name {
                        if qr_active {
                            bot.answer_callback_query(q.id.clone())
                                .text(
                                    "⚠️ A QR code is already active. Please scan it or wait for it to expire.",
                                )
                                .await?;
                            let _ = send_cleanup_msg(
                                &bot,
                                chat_id,
                                "⚠️ An active QR code is awaiting scan. Please scan the current code or wait for it to expire.",
                                8,
                            )
                            .await;
                            return Ok(());
                        }

                        dialogue
                            .update(State::TaskTelegramLinking {
                                task_name: active_task.clone(),
                                menu_message_id,
                                qr_active: true,
                            })
                            .await?;

                        bot.answer_callback_query(q.id.clone())
                            .text("Generating QR code...")
                            .await?;

                        let bot_clone = bot.clone();
                        let redis_clone = redis_client.clone();
                        let dialogue_clone = dialogue.clone();
                        let chat_id_value = chat_id.0;
                        let task_name_for_state = task_name.clone();
                        let menu_message_id_value = menu_message_id;
                        tokio::spawn(async move {
                            if let Err(err) = authenticate_task_user_via_qr(
                                bot_clone.clone(),
                                redis_clone,
                                chat_id_value,
                                task_name.clone(),
                                menu_message_id_value,
                                dialogue_clone.clone(),
                            )
                            .await
                            {
                                log::error!(
                                    "Failed to complete QR login chat_id={} task={} err={}",
                                    chat_id_value,
                                    task_name,
                                    err
                                );
                                take_active_qr_message(chat_id_value, &task_name_for_state);
                                if let Err(update_err) = dialogue_clone
                                    .update(State::TaskTelegramLinking {
                                        task_name: task_name_for_state,
                                        menu_message_id: menu_message_id_value,
                                        qr_active: false,
                                    })
                                    .await
                                {
                                    log::error!(
                                        "Failed to reset Telegram linking state chat_id={} err={}",
                                        chat_id_value,
                                        update_err
                                    );
                                }
                                let error_text =
                                    format!("⚠️ Telegram QR session failed to start: {}", err);
                                let _ = send_cleanup_msg(
                                    &bot_clone,
                                    ChatId(chat_id_value),
                                    &error_text,
                                    8,
                                )
                                .await;
                            }
                        });
                    } else {
                        bot.answer_callback_query(q.id.clone()).await?;
                    }
                } else {
                    bot.answer_callback_query(q.id.clone()).await?;
                }
            } else {
                bot.answer_callback_query(q.id.clone()).await?;
            }
        } else if let Some(task_name) = data.strip_prefix("task_telegram_link_confirm_yes_") {
            let task_name = task_name.to_string();
            bot.answer_callback_query(q.id.clone()).await?;

            if let Some(state) = dialogue.get().await?.clone() {
                if let State::TaskTelegramLinkConfirm {
                    task_name: active_task,
                    menu_message_id,
                } = state
                {
                    if active_task == task_name {
                        if let Some(pending) = take_pending_session(chat_id.0, &task_name) {
                            take_active_qr_message(chat_id.0, &task_name);

                            let mut con = redis_client.get_multiplexed_async_connection().await?;
                            if let Some(mut user_data) = get_user_data(&mut con, chat_id.0).await? {
                                if let Some(task) =
                                    user_data.tasks.iter_mut().find(|t| t.name == task_name)
                                {
                                    task.grammers_session_data =
                                        Some(pending.encoded_session.clone());
                                    task.telegram_username = pending.display_name.clone();
                                    save_user_data(&mut con, chat_id.0, &user_data).await?;
                                }
                            }

                            *USER_CLIENT_HANDLE.lock() = Some(pending.client.clone());
                            if let Err(err) = client_sender.send(pending.client.clone()).await {
                                log::warn!(
                                    "Unable to hand over Telegram client to background loop: {}",
                                    err
                                );
                            }

                            render_task_settings_view(
                                &bot,
                                redis_client.clone(),
                                chat_id,
                                menu_message_id,
                                &task_name,
                                sol_price_state.clone(),
                                rpc_clients.clone(),
                            )
                            .await?;
                            dialogue
                                .update(State::TaskSettingsMenu {
                                    _task_name: task_name.clone(),
                                    _menu_message_id: menu_message_id,
                                })
                                .await?;

                            let success_text = match pending.display_name {
                                Some(name) => format!("✅ Telegram session linked: {}", name),
                                None => "✅ Telegram session successfully linked.".to_string(),
                            };
                            let _ = send_cleanup_msg(&bot, chat_id, &success_text, 8).await;
                        } else {
                            take_active_qr_message(chat_id.0, &task_name);
                            let intro_text = telegram_linking_intro_text();
                            bot.edit_message_text(chat_id, menu_message_id, intro_text)
                                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                                .reply_markup(task_telegram_linking_keyboard(&task_name))
                                .await?;
                            dialogue
                                .update(State::TaskTelegramLinking {
                                    task_name,
                                    menu_message_id,
                                    qr_active: false,
                                })
                                .await?;
                            let _ = send_cleanup_msg(
                                &bot,
                                chat_id,
                                "⚠️ No pending Telegram session found. Please generate a new QR code.",
                                6,
                            )
                            .await;
                        }
                    }
                }
            }
        } else if let Some(task_name) = data.strip_prefix("task_telegram_link_confirm_no_") {
            let task_name = task_name.to_string();
            bot.answer_callback_query(q.id.clone()).await?;

            if let Some(state) = dialogue.get().await?.clone() {
                if let State::TaskTelegramLinkConfirm {
                    task_name: active_task,
                    menu_message_id,
                } = state
                {
                    if active_task == task_name {
                        if let Some(pending) = take_pending_session(chat_id.0, &task_name) {
                            drop(pending);
                        }
                        take_active_qr_message(chat_id.0, &task_name);
                        let intro_text = telegram_linking_intro_text();
                        bot.edit_message_text(chat_id, menu_message_id, intro_text)
                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                            .reply_markup(task_telegram_linking_keyboard(&task_name))
                            .await?;
                        dialogue
                            .update(State::TaskTelegramLinking {
                                task_name,
                                menu_message_id,
                                qr_active: false,
                            })
                            .await?;
                        let _ = send_cleanup_msg(
                            &bot,
                            chat_id,
                            "ℹ️ Please generate a new QR code to try a different account.",
                            6,
                        )
                        .await;
                    }
                }
            }
        } else if let Some(task_name) = data.strip_prefix("task_discord_token_") {
            let prompt = bot
                .send_message(chat_id, "Please enter your Discord user token:")
                .await?;
            dialogue
                .update(State::TaskReceiveDiscordToken {
                    task_name: task_name.to_string(),
                    menu_message_id: message.id,
                    prompt_message_id: prompt.id,
                })
                .await?;
        } else if let Some(task_name) = data.strip_prefix("task_discord_channel_") {
            let task_name = task_name.to_string();
            if let Some(task) = get_task_by_name(redis_client.clone(), chat_id.0, &task_name).await
            {
                if task.active {
                    bot.answer_callback_query(q.id.clone()).await?;
                    let _ = send_cleanup_msg(
                        &bot,
                        chat_id,
                        "⚠️ Task is active. Please stop it before changing the channel ID.",
                        5,
                    )
                    .await;
                    return Ok(());
                }
            }
            let prompt = bot
                .send_message(chat_id, "Please enter the Discord channel ID:")
                .await?;
            dialogue
                .update(State::TaskReceiveDiscordChannelId {
                    task_name,
                    menu_message_id: message.id,
                    prompt_message_id: prompt.id,
                })
                .await?;
        } else if let Some(task_name) = data.strip_prefix("task_discord_users_") {
            let prompt = bot.send_message(chat_id, "Please enter Discord usernames to monitor, separated by commas (e.g., user1,user2,user3):").await?;
            dialogue
                .update(State::TaskReceiveDiscordUsers {
                    task_name: task_name.to_string(),
                    menu_message_id: message.id,
                    prompt_message_id: prompt.id,
                })
                .await?;
        } else if data.starts_with("task_channels_") {
            let task_name = data.strip_prefix("task_channels_").unwrap().to_string();
            if let Some(task) = get_task_by_name(redis_client.clone(), chat_id.0, &task_name).await
            {
                if !task.has_telegram_user_session() {
                    bot.answer_callback_query(q.id.clone()).await?;
                    let _ = send_cleanup_msg(
                        &bot,
                        chat_id,
                        "⚠️ Telegram user client is not logged in. Please link a Telegram user before changing the channel.",
                        5,
                    )
                    .await;
                    return Ok(());
                }
                if task.active {
                    bot.answer_callback_query(q.id.clone()).await?;
                    let _ = send_cleanup_msg(
                        &bot,
                        chat_id,
                        "⚠️ Task is active. Please stop it before changing the channel.",
                        5,
                    )
                    .await;
                    return Ok(());
                }
            }
            let prompt_message = bot
                .send_message(chat_id, "Please enter a channel or group title to search:")
                .await?;
            dialogue
                .update(State::TaskSelectChannelSearch {
                    task_name,
                    menu_message_id: message.id,
                    prompt_message_id: prompt_message.id,
                })
                .await?;
        } else if data.starts_with("task_users_") {
            let task_name = data.strip_prefix("task_users_").unwrap().to_string();
            let mut con = redis_client.get_multiplexed_async_connection().await?;
            if let Some(user_data) = get_user_data(&mut con, chat_id.0).await? {
                if let Some(task) = user_data.tasks.iter().find(|t| t.name == task_name) {
                    if !task.has_telegram_user_session() {
                        bot.answer_callback_query(q.id.clone()).await?;
                        let _ = send_cleanup_msg(
                            &bot,
                            chat_id,
                            "⚠️ Telegram user client is not logged in. Please link a Telegram user before managing monitored users.",
                            5,
                        )
                        .await;
                        return Ok(());
                    }
                    if task.listen_channels.is_empty() {
                        let _ =
                            send_cleanup_msg(&bot, chat_id, "Please set a channel first.", 5).await;
                        return Ok(());
                    }
                    let channel_id = task.listen_channels[0];
                    let handle = user_client_handle.lock().clone();
                    if let Some(client) = handle {
                        match get_chat_admins(&client, channel_id).await {
                            Ok(admins) => {
                                let selected_users = task.listen_users.clone();
                                dialogue
                                    .update(State::TaskSelectUsersFromList {
                                        task_name,
                                        menu_message_id: message.id,
                                        channel_id,
                                        all_users: admins,
                                        selected_users,
                                        page: 0,
                                    })
                                    .await?;
                                let keyboard =
                                    user_selection_keyboard(&dialogue.get().await?.unwrap())
                                        .await
                                        .unwrap();
                                bot.edit_message_text(
                                    chat_id,
                                    message.id,
                                    "Select the admins to listen to (selection saves instantly):",
                                )
                                .reply_markup(keyboard)
                                .await?;
                            }
                            Err(e) => {
                                if is_chat_admin_required_error(&e) {
                                    if let Err(mark_err) = mark_channel_without_users(
                                        redis_client.clone(),
                                        chat_id.0,
                                        &task_name,
                                    )
                                    .await
                                    {
                                        log::warn!(
                                            "Failed to mark channel without admins chat_id={} task={} err={}",
                                            chat_id.0,
                                            task_name,
                                            mark_err
                                        );
                                    }
                                    if let Some(updated_task) = get_task_by_name(
                                        redis_client.clone(),
                                        chat_id.0,
                                        &task_name,
                                    )
                                    .await
                                    {
                                        let task_text = build_task_detail_text(
                                            redis_client.clone(),
                                            chat_id.0,
                                            &updated_task,
                                            sol_price_state.clone(),
                                            rpc_clients.clone(),
                                        )
                                        .await;
                                        bot.edit_message_text(chat_id, message.id, task_text)
                                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                                            .reply_markup(task_detail_keyboard(&updated_task))
                                            .await?;
                                    }
                                    let _ = send_cleanup_msg(
                                        &bot,
                                        chat_id,
                                        "This a channel, no specific users to monitor",
                                        5,
                                    )
                                    .await;
                                } else {
                                    let _ = send_cleanup_msg(
                                        &bot,
                                        chat_id,
                                        &format!("Error fetching admins: {}", e),
                                        5,
                                    )
                                    .await;
                                }
                            }
                        }
                    } else {
                        let _ =
                            send_cleanup_msg(&bot, chat_id, "User client not logged in.", 5).await;
                    }
                }
            }
        } else if data.starts_with("task_chan_select_") {
            let parts: Vec<&str> = data.split('_').collect();
            let task_name = parts[3].to_string();
            let selected_channel_id = parts[4].parse::<i64>()?;
            let mut prompt_message_id_opt: Option<MessageId> = None;
            let mut configuration_message_id_opt: Option<MessageId> = None;
            let mut selected_channel_name = None;
            if let Some(state) = dialogue.get().await?.clone() {
                if let State::TaskSelectChannelFromList {
                    task_name: state_task_name,
                    menu_message_id,
                    prompt_message_id,
                    all_channels,
                    ..
                } = state
                {
                    if state_task_name == task_name {
                        selected_channel_name = all_channels
                            .iter()
                            .find(|(_, id)| *id == selected_channel_id)
                            .map(|(name, _)| name.clone());
                        prompt_message_id_opt = Some(prompt_message_id);
                        configuration_message_id_opt = Some(menu_message_id);
                    }
                }
            }
            if configuration_message_id_opt.is_none() {
                return Ok(());
            }
            let mut con = redis_client.get_multiplexed_async_connection().await?;
            if let Some(mut user_data) = get_user_data(&mut con, chat_id.0).await? {
                if let Some(existing) = user_data.tasks.iter().find(|t| {
                    t.listen_channels.first().copied() == Some(selected_channel_id)
                        && t.name != task_name
                }) {
                    let _ = send_cleanup_msg(
                        &bot,
                        chat_id,
                        &format!(
                            "⚠️ Channel is already assigned to task '{}'.",
                            existing.name
                        ),
                        6,
                    )
                    .await;
                    return Ok(());
                }
                if let Some(task) = user_data.tasks.iter_mut().find(|t| t.name == task_name) {
                    task.listen_channels = vec![selected_channel_id];
                    task.listen_channel_name = selected_channel_name;
                    task.listen_users = vec![];
                    task.listen_usernames = vec![];
                    task.telegram_channel_is_broadcast = false;
                }
                save_user_data(&mut con, chat_id.0, &user_data).await?;
                if let Some(task) = user_data.tasks.iter().find(|t| t.name == task_name) {
                    let task_text = build_task_detail_text(
                        redis_client.clone(),
                        chat_id.0,
                        task,
                        sol_price_state.clone(),
                        rpc_clients.clone(),
                    )
                    .await;
                    if let Some(configuration_message_id) = configuration_message_id_opt {
                        bot.edit_message_text(chat_id, configuration_message_id, task_text)
                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                            .reply_markup(task_detail_keyboard(task))
                            .await?;
                    }
                }
            }
            if let Some(prompt_message_id) = prompt_message_id_opt {
                bot.delete_message(chat_id, prompt_message_id).await.ok();
            }
            dialogue.update(State::TasksMenu).await?;
        } else if data.starts_with("task_user_toggle_") || data.starts_with("task_user_page_") {
            if let Some(State::TaskSelectUsersFromList {
                task_name,
                menu_message_id,
                channel_id,
                all_users,
                mut selected_users,
                mut page,
            }) = dialogue.get().await?.clone()
            {
                if data.starts_with("task_user_toggle_") {
                    let user_id_to_toggle = data.split('_').last().unwrap().parse::<i64>()?;
                    if let Some(pos) = selected_users
                        .iter()
                        .position(|&id| id == user_id_to_toggle)
                    {
                        selected_users.remove(pos);
                    } else {
                        selected_users.push(user_id_to_toggle);
                    }
                    let mut con = redis_client.get_multiplexed_async_connection().await?;
                    if let Some(mut user_data) = get_user_data(&mut con, chat_id.0).await? {
                        if let Some(task) = user_data.tasks.iter_mut().find(|t| t.name == task_name)
                        {
                            task.listen_users = selected_users.clone();
                            let id_to_display: std::collections::HashMap<i64, String> =
                                all_users.iter().map(|(n, i, _)| (*i, n.clone())).collect();
                            let mut selected_names: Vec<String> = selected_users
                                .iter()
                                .filter_map(|id| id_to_display.get(id).cloned())
                                .collect();
                            selected_names.sort();
                            task.listen_usernames = selected_names;
                            task.telegram_channel_is_broadcast = false;
                        }
                        save_user_data(&mut con, chat_id.0, &user_data).await?;
                    }
                    bot.answer_callback_query(q.id)
                        .text("✅ Selection saved.")
                        .await?;
                } else if data.starts_with("task_user_page_") {
                    let new_page = data.split('_').last().unwrap().parse::<usize>()?;
                    page = new_page;
                }
                let new_state = State::TaskSelectUsersFromList {
                    task_name,
                    menu_message_id,
                    channel_id,
                    all_users,
                    selected_users,
                    page,
                };
                dialogue.update(new_state.clone()).await?;
                let keyboard = user_selection_keyboard(&new_state).await.unwrap();
                bot.edit_message_reply_markup(chat_id, message.id)
                    .reply_markup(keyboard)
                    .await?;
            }
        } else if data.starts_with("task_chan_page_") {
            if let Some(State::TaskSelectChannelFromList {
                task_name,
                menu_message_id,
                prompt_message_id,
                all_channels,
                mut page,
            }) = dialogue.get().await?.clone()
            {
                let new_page = data.split('_').last().unwrap().parse::<usize>()?;
                page = new_page;
                let new_state = State::TaskSelectChannelFromList {
                    task_name,
                    menu_message_id,
                    prompt_message_id,
                    all_channels,
                    page,
                };
                dialogue.update(new_state.clone()).await?;
                let keyboard = channel_selection_keyboard(&new_state).await.unwrap();
                bot.edit_message_reply_markup(chat_id, prompt_message_id)
                    .reply_markup(keyboard)
                    .await?;
            }
        } else if let Some(task_name) = data.strip_prefix("task_chan_cancel_") {
            if let Some(state) = dialogue.get().await?.clone() {
                if let State::TaskSelectChannelFromList {
                    task_name: state_task_name,
                    prompt_message_id: _,
                    ..
                } = state
                {
                    if state_task_name == task_name {
                        bot.delete_message(chat_id, message.id).await.ok();
                        dialogue.update(State::TasksMenu).await?;
                        bot.answer_callback_query(q.id)
                            .text("Channel selection cancelled.")
                            .await?;
                        return Ok(());
                    }
                }
            }
        }
    }
    Ok(())
}

pub async fn get_tasks(redis_client: RedisClient, chat_id: i64) -> Vec<Task> {
    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    if let Ok(Some(user_data)) = get_user_data(&mut con, chat_id).await {
        user_data.tasks
    } else {
        vec![]
    }
}

async fn get_task_by_name(redis_client: RedisClient, chat_id: i64, name: &str) -> Option<Task> {
    let tasks = get_tasks(redis_client, chat_id).await;
    tasks.into_iter().find(|t| t.name == *name)
}

async fn build_task_detail_text(
    redis_client: RedisClient,
    chat_id: i64,
    task: &Task,
    sol_price_state: SolPriceState,
    rpc_clients: RpcClients,
) -> String {
    let selected_wallet = load_selected_wallet(task, &rpc_clients).await;
    generate_task_detail_text(
        redis_client,
        chat_id,
        task,
        sol_price_state,
        selected_wallet.as_ref(),
    )
    .await
}

async fn build_task_settings_text(
    redis_client: RedisClient,
    chat_id: i64,
    task: &Task,
    sol_price_state: SolPriceState,
    rpc_clients: RpcClients,
) -> String {
    let selected_wallet = load_selected_wallet(task, &rpc_clients).await;
    generate_task_settings_text(
        redis_client,
        chat_id,
        task,
        sol_price_state,
        selected_wallet.as_ref(),
    )
    .await
}

async fn load_selected_wallet(task: &Task, rpc_clients: &RpcClients) -> Option<WalletDisplayInfo> {
    if let Some(wallet) = task.bloom_wallet.as_ref() {
        Some(build_wallet_display(wallet, rpc_clients).await)
    } else {
        None
    }
}

async fn build_wallet_display(
    wallet: &BloomWalletInfo,
    rpc_clients: &RpcClients,
) -> WalletDisplayInfo {
    let balance_sol = fetch_wallet_balance(&wallet.address, rpc_clients).await;
    WalletDisplayInfo {
        label: wallet.label.clone(),
        address: wallet.address.clone(),
        balance_sol,
    }
}

async fn collect_wallet_page(
    wallets: &[BloomWalletInfo],
    rpc_clients: &RpcClients,
) -> Vec<WalletDisplayInfo> {
    let mut result = Vec::with_capacity(wallets.len());
    for wallet in wallets {
        result.push(build_wallet_display(wallet, rpc_clients).await);
    }
    result
}

async fn fetch_wallet_balance(address: &str, rpc_clients: &RpcClients) -> Option<f64> {
    match Pubkey::from_str(address) {
        Ok(pubkey) => match rpc_clients.helius_client.get_balance(&pubkey).await {
            Ok(lamports) => Some(lamports as f64 / 1_000_000_000.0),
            Err(err) => {
                log::warn!("Failed to fetch SOL balance for {}: {}", address, err);
                None
            }
        },
        Err(err) => {
            log::warn!("Invalid wallet address {}: {}", address, err);
            None
        }
    }
}

async fn render_task_settings_view(
    bot: &Bot,
    redis_client: RedisClient,
    chat_id: ChatId,
    message_id: MessageId,
    task_name: &str,
    sol_price_state: SolPriceState,
    rpc_clients: RpcClients,
) -> HandlerResult {
    if let Some(task) = get_task_by_name(redis_client.clone(), chat_id.0, task_name).await {
        let text = build_task_settings_text(
            redis_client.clone(),
            chat_id.0,
            &task,
            sol_price_state.clone(),
            rpc_clients.clone(),
        )
        .await;
        bot.edit_message_text(chat_id, message_id, text)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .reply_markup(task_settings_keyboard(&task))
            .await?;
    }
    Ok(())
}

async fn render_task_wallets_view(
    bot: &Bot,
    redis_client: RedisClient,
    chat_id: ChatId,
    message_id: MessageId,
    task_name: &str,
    wallets: &[BloomWalletInfo],
    page: usize,
    sol_price_state: SolPriceState,
    rpc_clients: RpcClients,
) -> HandlerResult {
    if let Some(task) = get_task_by_name(redis_client.clone(), chat_id.0, task_name).await {
        let selected_address = task
            .bloom_wallet
            .as_ref()
            .map(|wallet| wallet.address.as_str());

        let start = (page * ITEMS_PER_PAGE).min(wallets.len());
        let end = (start + ITEMS_PER_PAGE).min(wallets.len());
        let page_wallets = collect_wallet_page(&wallets[start..end], &rpc_clients).await;
        let selected_wallet = load_selected_wallet(&task, &rpc_clients).await;

        let price_guard = sol_price_state.read().await;
        let sol_price_value = (*price_guard).map(|value| value as f64);
        drop(price_guard);

        let text = generate_task_wallets_text(
            &task,
            selected_wallet.as_ref(),
            &page_wallets,
            sol_price_value,
        );
        let keyboard = task_wallets_keyboard(task_name, wallets, selected_address, page);
        bot.edit_message_text(chat_id, message_id, text)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .reply_markup(keyboard)
            .await?;
    }
    Ok(())
}

async fn persist_task_wallet_selection(
    redis_client: &RedisClient,
    chat_id: i64,
    task_name: &str,
    wallet: Option<&crate::interfaces::bot::data::BloomWalletInfo>,
) -> Result<Option<Task>, Box<dyn std::error::Error + Send + Sync>> {
    let mut con = redis_client.get_multiplexed_async_connection().await?;
    if let Some(mut user_data) = get_user_data(&mut con, chat_id).await? {
        if let Some(task) = user_data.tasks.iter_mut().find(|t| t.name == task_name) {
            task.bloom_wallet = wallet.cloned();
            let updated_task = task.clone();
            save_user_data(&mut con, chat_id, &user_data).await?;
            return Ok(Some(updated_task));
        }
    }
    Ok(None)
}

async fn clear_telegram_task_channel_configuration(
    redis_client: RedisClient,
    chat_id: i64,
    task_name: &str,
) -> Result<Option<Task>, Box<dyn std::error::Error + Send + Sync>> {
    let mut con = redis_client.get_multiplexed_async_connection().await?;
    if let Some(mut user_data) = get_user_data(&mut con, chat_id).await? {
        if let Some(task) = user_data.tasks.iter_mut().find(|t| t.name == task_name) {
            task.listen_channels.clear();
            task.listen_channel_name = None;
            task.listen_users.clear();
            task.listen_usernames.clear();
            task.telegram_channel_is_broadcast = false;
            task.active = false;
            let updated_task = task.clone();
            save_user_data(&mut con, chat_id, &user_data).await?;
            return Ok(Some(updated_task));
        }
    }
    Ok(None)
}

async fn toggle_task_active(
    redis_client: RedisClient,
    chat_id: i64,
    task_name: &str,
) -> HandlerResult {
    use crate::interfaces::bot::data::types::Platform;
    let mut con = redis_client.get_multiplexed_async_connection().await?;
    if let Some(mut user_data) = get_user_data(&mut con, chat_id).await? {
        if let Some(task) = user_data.tasks.iter_mut().find(|t| t.name == *task_name) {
            task.active = !task.active;
            let platform = task.platform.clone();
            let task_clone = task.clone();
            save_user_data(&mut con, chat_id, &user_data).await?;
            if task_clone.active {
                match platform {
                    Platform::Telegram => {
                        crate::interfaces::bot::tasks::tg::start_task_monitor(task_clone, chat_id)
                            .await;
                    }
                    Platform::Discord => {
                        crate::interfaces::bot::tasks::discord::start_task_monitor(
                            task_clone, chat_id,
                        )
                        .await;
                    }
                }
            }
        }
    }
    Ok(())
}

fn activation_requirement_error(task: &Task) -> Option<&'static str> {
    if task.bloom_wallet.is_none() {
        return Some("❌ Please assign a Bloom wallet before activating this task.");
    }

    match task.platform {
        Platform::Telegram => {
            let has_session = task
                .grammers_session_data
                .as_ref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);
            if !has_session {
                return Some(
                    "❌ Telegram user session is not configured. Link a Telegram user via the QR login flow before activating this task.",
                );
            }
            if task.listen_channels.is_empty() {
                return Some("❌ Please set a Telegram channel ID before activating this task.");
            }
            if task.listen_users.is_empty()
                && task.listen_usernames.is_empty()
                && !task.telegram_channel_is_broadcast
            {
                return Some(
                    "❌ Please choose at least one Telegram user to monitor before activating this task.",
                );
            }
        }
        Platform::Discord => {
            let token_missing = task
                .discord_token
                .as_ref()
                .map(|token| token.trim().is_empty())
                .unwrap_or(true);
            if token_missing {
                return Some("❌ Please set a Discord token before activating this task.");
            }

            let channel_missing = task
                .discord_channel_id
                .as_ref()
                .map(|channel| channel.trim().is_empty())
                .unwrap_or(true);
            if channel_missing {
                return Some("❌ Please set a Discord channel ID before activating this task.");
            }

            if task.discord_users.is_empty() {
                return Some(
                    "❌ Please choose at least one Discord user to monitor before activating this task.",
                );
            }
        }
    }

    None
}

fn is_chat_admin_required_error(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| cause.to_string().contains("CHAT_ADMIN_REQUIRED"))
}

async fn mark_channel_without_users(
    redis_client: RedisClient,
    chat_id: i64,
    task_name: &str,
) -> redis::RedisResult<()> {
    let mut con = redis_client.get_multiplexed_async_connection().await?;
    if let Some(mut user_data) = get_user_data(&mut con, chat_id).await? {
        if let Some(task) = user_data.tasks.iter_mut().find(|t| t.name == task_name) {
            task.telegram_channel_is_broadcast = true;
            save_user_data(&mut con, chat_id, &user_data).await?;
        }
    }
    Ok(())
}

async fn toggle_task_inform_only(
    redis_client: RedisClient,
    chat_id: i64,
    task_name: &str,
) -> HandlerResult {
    use crate::interfaces::bot::data::types::Platform;
    let mut con = redis_client.get_multiplexed_async_connection().await?;
    if let Some(mut user_data) = get_user_data(&mut con, chat_id).await? {
        let was_active;
        let platform;

        if let Some(task) = user_data.tasks.iter_mut().find(|t| t.name == *task_name) {
            was_active = task.active;
            platform = task.platform.clone();

            if was_active {
                task.active = false;
            }
        } else {
            return Ok(());
        }

        if was_active {
            save_user_data(&mut con, chat_id, &user_data).await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        if let Some(task) = user_data.tasks.iter_mut().find(|t| t.name == *task_name) {
            task.inform_only = !task.inform_only;

            if was_active {
                task.active = true;
            }
        }

        save_user_data(&mut con, chat_id, &user_data).await?;

        if was_active {
            if let Some(task) = user_data.tasks.iter().find(|t| t.name == *task_name) {
                let task_clone = task.clone();
                match platform {
                    Platform::Telegram => {
                        crate::interfaces::bot::tasks::tg::start_task_monitor(task_clone, chat_id)
                            .await;
                    }
                    Platform::Discord => {
                        crate::interfaces::bot::tasks::discord::start_task_monitor(
                            task_clone, chat_id,
                        )
                        .await;
                    }
                }
            }
        }
    }
    Ok(())
}

async fn delete_task(redis_client: RedisClient, chat_id: i64, task_name: &str) -> HandlerResult {
    let mut con = redis_client.get_multiplexed_async_connection().await?;
    if let Some(mut user_data) = get_user_data(&mut con, chat_id).await? {
        if let Some(existing) = user_data.tasks.iter().find(|t| t.name == *task_name) {
            if existing.active {
                return Ok(());
            }
        }
        user_data.tasks.retain(|t| t.name != *task_name);
        save_user_data(&mut con, chat_id, &user_data).await?;
    }
    Ok(())
}
