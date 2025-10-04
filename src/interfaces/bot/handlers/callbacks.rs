use grammers_client::Client as TelegramClient;
use parking_lot::Mutex;
use redis::Client as RedisClient;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::mpsc::Sender;

use crate::application::pricing::SolPriceState;
use crate::infrastructure::blockchain::RpcClients;
use crate::interfaces::bot::handlers::tasks::get_tasks;
use crate::interfaces::bot::user::client::UserClientHandle;
use crate::interfaces::bot::{State, generate_tasks_text, tasks_menu_keyboard};

use super::{tasks, trade};

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
    client_sender: Sender<TelegramClient>,
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

        let trade_actions = ["r"];

        if data.starts_with("task_") || data == "view_tasks" || data == "create_task" {
            tasks::handle_task_callbacks(
                q.clone(),
                bot.clone(),
                redis_client,
                dialogue,
                sol_price_state.clone(),
                user_client_handle.clone(),
                client_sender.clone(),
            )
            .await?;
        } else if trade_actions.iter().any(|&action| data.starts_with(action)) {
            trade::handle_trade_callback(
                q.clone(),
                bot.clone(),
                redis_client,
                dialogue,
                sol_price_state,
                user_client_handle,
                rpc_clients,
                client_sender,
            )
            .await?;
        } else if data == "main_menu" {
            let tasks_text = generate_tasks_text(redis_client.clone(), chat_id.0).await;
            let tasks = get_tasks(redis_client.clone(), chat_id.0).await;
            bot.edit_message_text(chat_id, message.id, tasks_text)
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                .reply_markup(tasks_menu_keyboard(&tasks))
                .await?;
            dialogue.update(State::TasksMenu).await?;
        }

        bot.answer_callback_query(q.id).await?;
    }
    Ok(())
}
