use crate::PENDING_BLOOM_INFO;
use crate::interfaces::bot::State;
use crate::interfaces::bot::data::get_user_data;
use crate::interfaces::bot::ui::menu::escape_markdown;
use crate::interfaces::bot::{
    task_telegram_confirm_keyboard, task_telegram_linking_keyboard, telegram_linking_expired_text,
    telegram_linking_scan_text,
};
use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::{
    STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD,
};
use grammers_client::types::participant::Role;
use grammers_client::types::{Chat, User};
use grammers_client::{Client, Config, InitParams, InvocationError};
use grammers_session::Session;
use grammers_tl_types as tl;
use image::{DynamicImage, ImageOutputFormat, Luma};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use qrcode::QrCode;
use redis::Client as RedisClient;
use std::collections::HashMap;
use std::env;
use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
use teloxide::prelude::*;
use teloxide::types::{ChatId, InputFile, MessageId, ParseMode};
use tokio::sync::oneshot;
use tokio::time::timeout;

pub type UserClientHandle = grammers_client::Client;

pub static ACTIVE_QR_MESSAGES: Lazy<Arc<Mutex<HashMap<(i64, String), MessageId>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

pub fn set_active_qr_message(chat_id: i64, task_name: &str, message_id: MessageId) {
    ACTIVE_QR_MESSAGES
        .lock()
        .insert((chat_id, task_name.to_string()), message_id);
}

pub fn take_active_qr_message(chat_id: i64, task_name: &str) -> Option<MessageId> {
    ACTIVE_QR_MESSAGES
        .lock()
        .remove(&(chat_id, task_name.to_string()))
}

pub struct PendingTelegramSession {
    pub client: Client,
    pub encoded_session: String,
    pub display_name: Option<String>,
}

pub static PENDING_QR_SESSIONS: Lazy<Arc<Mutex<HashMap<(i64, String), PendingTelegramSession>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

pub fn store_pending_session(
    chat_id: i64,
    task_name: &str,
    session: PendingTelegramSession,
) -> Option<PendingTelegramSession> {
    PENDING_QR_SESSIONS
        .lock()
        .insert((chat_id, task_name.to_string()), session)
}

pub fn take_pending_session(chat_id: i64, task_name: &str) -> Option<PendingTelegramSession> {
    PENDING_QR_SESSIONS
        .lock()
        .remove(&(chat_id, task_name.to_string()))
}

pub fn encode_session(session: &Session) -> String {
    let bytes = session.save();
    BASE64_STANDARD.encode(bytes)
}

fn build_qr_png(data: &str) -> Result<Vec<u8>> {
    let code =
        QrCode::new(data.as_bytes()).map_err(|err| anyhow!("Failed to create QR code: {}", err))?;
    let image = code
        .render::<Luma<u8>>()
        .min_dimensions(160, 160)
        .max_dimensions(160, 160)
        .build();
    let mut buffer = Cursor::new(Vec::new());
    DynamicImage::ImageLuma8(image)
        .write_to(&mut buffer, ImageOutputFormat::Png)
        .map_err(|err| anyhow!("Failed to encode QR PNG: {}", err))?;
    Ok(buffer.into_inner())
}

fn format_telegram_display_name(user: &grammers_client::types::User) -> Option<String> {
    if let Some(username) = user.username() {
        if username.trim().is_empty() {
            None
        } else {
            Some(format!("@{}", username.trim()))
        }
    } else {
        let first = user.first_name().unwrap_or("").trim().to_string();
        let last = user.last_name().unwrap_or("").trim().to_string();
        let combined = match (first.is_empty(), last.is_empty()) {
            (true, true) => String::new(),
            (false, true) => first,
            (true, false) => last,
            (false, false) => format!("{} {}", first, last),
        };
        if combined.trim().is_empty() {
            None
        } else {
            Some(combined)
        }
    }
}

enum LoginSignalOutcome {
    TokenUpdated,
    TimedOut,
}

