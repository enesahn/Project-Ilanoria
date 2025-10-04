use crate::interfaces::bot::escape_markdown;
use teloxide::prelude::*;
use tokio::time::{Duration, sleep};

pub async fn send_cleanup_msg(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    seconds: u64,
) -> Result<(), teloxide::RequestError> {
    let sanitized = escape_markdown(text);
    let sent = bot
        .send_message(chat_id, sanitized)
        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        .await?;
    let bot_clone = bot.clone();
    let chat_id_clone = chat_id;
    let message_id = sent.id;
    tokio::spawn(async move {
        sleep(Duration::from_secs(seconds)).await;
        let _ = bot_clone.delete_message(chat_id_clone, message_id).await;
    });
    Ok(())
}
