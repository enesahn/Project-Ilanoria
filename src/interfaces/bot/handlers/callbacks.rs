use parking_lot::Mutex;
use redis::Client as RedisClient;
use std::sync::Arc;
use teloxide::prelude::*;

use crate::application::pricing::SolPriceState;
use crate::infrastructure::blockchain::RpcClients;
use crate::interfaces::bot::user::client::UserClientHandle;
use crate::interfaces::bot::{State, generate_main_menu_text, main_menu_keyboard};

use super::{settings, tasks, trade, wallets};

type MyDialogue = Dialogue<State, teloxide::dispatching::dialogue::InMemStorage<State>>;
type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub async fn callback_handler(
    q: CallbackQuery,
    bot: Bot,
    redis_client: RedisClient,
    dialogue: MyDialogue,
    sol_price_state: SolPriceState,
    user_client_handle: Arc<Mutex<Option<UserClientHandle>>>,
    rpc_clients: RpcClients,
) -> HandlerResult {
    if let Some(message) = q.message.clone() {
        let chat_id = message.chat.id;
        let data = q.data.clone().unwrap_or_default();

        log::info!("[CALLBACK] Data: '{}' from ChatID: {}", data, chat_id);

        if data == "rm" {
            match bot.delete_message(chat_id, message.id).await {
                Ok(_) => {}
                Err(e) => log::error!("delete failed: {}", e),
            }
            bot.answer_callback_query(q.id).await?;
            return Ok(());
        }

        let wallet_actions = [
            "view_wallets",
            "create_wallet",
            "import_wallet",
            "set_default_",
            "remove_wallet_",
        ];
        let trade_actions = ["r", "b_", "s_"];
        let settings_actions = [
            "view_cfg",
            "edit_slippage",
            "edit_buy_priority_fee",
            "edit_sell_priority_fee",
        ];

        if data.starts_with("task_") || data == "view_tasks" || data == "create_task" {
            tasks::handle_task_callbacks(
                q.clone(),
                bot.clone(),
                redis_client,
                dialogue,
                user_client_handle.clone(),
            )
            .await?;
        } else if wallet_actions
            .iter()
            .any(|&action| data.starts_with(action))
        {
            wallets::callback_handler(q.clone(), bot.clone(), redis_client, dialogue).await?;
        } else if trade_actions.iter().any(|&action| data.starts_with(action)) {
            trade::handle_trade_callback(
                q.clone(),
                bot.clone(),
                redis_client,
                dialogue,
                sol_price_state,
                user_client_handle,
                rpc_clients,
            )
            .await?;
        } else if settings_actions
            .iter()
            .any(|&action| data.starts_with(action))
        {
            settings::handle_settings_callback(q.clone(), bot.clone(), redis_client, dialogue)
                .await?;
        } else if data == "main_menu" {
            let menu_text = generate_main_menu_text(
                redis_client.clone(),
                chat_id.0,
                sol_price_state,
                rpc_clients,
            )
            .await;
            bot.edit_message_text(chat_id, message.id, menu_text)
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                .reply_markup(main_menu_keyboard())
                .await?;
            dialogue.update(State::Start).await?;
        } else if data == "refresh_main" {
            let menu_text = generate_main_menu_text(
                redis_client.clone(),
                chat_id.0,
                sol_price_state,
                rpc_clients,
            )
            .await;
            match bot
                .edit_message_text(chat_id, message.id, menu_text)
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                .reply_markup(main_menu_keyboard())
                .await
            {
                Ok(_) => {}
                Err(teloxide::RequestError::Api(teloxide::ApiError::MessageNotModified)) => {}
                Err(e) => return Err(Box::new(e)),
            }
            dialogue.update(State::Start).await?;
        }

        bot.answer_callback_query(q.id).await?;
    }
    Ok(())
}