async fn wait_for_login_signal(
    client: &Client,
    max_wait: Duration,
) -> Result<LoginSignalOutcome, InvocationError> {
    let mut remaining = max_wait;
    while remaining > Duration::ZERO {
        let start = Instant::now();
        match timeout(remaining, client.next_raw_update()).await {
            Ok(Ok((update, _, _))) => {
                if matches!(update, tl::enums::Update::LoginToken) {
                    return Ok(LoginSignalOutcome::TokenUpdated);
                }
                let elapsed = start.elapsed();
                remaining = remaining.saturating_sub(elapsed);
            }
            Ok(Err(err)) => return Err(err),
            Err(_) => return Ok(LoginSignalOutcome::TimedOut),
        }
    }
    Ok(LoginSignalOutcome::TimedOut)
}

pub async fn authenticate_task_user_via_qr(
    bot: Bot,
    redis_client: RedisClient,
    chat_id: i64,
    task_name: String,
    menu_message_id: MessageId,
    dialogue: Dialogue<State, InMemStorage<State>>,
) -> Result<()> {
    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .context("Failed to connect to Redis")?;
    let user_data = get_user_data(&mut con, chat_id)
        .await
        .context("Failed to load user data")?
        .ok_or_else(|| anyhow!("No user data found for chat {}", chat_id))?;
    drop(con);

    if !user_data.tasks.iter().any(|task| task.name == task_name) {
        return Err(anyhow!(
            "Task '{}' not found while preparing QR authentication",
            task_name
        ));
    }

    let session = Session::new();
    let api_id = env::var("API_ID")
        .context("API_ID environment variable missing")?
        .parse()
        .context("API_ID must be a valid integer")?;
    let api_hash = env::var("API_HASH").context("API_HASH environment variable missing")?;

    let params = InitParams {
        device_model: "PC".to_string(),
        system_version: "Windows 11".to_string(),
        app_version: "Telegram Desktop 4.16.8".to_string(),
        lang_code: "en".to_string(),
        system_lang_code: "en".to_string(),
        ..Default::default()
    };

    let client = Client::connect(Config {
        session,
        api_id,
        api_hash: api_hash.clone(),
        params,
    })
    .await
    .context("Failed to connect Telegram client for QR login")?;

    let chat = ChatId(chat_id);
    let mut qr_message_id: Option<MessageId> = None;
    let mut last_token: Option<Vec<u8>> = None;

    take_active_qr_message(chat_id, &task_name);

    loop {
        let mut response = client
            .export_login_token_raw(&[])
            .await
            .context("Failed to export Telegram login token")?;

        loop {
            match response {
                tl::enums::auth::LoginToken::Token(ref token_payload) => {
                    let display_timeout = token_payload.expires.clamp(5, 60);
                    let should_refresh = last_token
                        .as_ref()
                        .map(|previous| previous != &token_payload.token)
                        .unwrap_or(true);

                    if should_refresh {
                        let tg_url = format!(
                            "tg://login?token={}",
                            BASE64_URL_SAFE_NO_PAD.encode(&token_payload.token)
                        );
                        let png_bytes = build_qr_png(&tg_url)?;

                        if let Some(existing) = qr_message_id.take() {
                            let _ = bot.delete_message(chat, existing).await;
                            take_active_qr_message(chat_id, &task_name);
                        }

                        let sent = bot
                            .send_photo(
                                chat,
                                InputFile::memory(png_bytes).file_name("telegram-login.png"),
                            )
                            .await?;
                        set_active_qr_message(chat_id, &task_name, sent.id);
                        qr_message_id = Some(sent.id);
                        last_token = Some(token_payload.token.clone());
                        let status_text = telegram_linking_scan_text();
                        bot.edit_message_text(chat, menu_message_id, status_text)
                            .parse_mode(ParseMode::MarkdownV2)
                            .reply_markup(task_telegram_linking_keyboard(&task_name))
                            .await?;
                    }

                    let wait_outcome =
                        wait_for_login_signal(&client, Duration::from_secs(display_timeout as u64))
                            .await
                            .map_err(|err| {
                                anyhow!(
                                    "Failed while waiting for Telegram login confirmation: {}",
                                    err
                                )
                            })?;

                    if matches!(wait_outcome, LoginSignalOutcome::TimedOut) {
                        if let Some(existing) = qr_message_id.take() {
                            let _ = bot.delete_message(chat, existing).await;
                            take_active_qr_message(chat_id, &task_name);
                        }
                        let expired_text = telegram_linking_expired_text();
                        let edit_result = bot
                            .edit_message_text(chat, menu_message_id, expired_text)
                            .parse_mode(ParseMode::MarkdownV2)
                            .reply_markup(task_telegram_linking_keyboard(&task_name))
                            .await;
                        if let Err(err) = edit_result {
                            if !matches!(
                                err,
                                teloxide::RequestError::Api(teloxide::ApiError::MessageNotModified)
                            ) {
                                return Err(err.into());
                            }
                        }
                        take_active_qr_message(chat_id, &task_name);
                        return Ok(());
                    }

                    break;
                }
                tl::enums::auth::LoginToken::MigrateTo(migrate) => {
                    client
                        .switch_to_dc(migrate.dc_id)
                        .await
                        .context("Failed to switch Telegram datacenter for QR login")?;
                    response = client
                        .import_login_token_raw(&migrate.token)
                        .await
                        .context("Failed to import Telegram login token in target datacenter")?;
                }
                tl::enums::auth::LoginToken::Success(success) => {
                    if let Some(existing) = qr_message_id.take() {
                        let _ = bot.delete_message(chat, existing).await;
                    }

                    let auth_payload = match success.authorization {
                        tl::enums::auth::Authorization::Authorization(inner) => inner,
                        other => {
                            return Err(anyhow!(
                                "Unexpected authorization variant during QR login: {:?}",
                                other
                            ));
                        }
                    };

                    let user = client
                        .apply_authorization(auth_payload)
                        .await
                        .context("Failed to apply Telegram authorization")?;
                    let encoded_session = encode_session(&client.session());
                    let display_name = format_telegram_display_name(&user);
                    let username = user.username().map(|value| format!("@{}", value));
                    let user_id = user.id();

                    if let Some(previous) = store_pending_session(
                        chat_id,
                        &task_name,
                        PendingTelegramSession {
                            client: client.clone(),
                            encoded_session,
                            display_name: display_name.clone(),
                        },
                    ) {
                        drop(previous);
                    }

                    take_active_qr_message(chat_id, &task_name);

                    let mut lines = Vec::new();
                    lines.push(r"✅ *QR code scanned\!*".to_string());
                    lines.push(String::new());
                    lines.push(escape_markdown(
                        "Please confirm the Telegram account before linking.",
                    ));

                    let mut account_details = Vec::new();
                    if let Some(name) = display_name.as_ref() {
                        account_details.push(format!("• Name: *{}*", escape_markdown(name)));
                    }
                    if let Some(user_name) = username.as_ref() {
                        account_details
                            .push(format!("• Username: *{}*", escape_markdown(user_name)));
                    }
                    let escaped_id = escape_markdown(&user_id.to_string());
                    account_details.push(format!("• User ID: `{}`", escaped_id));

                    lines.extend(account_details);
                    lines.push(String::new());
                    lines.push(escape_markdown(
                        "Select an option below to continue with the linking process.",
                    ));

                    let confirm_text = lines.join("\n");

                    let edit_result = bot
                        .edit_message_text(chat, menu_message_id, confirm_text)
                        .parse_mode(ParseMode::MarkdownV2)
                        .reply_markup(task_telegram_confirm_keyboard(&task_name))
                        .await;

                    if let Err(err) = edit_result {
                        if !matches!(
                            err,
                            teloxide::RequestError::Api(teloxide::ApiError::MessageNotModified)
                        ) {
                            return Err(err.into());
                        }
                    }

                    let _ = dialogue
                        .update(State::TaskTelegramLinkConfirm {
                            task_name,
                            menu_message_id,
                        })
                        .await;

                    return Ok(());
                }
            }
        }
    }
}

