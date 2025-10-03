use redis::Client as RedisClient;
use teloxide::{prelude::*, types::ChatId, utils::command::BotCommands};

use crate::application::pricing::SolPriceState;
use crate::infrastructure::blockchain::RpcClients;
use crate::interfaces::bot::{
    State, UserConfig, UserData, create_new_wallet, generate_main_menu_text, get_user_data,
    main_menu_keyboard, save_user_data,
};

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
type MyDialogue = Dialogue<State, teloxide::dispatching::dialogue::InMemStorage<State>>;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Supported commands:")]
pub enum Command {
    #[command(description = "Start the bot and see the main menu.")]
    Start,
}

pub async fn start(
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    redis_client: RedisClient,
    sol_price_state: SolPriceState,
    rpc_clients: RpcClients,
) -> HandlerResult {
    let chat_id = msg.chat.id.0;
    let mut con = redis_client.get_multiplexed_async_connection().await?;
    let user_data = get_user_data(&mut con, chat_id).await?;

    if user_data.is_none() {
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

    let menu_text =
        generate_main_menu_text(redis_client.clone(), chat_id, sol_price_state, rpc_clients).await;
    bot.send_message(ChatId(chat_id), menu_text)
        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        .reply_markup(main_menu_keyboard())
        .await?;

    dialogue.update(State::Start).await?;

    Ok(())
}
