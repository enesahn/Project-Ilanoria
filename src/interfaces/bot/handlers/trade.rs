use parking_lot::Mutex;
use redis::Client as RedisClient;
use std::sync::Arc;
use teloxide::prelude::*;

use super::text::{format_token_info_message, get_parsed_token_info, parse_mint_from_text_robust};
use crate::application::pricing::SolPriceState;
use crate::infrastructure::blockchain::RpcClients;
use crate::interfaces::bot::user::client::UserClientHandle;
use crate::interfaces::bot::{State, send_cleanup_msg, token_info_keyboard};

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
type MyDialogue = Dialogue<State, teloxide::dispatching::dialogue::InMemStorage<State>>;

pub async fn handle_trade_callback(
    q: CallbackQuery,
    bot: Bot,
    redis_client: RedisClient,
    _dialogue: MyDialogue,
    sol_price_state: SolPriceState,
    user_client_handle: Arc<Mutex<Option<UserClientHandle>>>,
    rpc_clients: RpcClients,
) -> HandlerResult {
    if let Some(message) = q.message.clone() {
        let chat_id = message.chat.id;
        let data = q.data.clone().unwrap_or_default();
        let message_text = message.text().unwrap_or_default();

        if data == "r" {
            let maybe_mint = parse_mint_from_text_robust(message_text);
            log::info!("[TOKEN_REFRESH] Parsed Mint Address: {:?}", maybe_mint);

            if let Some(mint) = maybe_mint {
                match get_parsed_token_info(&mint, user_client_handle).await {
                    Ok(token_info) => {
                        let new_text = format_token_info_message(
                            &mint,
                            &token_info,
                            chat_id.0,
                            redis_client.clone(),
                            sol_price_state.clone(),
                            rpc_clients.clone(),
                        )
                        .await;

                        let keyboard = token_info_keyboard(&mint);
                        if let Err(err) = bot
                            .edit_message_text(chat_id, message.id, new_text)
                            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                            .disable_web_page_preview(true)
                            .reply_markup(keyboard)
                            .await
                        {
                            if !matches!(
                                err,
                                teloxide::RequestError::Api(teloxide::ApiError::MessageNotModified)
                            ) {
                                return Err(Box::new(err));
                            }
                        }
                    }
                    Err(e) => {
                        bot.answer_callback_query(q.id.clone()).await?;
                        let error_text = e.to_string();
                        let _ = send_cleanup_msg(&bot, chat_id, &error_text, 5).await;
                    }
                }
            } else {
                bot.answer_callback_query(q.id.clone()).await?;
                let _ = send_cleanup_msg(
                    &bot,
                    chat_id,
                    "Could not find mint address in the message.",
                    5,
                )
                .await;
            }
        }
    }

    Ok(())
}