pub async fn get_token_info_from_bloom(client: &UserClientHandle, mint: &str) -> Result<String> {
    let bloom_bot = client
        .resolve_username("BloomSolana_bot")
        .await?
        .ok_or_else(|| anyhow!("Could not resolve @BloomSolana_bot"))?;

    client
        .send_message(bloom_bot.clone(), mint.to_string())
        .await?;

    let (tx, rx) = oneshot::channel::<String>();
    {
        let mut map = PENDING_BLOOM_INFO.lock();
        map.insert(mint.to_string(), tx);
    }

    match tokio::time::timeout(Duration::from_secs(10), rx).await {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(_)) => Err(anyhow!("Bloom info channel closed")),
        Err(_) => {
            PENDING_BLOOM_INFO.lock().remove(mint);
            Err(anyhow!("Timed out waiting for a response from Bloom bot"))
        }
    }
}

fn format_user_name(user: &User) -> String {
    if let Some(username) = user.username() {
        format!("@{}", username)
    } else {
        let mut name = user.first_name().unwrap_or("").to_string();
        if let Some(last) = user.last_name() {
            name.push(' ');
            name.push_str(last);
        }
        name
    }
}

pub async fn get_chat_admins(
    client: &UserClientHandle,
    chat_id: i64,
) -> Result<Vec<(String, i64, String)>> {
    let mut target_chat: Option<Chat> = None;

    let mut dialogs = client.iter_dialogs();
    while let Some(dialog) = dialogs.next().await? {
        let chat = dialog.chat();
        if chat.id() == chat_id {
            target_chat = Some(chat.clone());
            break;
        }
    }

    let chat = target_chat.ok_or_else(|| anyhow!("Chat not found"))?;
    let mut participants = client.iter_participants(&chat);
    let mut admins = Vec::new();

    admins.push((
        chat.name().unwrap_or("Unknown").to_string(),
        chat.id(),
        "Channel/Group".to_string(),
    ));

    while let Some(participant) = participants.next().await? {
        let role_str = match &participant.role {
            Role::Creator(_) => Some("Creator".to_string()),
            Role::Admin { .. } => Some("Admin".to_string()),
            _ => None,
        };

        if let Some(role) = role_str {
            let user = participant.user;
            if user.is_bot() {
                continue;
            }
            admins.push((format_user_name(&user), user.id(), role));
        }
    }

    Ok(admins)
}

