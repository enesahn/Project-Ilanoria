use redis::Client as RedisClient;
use teloxide::prelude::*;
use teloxide::types::MessageId;

use crate::interfaces::bot::{
    State, create_new_wallet, generate_wallets_text, get_user_data, save_user_data,
    wallets_menu_keyboard,
};

type MyDialogue = Dialogue<State, teloxide::dispatching::dialogue::InMemStorage<State>>;
type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub async fn callback_handler(
    q: CallbackQuery,
    bot: Bot,
    redis_client: RedisClient,
    dialogue: MyDialogue,
) -> HandlerResult {
    if let Some(message) = q.message.clone() {
        let chat_id = message.chat.id;
        let data = q.data.clone().unwrap_or_default();

        if data.starts_with("set_default_") {
            handle_set_default(data, &bot, chat_id, message.id, redis_client).await?;
        } else if data.starts_with("remove_wallet_") {
            handle_remove_wallet(data, &bot, chat_id, message.id, redis_client, &q).await?;
        } else {
            match data.as_str() {
                "view_wallets" => {
                    let mut con = redis_client.get_multiplexed_async_connection().await?;
                    if let Some(user_data) = get_user_data(&mut con, chat_id.0).await? {
                        let wallets_text = generate_wallets_text(redis_client, chat_id.0).await;
                        bot.edit_message_text(chat_id, message.id, wallets_text)
                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                            .reply_markup(wallets_menu_keyboard(
                                &user_data.wallets,
                                user_data.default_wallet_index,
                            ))
                            .await?;
                        dialogue.update(State::WalletsMenu).await?;
                    }
                }
                "create_wallet" => {
                    let mut con = redis_client.get_multiplexed_async_connection().await?;
                    if let Some(mut user_data) = get_user_data(&mut con, chat_id.0).await? {
                        let wallet_name = format!("Wallet {}", user_data.wallets.len() + 1);
                        let new_wallet = create_new_wallet(wallet_name);
                        user_data.wallets.push(new_wallet);
                        save_user_data(&mut con, chat_id.0, &user_data).await?;

                        let wallets_text =
                            generate_wallets_text(redis_client.clone(), chat_id.0).await;
                        bot.edit_message_text(chat_id, message.id, wallets_text)
                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                            .reply_markup(wallets_menu_keyboard(
                                &user_data.wallets,
                                user_data.default_wallet_index,
                            ))
                            .await?;
                    }
                }
                "import_wallet" => {
                    let prompt_message = bot
                        .send_message(chat_id, "Please enter the private key (in base58 format).")
                        .await?;
                    dialogue
                        .update(State::ReceiveImportKey {
                            menu_message_id: message.id,
                            prompt_message_id: prompt_message.id,
                        })
                        .await?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

async fn handle_set_default(
    data: String,
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    redis_client: RedisClient,
) -> HandlerResult {
    let index_str = data.strip_prefix("set_default_").unwrap();
    if let Ok(index) = index_str.parse::<usize>() {
        let mut con = redis_client.get_multiplexed_async_connection().await?;
        if let Some(mut user_data) = get_user_data(&mut con, chat_id.0).await? {
            if index < user_data.wallets.len() {
                user_data.default_wallet_index = index;
                save_user_data(&mut con, chat_id.0, &user_data).await?;

                let wallets_text = generate_wallets_text(redis_client, chat_id.0).await;
                bot.edit_message_text(chat_id, message_id, wallets_text)
                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                    .reply_markup(wallets_menu_keyboard(
                        &user_data.wallets,
                        user_data.default_wallet_index,
                    ))
                    .await?;
            }
        }
    }
    Ok(())
}

async fn handle_remove_wallet(
    data: String,
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    redis_client: RedisClient,
    q: &CallbackQuery,
) -> HandlerResult {
    let index_str = data.strip_prefix("remove_wallet_").unwrap();
    if let Ok(index) = index_str.parse::<usize>() {
        let mut con = redis_client.get_multiplexed_async_connection().await?;
        if let Some(mut user_data) = get_user_data(&mut con, chat_id.0).await? {
            if user_data.wallets.len() > 1 && index < user_data.wallets.len() {
                user_data.wallets.remove(index);
                if user_data.default_wallet_index >= index {
                    user_data.default_wallet_index =
                        user_data.default_wallet_index.saturating_sub(1);
                }
                save_user_data(&mut con, chat_id.0, &user_data).await?;

                let wallets_text = generate_wallets_text(redis_client, chat_id.0).await;
                bot.edit_message_text(chat_id, message_id, wallets_text)
                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                    .reply_markup(wallets_menu_keyboard(
                        &user_data.wallets,
                        user_data.default_wallet_index,
                    ))
                    .await?;
            } else {
                bot.answer_callback_query(q.id.clone())
                    .text("Cannot remove the last wallet.")
                    .show_alert(true)
                    .await?;
            }
        }
    }
    Ok(())
}
