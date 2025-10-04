use parking_lot::Mutex;
use redis::Client as RedisClient;
use reqwest::Client as ReqwestClient;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teloxide::{dispatching::dialogue::InMemStorage, prelude::*};
use tokio::sync::{RwLock, broadcast, mpsc, oneshot};

mod application;
mod infrastructure;
mod interfaces;

use application::health::worker::{WarmerState, WarmupResult, WarmupStatus, run_warmer};
use application::indexer::{preload_from_redis, run_raydium_pool_ingest, run_ws_ingest};
use application::pricing::{SolPriceState, run_price_fetcher};
use infrastructure::blockchain::{RpcClients, create_rpc_clients, run_bloom_ws_listener};
use infrastructure::logging;
use interfaces::bot::State;
use interfaces::bot::handlers::{
    callbacks::callback_handler,
    start::{Command, start},
    text::text_handler,
};
use interfaces::bot::user::client::UserClientHandle;
use interfaces::console::menu::MenuManager;

pub struct BloomBuyAck {
    pub pending_time: Instant,
    pub success_time: Instant,
    pub token_name: Option<String>,
    pub signature: Option<String>,
}

#[derive(Clone)]
pub struct BloomSwapTracker {
    pub mint: String,
    pub side: String,
    pub started_at: Instant,
}

pub static USER_CLIENT_HANDLE: once_cell::sync::Lazy<Arc<Mutex<Option<UserClientHandle>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(None)));
pub static PENDING_BLOOM_RESPONSES: once_cell::sync::Lazy<
    Arc<Mutex<HashMap<String, oneshot::Sender<BloomBuyAck>>>>,
> = once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));
pub static PENDING_BLOOM_INFO: once_cell::sync::Lazy<
    Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
> = once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));
pub static ACTIVE_TASK_SESSIONS: once_cell::sync::Lazy<
    Arc<Mutex<HashMap<(i64, String), uuid::Uuid>>>,
> = once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));
pub static ACTIVE_BLOOM_SWAPS: once_cell::sync::Lazy<
    Arc<Mutex<HashMap<String, BloomSwapTracker>>>,
> = once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BloomWsConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
    Unavailable,
}

impl Default for BloomWsConnectionStatus {
    fn default() -> Self {
        BloomWsConnectionStatus::Connecting
    }
}

#[derive(Default)]
pub struct BloomWsConnectionState {
    pub status: BloomWsConnectionStatus,
    pub message: String,
}
pub static BLOOM_WS_CONNECTION: once_cell::sync::Lazy<Arc<Mutex<BloomWsConnectionState>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(BloomWsConnectionState::default())));
pub static HTTP_CLIENT: once_cell::sync::Lazy<ReqwestClient> = once_cell::sync::Lazy::new(|| {
    ReqwestClient::builder()
        .tcp_keepalive(Duration::from_secs(60))
        .build()
        .expect("Failed to create HTTP client")
});

type MyDialogue = Dialogue<State, teloxide::dispatching::dialogue::InMemStorage<State>>;
type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

fn message_author_id(message: &Message) -> Option<u64> {
    message.from().map(|user| user.id.0)
}

fn is_authorized_user(user_id: Option<u64>, admin_id: u64) -> bool {
    user_id.map(|id| id == admin_id).unwrap_or(false)
}

fn setup_console_ui(warmer_state: WarmerState, redis_url: String) {
    tokio::spawn(async move {
        let mut menu_manager = MenuManager::new(warmer_state, redis_url);
        menu_manager.run().await;
    });
}

async fn run_warmer_task(warmer_state: WarmerState) {
    run_warmer(warmer_state).await;
}

async fn run_price_fetcher_task(price_state: SolPriceState) {
    run_price_fetcher(price_state).await;
}

async fn handle_commands(
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    _cmd: Command,
    redis_client: RedisClient,
    sol_price_state: SolPriceState,
    rpc_clients: RpcClients,
) -> HandlerResult {
    start(
        bot,
        dialogue,
        msg,
        redis_client,
        sol_price_state,
        rpc_clients,
    )
    .await?;
    Ok(())
}

async fn forbidden_message(bot: Bot, msg: Message) -> HandlerResult {
    let user_display = message_author_id(&msg)
        .map(|id| id.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    log::warn!(
        "Unauthorized Telegram message access attempt from user_id={} chat_id={}",
        user_display,
        msg.chat.id.0
    );
    bot.send_message(
        msg.chat.id,
        "🚫 Access forbidden: this bot is restricted to the administrator.",
    )
    .await
    .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> { Box::new(err) })?;
    Ok(())
}

async fn forbidden_callback(bot: Bot, q: CallbackQuery) -> HandlerResult {
    let user_display = q.from.id.0;
    let chat_id = q.message.as_ref().map(|msg| msg.chat.id);
    log::warn!(
        "Unauthorized Telegram callback access attempt from user_id={}",
        user_display
    );
    if let Some(chat) = chat_id {
        bot.send_message(
            chat,
            "🚫 Access forbidden: this bot is restricted to the administrator.",
        )
        .await
        .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> { Box::new(err) })?;
    }
    bot.answer_callback_query(q.id)
        .text("Access forbidden.")
        .show_alert(true)
        .await
        .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> { Box::new(err) })?;
    Ok(())
}