pub async fn is_channel_member(client: &UserClientHandle, channel_id: i64) -> Result<bool> {
    let mut dialogs = client.iter_dialogs();
    while let Some(dialog) = dialogs.next().await? {
        let chat = dialog.chat();
        if chat.id() == channel_id {
            let me = client.get_me().await?;
            match client.get_permissions(chat.clone(), &me).await {
                Ok(_) => return Ok(true),
                Err(err) if err.is("USER_NOT_PARTICIPANT") || err.is("CHANNEL_PRIVATE") => {
                    return Ok(false);
                }
                Err(err) if err.is("PEER_ID_INVALID") => {
                    return Ok(false);
                }
                Err(err) => return Err(err.into()),
            }
        }
    }
    Ok(false)
}

pub async fn search_dialogs(client: &UserClientHandle, query: &str) -> Result<Vec<(String, i64)>> {
    let mut results = Vec::new();
    let mut dialogs = client.iter_dialogs();
    let query_lower = query.to_lowercase();

    while let Some(dialog) = dialogs.next().await? {
        let chat = dialog.chat();
        if matches!(chat, Chat::Group(_) | Chat::Channel(_)) {
            if let Some(name) = chat.name() {
                if name.to_lowercase().contains(&query_lower) {
                    results.push((name.to_string(), chat.id()));
                }
            }
        }
    }
    Ok(results)
}
