use redis::Client as RedisClient;
use teloxide::prelude::*;
use teloxide::types::{MessageId, ParseMode};

use crate::interfaces::bot::{State, generate_settings_text, settings_menu_keyboard};

type MyDialogue = Dialogue<State, teloxide::dispatching::dialogue::InMemStorage<State>>;
type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub async fn handle_settings_callback(
    q: CallbackQuery,
    bot: Bot,
    redis_client: RedisClient,
    dialogue: MyDialogue,
) -> HandlerResult {
    if let Some(message) = q.message.clone() {
        let chat_id = message.chat.id;
        let data = q.data.clone().unwrap_or_default();

        match data.as_str() {
            "view_cfg" => {
                let settings_text = generate_settings_text(redis_client, chat_id.0).await;
                bot.edit_message_text(chat_id, message.id, settings_text)
                    .parse_mode(ParseMode::MarkdownV2)
                    .reply_markup(settings_menu_keyboard())
                    .await?;
                dialogue.update(State::SettingsMenu).await?;
            }
            _ => {
                handle_edit_action(&data, &bot, chat_id, message.id, dialogue).await?;
            }
        }
    }
    Ok(())
}

async fn handle_edit_action(
    data: &str,
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    dialogue: MyDialogue,
) -> HandlerResult {
    let prompt_text = match data {
        "edit_slippage" => "Please enter the new slippage percentage (e.g., 25).",
        _ => return Ok(()),
    };

    let prompt_message = bot.send_message(chat_id, prompt_text).await?;

    let new_state = match data {
        "edit_slippage" => State::ReceiveSlippage {
            menu_message_id: message_id,
            prompt_message_id: prompt_message.id,
        },
        _ => State::Start,
    };

    dialogue.update(new_state).await?;
    Ok(())
}
