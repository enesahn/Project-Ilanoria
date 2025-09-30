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
use application::indexer::{preload_from_redis, run_ws_ingest};
use application::pricing::{SolPriceState, run_price_fetcher};
use infrastructure::blockchain::{RpcClients, create_rpc_clients, run_bloom_ws_listener};
use interfaces::bot::State;
use interfaces::bot::handlers::{
    callbacks::callback_handler,
    start::{Command, start},
    text::text_handler,
    trade::trade_handler,
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
pub static HTTP_CLIENT: once_cell::sync::Lazy<ReqwestClient> = once_cell::sync::Lazy::new(|| {
    ReqwestClient::builder()
        .tcp_keepalive(Duration::from_secs(60))
        .build()
        .expect("Failed to create HTTP client")
});

type MyDialogue = Dialogue<State, teloxide::dispatching::dialogue::InMemStorage<State>>;

fn setup_console_ui(
    warmer_state: WarmerState,
    redis_url: String,
    user_client_handle: Arc<Mutex<Option<UserClientHandle>>>,
    client_sender: mpsc::Sender<grammers_client::Client>,
) {
    tokio::spawn(async move {
        let mut menu_manager =
            MenuManager::new(warmer_state, redis_url, user_client_handle, client_sender);
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
    cmd: Command,
    redis_client: RedisClient,
    sol_price_state: SolPriceState,
    rpc_clients: RpcClients,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        Command::Start => {
            start(
                bot,
                dialogue,
                msg,
                redis_client,
                sol_price_state,
                rpc_clients,
            )
            .await?
        }
        Command::Buy(..) | Command::Sell(..) => {
            trade_handler(bot, msg, cmd, redis_client, rpc_clients).await?
        }
    };
    Ok(())
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    dotenv::dotenv().ok();

    rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider())
        .expect("install rustls ring provider");

    if let Err(e) = application::filter::init_word_filter().await {
        log::error!("Failed to initialize word filter: {}", e);
    }

    let rpc_clients = create_rpc_clients();

    tokio::spawn(run_bloom_ws_listener());

    let urls_to_warm = vec![
        "http://eu1.bloom-ext.app".to_string(),
        infrastructure::blockchain::ZEROSLOT_RPC_URL.to_string(),
        infrastructure::blockchain::NODE1_RPC_URL.to_string(),
        infrastructure::blockchain::QUICKNODE_RPC_URL.to_string(),
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

    let redis_url_clone = redis_url.clone();
    let user_client_handle_clone = Arc::clone(&USER_CLIENT_HANDLE);
    let client_sender_clone = client_sender.clone();

    tokio::spawn(async move {
        match interfaces::bot::user::client::try_auto_login_user_client(redis_url_clone).await {
            Ok((client, handle)) => {
                log::info!("Telegram User Client auto-login successful. Session is active.");
                *user_client_handle_clone.lock() = Some(handle);
                if client_sender_clone.send(client).await.is_err() {
                    log::error!("Failed to send auto-logged in client to its network loop task.");
                }
            }
            Err(_e) => {
                log::warn!(
                    "Telegram User Client auto-login failed. Please log in manually via the console UI."
                );
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let ui_state_clone = Arc::clone(&warmer_state);
    let ui_handle_clone = Arc::clone(&user_client_handle);
    setup_console_ui(
        ui_state_clone,
        redis_url.clone(),
        ui_handle_clone,
        client_sender,
    );

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

    let warmer_task_state_clone = Arc::clone(&warmer_state);
    tokio::spawn(run_warmer_task(warmer_task_state_clone));

    let sol_price_state = Arc::new(RwLock::new(None));
    let price_fetcher_task_state_clone = Arc::clone(&sol_price_state);
    tokio::spawn(run_price_fetcher_task(price_fetcher_task_state_clone));

    let bot = Bot::from_env();
    let redis_client = RedisClient::open(redis_url).expect("Failed to create Redis client");

    let dialogue_handler = Update::filter_message()
        .enter_dialogue::<Message, InMemStorage<State>, State>()
        .branch(
            dptree::filter_map(|state: State| match state {
                State::ReceiveSlippage { .. }
                | State::ReceiveBuyPriorityFee { .. }
                | State::ReceiveSellPriorityFee { .. }
                | State::ReceiveImportKey { .. }
                | State::ReceiveWalletName { .. }
                | State::ReceiveCustomBuyAmount { .. }
                | State::ReceiveCustomSellPercentage { .. }
                | State::TaskSelectChannelSearch { .. } => Some(()),
                _ => None,
            })
            .endpoint(text_handler),
        )
        .branch(dptree::endpoint(text_handler));

    let command_handler = Update::filter_message()
        .filter_command::<Command>()
        .enter_dialogue::<Message, InMemStorage<State>, State>()
        .endpoint(handle_commands);

    let callback_query_handler = Update::filter_callback_query()
        .enter_dialogue::<CallbackQuery, InMemStorage<State>, State>()
        .endpoint(callback_handler);

    let handler = dptree::entry()
        .branch(command_handler)
        .branch(dialogue_handler)
        .branch(callback_query_handler);

    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![
            InMemStorage::<State>::new(),
            redis_client,
            sol_price_state.clone(),
            user_client_handle,
            rpc_clients
        ])
        .enable_ctrlc_handler()
        .build();

    dispatcher.dispatch().await;
}
