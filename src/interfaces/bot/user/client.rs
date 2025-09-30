use crate::{PENDING_BLOOM_INFO, USER_CLIENT_HANDLE};
use anyhow::{Result, anyhow};
use grammers_client::types::participant::Role;
use grammers_client::types::{Chat, User};
use grammers_client::{Client, Config, InitParams, SignInError};
use grammers_session::Session;
use redis::Commands;
use std::env;
use std::io::{self, Write};
use std::time::Duration;
use tokio::sync::oneshot;

pub type UserClientHandle = grammers_client::Client;

const SESSION_KEY: &str = "grammers_session_data";

fn get_redis_connection(redis_url: &str) -> Result<redis::Connection> {
    let client = redis::Client::open(redis_url)?;
    Ok(client.get_connection()?)
}

fn load_or_create_session(redis_url: &str) -> Result<Session> {
    let mut conn = get_redis_connection(redis_url)?;
    let data: Option<Vec<u8>> = conn.get(SESSION_KEY)?;

    if let Some(session_data) = data {
        log::info!("Loading Grammers session from Redis...");
        Ok(Session::load(&session_data)?)
    } else {
        log::info!("No Grammers session found in Redis, creating a new one.");
        Ok(Session::new())
    }
}

fn save_session(session: &Session, redis_url: &str) -> Result<()> {
    log::info!("Saving Grammers session to Redis...");
    let mut conn = get_redis_connection(redis_url)?;
    let data = session.save();
    let _: () = conn.set(SESSION_KEY, data)?;
    Ok(())
}

async fn get_stdin_line() -> Result<String> {
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

pub async fn try_auto_login_user_client(redis_url: String) -> Result<(Client, UserClientHandle)> {
    let session = load_or_create_session(&redis_url)?;

    let api_id = env::var("API_ID")
        .expect("API_ID must be set in .env file")
        .parse()?;
    let api_hash = env::var("API_HASH").expect("API_HASH must be set in .env file");

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
    .await?;

    if !client.is_authorized().await? {
        return Err(anyhow!("Session is not authorized. Manual login required."));
    }

    let handle = client.clone();
    *USER_CLIENT_HANDLE.lock() = Some(handle.clone());
    Ok((client, handle))
}

pub async fn create_user_client(redis_url: String) -> Result<(Client, UserClientHandle)> {
    let session = load_or_create_session(&redis_url)?;

    let api_id = env::var("API_ID")
        .expect("API_ID must be set in .env file")
        .parse()?;
    let api_hash = env::var("API_HASH").expect("API_HASH must be set in .env file");

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
    .await?;

    let handle = client.clone();
    *USER_CLIENT_HANDLE.lock() = Some(handle.clone());

    if !client.is_authorized().await? {
        let phone = {
            print!("Enter your phone number (e.g., +1234567890): ");
            io::stdout().flush().unwrap();
            get_stdin_line().await?
        };
        let token = client.request_login_code(&phone).await?;

        let code = {
            print!("Enter the login code you received: ");
            io::stdout().flush().unwrap();
            get_stdin_line().await?
        };

        let signed_in = client.sign_in(&token, &code).await;
        match signed_in {
            Err(SignInError::PasswordRequired(password_token)) => {
                let password = {
                    print!("Enter your 2FA password: ");
                    io::stdout().flush().unwrap();
                    get_stdin_line().await?
                };
                client.check_password(password_token, password).await?;
            }
            Ok(_) => (),
            Err(e) => return Err(e.into()),
        }
        save_session(&client.session(), &redis_url)?;
    }

    Ok((client, handle))
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
    let mut name = user.first_name().unwrap_or("").to_string();
    if let Some(last) = user.last_name() {
        name.push(' ');
        name.push_str(last);
    }
    name
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
