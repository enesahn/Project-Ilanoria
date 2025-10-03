use redis::Client as RedisClient;
use teloxide::{prelude::*, types::ChatId, utils::command::BotCommands};

use crate::application::pricing::SolPriceState;
use crate::infrastructure::blockchain::RpcClients;
use crate::interfaces::bot::handlers::tasks::get_tasks;
use crate::interfaces::bot::{
    State, UserConfig, UserData, create_new_wallet, generate_tasks_text, get_user_data,
    save_user_data, tasks_menu_keyboard,
};

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
type MyDialogue = Dialogue<State, teloxide::dispatching::dialogue::InMemStorage<State>>;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Supported commands:")]
pub enum Command {
    #[command(description = "Start the bot and review your tasks.")]
    Start,
}

pub async fn start(
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    redis_client: RedisClient,
    _sol_price_state: SolPriceState,
    _rpc_clients: RpcClients,
) -> HandlerResult {
    let chat_id = msg.chat.id.0;
    let mut con = redis_client.get_multiplexed_async_connection().await?;
    let has_user_data = get_user_data(&mut con, chat_id).await?.is_some();

    if !has_user_data {
        bot.send_message(
            ChatId(chat_id),
            "Welcome! Creating a new wallet and default settings for you...",
        )
        .await?;
        let initial_wallet = create_new_wallet("Main Wallet".to_string());
        let new_user_data = UserData {
            wallets: vec![initial_wallet],
            default_wallet_index: 0,
            config: UserConfig {
                slippage_percent: 25,
                buy_priority_fee_sol: 0.001,
                sell_priority_fee_sol: 0.001,
            },
            tasks: vec![],
        };
        save_user_data(&mut con, chat_id, &new_user_data).await?;
    }

    let tasks_text = generate_tasks_text(redis_client.clone(), chat_id).await;
    let tasks = get_tasks(redis_client.clone(), chat_id).await;
    bot.send_message(ChatId(chat_id), tasks_text)
        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        .reply_markup(tasks_menu_keyboard(&tasks))
        .await?;

    dialogue.update(State::TasksMenu).await?;

    Ok(())
}