#[tokio::main]
async fn main() {
    logging::init();
    dotenv::dotenv().ok();

    rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider())
        .expect("install rustls ring provider");

    let admin_user_id = env::var("ADMIN_TG_ID")
        .or_else(|_| env::var("ADMIN_TG_IG"))
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(7_507_740_649);
    log::info!("Admin Telegram user id configured as {}", admin_user_id);
    let admin_user_id = Arc::new(admin_user_id);

    if let Err(e) = application::filter::init_word_filter().await {
        log::error!("Failed to initialize word filter: {}", e);
    }

    let rpc_clients = create_rpc_clients();

    tokio::spawn(run_bloom_ws_listener());

    let urls_to_warm = vec![
        "http://eu1.bloom-ext.app".to_string(),
        infrastructure::blockchain::ZEROSLOT_RPC_URL.to_string(),
        infrastructure::blockchain::NODE1_RPC_URL.to_string(),
        infrastructure::blockchain::SHYFT_RPC_URL.to_string(),
        infrastructure::blockchain::HELIUS_RPC_URL.to_string(),
    ];

    let initial_warmer_state: Vec<WarmupResult> = urls_to_warm
        .into_iter()
        .map(|url| WarmupResult {
            url: url.to_string(),
            status: WarmupStatus::Pending,
            latency_ms: None,
            last_checked: None,
        })
        .collect();

    let warmer_state = Arc::new(Mutex::new(initial_warmer_state));
    let redis_url = env::var("REDIS_URL").expect("REDIS_URL must be set");

    if let Err(e) = preload_from_redis(&redis_url).await {
        log::error!("Failed to preload from Redis: {}", e);
    }

    let user_client_handle = Arc::clone(&USER_CLIENT_HANDLE);
    let (client_sender, mut client_receiver) = mpsc::channel::<grammers_client::Client>(1);
    let client_sender_for_dispatcher = client_sender.clone();

    let ui_state_clone = Arc::clone(&warmer_state);
    setup_console_ui(ui_state_clone, redis_url.clone());

    tokio::spawn(async move {
        if let Some(user_client) = client_receiver.recv().await {
            log::info!("Received Grammers client, starting update hub and network loop.");

            let (tx, _rx) =
                broadcast::channel::<interfaces::bot::core::update_bus::UpdateArc>(1024);
            interfaces::bot::core::update_bus::init_with_sender(tx.clone());

            let client_for_loop = user_client.clone();
            tokio::spawn(async move {
                loop {
                    match client_for_loop.next_update().await {
                        Ok(update) => {
                            let timed = interfaces::bot::core::update_bus::TimedUpdate {
                                ts: Instant::now(),
                                update,
                            };
                            let _ = tx.send(Arc::new(timed));
                        }
                        Err(e) => {
                            log::error!("Update hub error: {}", e);
                            break;
                        }
                    }
                }
            });

            tokio::spawn(interfaces::bot::core::bloom_listener::run_bloom_listener(
                user_client.clone(),
            ));

            match user_client.run_until_disconnected().await {
                Ok(_) => log::info!("Grammers client disconnected gracefully."),
                Err(e) => log::error!("Grammers client disconnected with an error: {}", e),
            }
        }
    });

    tokio::spawn(run_ws_ingest());
    tokio::spawn(run_raydium_pool_ingest());

    let warmer_task_state_clone = Arc::clone(&warmer_state);
    tokio::spawn(run_warmer_task(warmer_task_state_clone));

    let sol_price_state = Arc::new(RwLock::new(None));
    let price_fetcher_task_state_clone = Arc::clone(&sol_price_state);
    tokio::spawn(run_price_fetcher_task(price_fetcher_task_state_clone));

    let bot = Bot::from_env();
    let redis_client = RedisClient::open(redis_url).expect("Failed to create Redis client");

    let unauthorized_message_handler = Update::filter_message()
        .filter(|msg: Message, admin_user: Arc<u64>| {
            !is_authorized_user(message_author_id(&msg), *admin_user)
        })
        .endpoint(forbidden_message);

    let unauthorized_callback_handler = Update::filter_callback_query()
        .filter(|q: CallbackQuery, admin_user: Arc<u64>| q.from.id.0 != *admin_user)
        .endpoint(forbidden_callback);

    let dialogue_handler = Update::filter_message()
        .filter(|msg: Message, admin_user: Arc<u64>| {
            is_authorized_user(message_author_id(&msg), *admin_user)
        })
        .enter_dialogue::<Message, InMemStorage<State>, State>()
        .branch(
            dptree::filter_map(|state: State| match state {
                State::TaskSelectChannelSearch { .. } => Some(()),
                _ => None,
            })
            .endpoint(text_handler),
        )
        .branch(dptree::endpoint(text_handler));

    let command_handler = Update::filter_message()
        .filter(|msg: Message, admin_user: Arc<u64>| {
            is_authorized_user(message_author_id(&msg), *admin_user)
        })
        .filter_command::<Command>()
        .enter_dialogue::<Message, InMemStorage<State>, State>()
        .endpoint(handle_commands);

    let callback_query_handler = Update::filter_callback_query()
        .filter(|q: CallbackQuery, admin_user: Arc<u64>| q.from.id.0 == *admin_user)
        .enter_dialogue::<CallbackQuery, InMemStorage<State>, State>()
        .endpoint(callback_handler);

    let handler = dptree::entry()
        .branch(unauthorized_message_handler)
        .branch(unauthorized_callback_handler)
        .branch(command_handler)
        .branch(dialogue_handler)
        .branch(callback_query_handler);

    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![
            InMemStorage::<State>::new(),
            redis_client,
            sol_price_state.clone(),
            user_client_handle,
            rpc_clients,
            Arc::clone(&admin_user_id),
            client_sender_for_dispatcher
        ])
        .enable_ctrlc_handler()
        .build();

    dispatcher.dispatch().await;
}
