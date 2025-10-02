use crate::BLOOM_WS_CONNECTION;
use crate::application::health::worker::{WarmerState, WarmupStatus};
use crate::application::indexer::{ram_index_stats, redis_index_stats};
use crate::infrastructure::logging::suppress_stdout_logs;
use crate::interfaces::bot::data::storage::get_user_tasks;
use crate::interfaces::bot::tasks::subscribe_task_logs;
use crate::interfaces::bot::user::client::{UserClientHandle, create_user_client};
use crate::interfaces::bot::{
    clear_user_logs, clear_user_tx_log, get_all_user_ids, get_tx_logs, get_user_logs,
    get_user_tx_signatures,
};
use crate::interfaces::console::console::ConsoleUI;
use chrono::{DateTime, Utc};
use colored::*;
use parking_lot::Mutex;
use regex::Regex;
use std::io::Write as _;
use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, BufReader};
use tokio::select;
use tokio::sync::mpsc;

enum MenuState {
    MainMenu,
    SettingsRoot,
    TelegramUserMenu,
    WarmerStatus,
    ServerLogsUserList,
    UserLogTypeSelection { user_id: i64 },
    GlobalLogsUserDetail { user_id: i64, page: usize },
    TxLogList { user_id: i64, page: usize },
    TxPerformanceSummary { user_id: i64, signature: String },
    TxRawLogs { user_id: i64, signature: String },
    UserTaskLogList { user_id: i64 },
    TaskLiveLogs { user_id: i64, task_name: String },
    RedisIndex,
    Exiting,
}

#[derive(Default, Debug)]
struct PerformanceMetrics {
    blockhash_fetch_ms: Option<f64>,
    dex_params_fetch_ms: Option<f64>,
    preparation_ms: Option<f64>,
    submission_ms: Option<f64>,
    confirmation_ms: Option<f64>,
    total_duration_ms: Option<f64>,
}

pub struct MenuManager {
    state: MenuState,
    warmer_state: WarmerState,
    redis_url: String,
    user_client_handle: Arc<Mutex<Option<UserClientHandle>>>,
    client_sender: mpsc::Sender<grammers_client::Client>,
    skip_input_cycle: bool,
}

struct TxDisplayInfo {
    details: String,
    fee: String,
}

fn format_token_amount(raw_amount: u64, decimals: u8) -> String {
    let divisor = 10f64.powi(decimals as i32);
    if divisor == 0.0 {
        return raw_amount.to_string();
    }
    let amount = raw_amount as f64 / divisor;
    if amount >= 1_000_000_000.0 {
        format!("{:.2}B", amount / 1_000_000_000.0)
    } else if amount >= 1_000_000.0 {
        format!("{:.2}M", amount / 1_000_000.0)
    } else if amount >= 1_000.0 {
        format!("{:.2}K", amount / 1_000.0)
    } else {
        let s = format!("{:.6}", amount);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

impl MenuManager {
    pub fn new(
        warmer_state: WarmerState,
        redis_url: String,
        user_client_handle: Arc<Mutex<Option<UserClientHandle>>>,
        client_sender: mpsc::Sender<grammers_client::Client>,
    ) -> Self {
        MenuManager {
            state: MenuState::MainMenu,
            warmer_state,
            redis_url,
            user_client_handle,
            client_sender,
            skip_input_cycle: false,
        }
    }

    pub async fn run(&mut self) {
        while !matches!(self.state, MenuState::Exiting) {
            self.display_current_menu().await;
            if self.skip_input_cycle {
                self.skip_input_cycle = false;
                continue;
            }
            self.handle_input().await;
        }
        ConsoleUI::print_header("Exiting");
        ConsoleUI::print_info("Shutting down the server...");
    }

    async fn display_current_menu(&mut self) {
        match &self.state {
            MenuState::MainMenu => self.display_main_menu(),
            MenuState::SettingsRoot => self.display_settings_root(),
            MenuState::TelegramUserMenu => self.display_telegram_user_menu().await,
            MenuState::WarmerStatus => self.display_warmer_status(),
            MenuState::ServerLogsUserList => self.display_server_logs_user_list().await,
            MenuState::UserLogTypeSelection { user_id } => {
                self.display_user_log_type_selection(*user_id)
            }
            MenuState::GlobalLogsUserDetail { user_id, page } => {
                self.display_global_logs_user_detail(*user_id, *page).await
            }
            MenuState::TxLogList { user_id, page } => {
                self.display_tx_log_list(*user_id, *page).await
            }
            MenuState::TxPerformanceSummary { user_id, signature } => {
                self.display_tx_performance_summary(*user_id, signature)
                    .await
            }
            MenuState::TxRawLogs { user_id, signature } => {
                self.display_tx_raw_logs(*user_id, signature).await
            }
            MenuState::UserTaskLogList { user_id } => {
                self.display_user_task_log_list(*user_id).await
            }
            MenuState::TaskLiveLogs { user_id, task_name } => {
                let user_id = *user_id;
                let task_name = task_name.clone();
                self.run_task_live_logs(user_id, task_name).await;
            }
            MenuState::RedisIndex => self.display_redis_index().await,
            MenuState::Exiting => {}
        }
    }

    async fn handle_input(&mut self) {
        let mut input = String::new();
        let mut reader = BufReader::new(io::stdin());
        if reader.read_line(&mut input).await.is_err() {
            self.state = MenuState::Exiting;
            return;
        }
        let choice = input.trim();

        if choice.is_empty() {
            return;
        }

        match self.state {
            MenuState::MainMenu => self.handle_main_menu_input(choice).await,
            MenuState::SettingsRoot => self.handle_settings_root_input(choice).await,
            MenuState::TelegramUserMenu => self.handle_telegram_user_menu_input(choice).await,
            MenuState::WarmerStatus => self.handle_warmer_status_input(choice),
            MenuState::ServerLogsUserList => self.handle_server_logs_user_list_input(choice).await,
            MenuState::UserLogTypeSelection { .. } => {
                self.handle_user_log_type_selection_input(choice).await
            }
            MenuState::GlobalLogsUserDetail { .. } => {
                self.handle_global_logs_user_detail_input(choice).await
            }
            MenuState::TxLogList { .. } => self.handle_tx_log_list_input(choice).await,
            MenuState::TxPerformanceSummary { .. } => {
                self.handle_tx_performance_summary_input(choice).await
            }
            MenuState::TxRawLogs { .. } => self.handle_tx_raw_logs_input(choice).await,
            MenuState::UserTaskLogList { .. } => self.handle_user_task_log_list_input(choice).await,
            MenuState::TaskLiveLogs { .. } => {}
            MenuState::RedisIndex => self.handle_redis_index_input(choice).await,
            MenuState::Exiting => {}
        }
    }

    fn display_main_menu(&self) {
        ConsoleUI::print_header("Bot Control Panel");
        ConsoleUI::print_info("Telegram Bot is running in the background.");
        let (is_connected, last_success_at) = {
            let state = BLOOM_WS_CONNECTION.lock();
            (state.is_connected, state.last_success_at)
        };
        if is_connected {
            let detail = last_success_at
                .map(|timestamp| {
                    let datetime: DateTime<Utc> = DateTime::from(timestamp);
                    format!(
                        "Bloom WebSocket handshake validated at {} UTC",
                        datetime.format("%Y-%m-%d %H:%M:%S")
                    )
                })
                .unwrap_or_else(|| "Bloom WebSocket handshake validated".to_string());
            ConsoleUI::print_info(&detail);
        }
        println!();
        ConsoleUI::print_option(1, "Server Logs");
        ConsoleUI::print_option(2, "Settings");
        ConsoleUI::print_option(3, "Warmer Status");
        ConsoleUI::print_option(4, "Redis CA Index Management");
        println!();
        ConsoleUI::print_exit_option('q', "Exit Server");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_main_menu_input(&mut self, input: &str) {
        match input {
            "1" => self.state = MenuState::ServerLogsUserList,
            "2" => self.state = MenuState::SettingsRoot,
            "3" => self.state = MenuState::WarmerStatus,
            "4" => self.state = MenuState::RedisIndex,
            "q" | "Q" => {
                ConsoleUI::clear_screen();
                println!(
                    "\n  {} {}",
                    "⚠".bright_yellow().bold(),
                    "Are you sure you want to exit? (y/n)".bright_yellow()
                );
                ConsoleUI::print_prompt();
                let mut confirm = String::new();
                let mut reader = BufReader::new(io::stdin());
                if reader.read_line(&mut confirm).await.is_ok() {
                    if confirm.trim().eq_ignore_ascii_case("y") {
                        self.state = MenuState::Exiting;
                        std::process::exit(0);
                    }
                }
            }
            _ => {}
        }
    }

    fn display_settings_root(&self) {
        ConsoleUI::print_header("Settings");
        ConsoleUI::print_option(1, "Telegram API User");
        println!();
        ConsoleUI::print_exit_option('0', "Back to Main Menu");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_settings_root_input(&mut self, input: &str) {
        match input {
            "1" => self.state = MenuState::TelegramUserMenu,
            "0" => self.state = MenuState::MainMenu,
            _ => {}
        }
    }

    async fn display_telegram_user_menu(&self) {
        ConsoleUI::print_header("Telegram API User Management");

        let handle_opt = self.user_client_handle.lock().clone();

        if let Some(client) = handle_opt {
            let mut full_name = String::from("N/A");
            let mut username = String::from("N/A");

            if let Ok(me) = client.get_me().await {
                let first_name = me.first_name().unwrap_or("").to_string();
                let last_name = me.last_name().map(|s| s.to_string());

                full_name = match last_name {
                    Some(last) if !last.is_empty() => format!("{} {}", first_name, last),
                    _ => first_name,
                };

                if let Some(u) = me.username() {
                    username = format!("@{}", u);
                }
            }

            let status = "Logged In".bright_green().bold();
            println!("  {} {}", "Status:".bright_white().bold(), status);
            println!();
            println!("  {} {}", "Name:".bright_white(), full_name.bright_cyan());
            println!(
                "  {} {}",
                "Username:".bright_white(),
                username.bright_cyan()
            );
            println!(
                "  {} {}",
                "Connected DC:".bright_white(),
                "N/A".truecolor(150, 150, 150)
            );
            println!(
                "  {} {}",
                "IP Address:".bright_white(),
                "N/A".truecolor(150, 150, 150)
            );
        } else {
            let status = "Not Logged In".bright_red().bold();
            println!("  {} {}", "Status:".bright_white().bold(), status);
            println!();
            ConsoleUI::print_info("The user client is required to fetch data from other bots");
            println!();
            ConsoleUI::print_option(1, "Log In / Create Session");
        }

        println!();
        ConsoleUI::print_exit_option('0', "Back to Settings");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_telegram_user_menu_input(&mut self, input: &str) {
        match input {
            "0" => self.state = MenuState::SettingsRoot,
            "1" => {
                if self.user_client_handle.lock().is_some() {
                    ConsoleUI::print_error("A user is already logged in.");
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    return;
                }
                ConsoleUI::clear_screen();
                ConsoleUI::print_info("Starting Telegram user login process...");
                println!();
                match create_user_client(self.redis_url.clone()).await {
                    Ok((client, handle)) => {
                        *self.user_client_handle.lock() = Some(handle);
                        match self.client_sender.send(client).await {
                            Ok(_) => {
                                ConsoleUI::print_success(
                                    "Login successful! Client is now running.",
                                );
                                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                            }
                            Err(e) => {
                                ConsoleUI::print_error(&format!(
                                    "Failed to activate client: {}",
                                    e
                                ));
                                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                            }
                        }
                    }
                    Err(e) => {
                        ConsoleUI::print_error(&format!("Login failed: {}", e));
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    }
                }
            }
            _ => {}
        }
    }

    fn display_warmer_status(&self) {
        ConsoleUI::print_header("Connection Warmer Status");

        let state = self.warmer_state.lock();

        let url_w = 55;
        let status_w = 12;

        println!(
            "  {:<url_w$}{:<status_w$}{}",
            "URL".bold().cyan(),
            "Status".bold().cyan(),
            "Latency".bold().cyan(),
            url_w = url_w,
            status_w = status_w
        );
        println!("  {}", "─".repeat(url_w + status_w + 12));

        for result in state.iter() {
            let status_text = match result.status {
                WarmupStatus::Success => "OK".green(),
                WarmupStatus::Failed => "Failed".red(),
                WarmupStatus::Pending => "Pending...".yellow(),
            };

            let latency_text = result
                .latency_ms
                .map(|ms| format!("{:.2}ms", ms))
                .unwrap_or_else(|| "N/A".to_string());

            let latency_colored = match result.status {
                WarmupStatus::Success => latency_text.green(),
                WarmupStatus::Failed => latency_text.red(),
                WarmupStatus::Pending => latency_text.yellow(),
            };

            let url_display = if result.url.len() > url_w - 3 {
                format!("{}...", &result.url[..url_w - 6])
            } else {
                result.url.clone()
            };

            println!(
                "  {:<url_w$}{:<status_w$}{}",
                url_display.white(),
                status_text,
                latency_colored,
                url_w = url_w,
                status_w = status_w
            );
        }

        println!();
        ConsoleUI::print_exit_option('0', "Back to Main Menu");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    fn handle_warmer_status_input(&mut self, input: &str) {
        if input == "0" {
            self.state = MenuState::MainMenu;
        }
    }

    async fn display_server_logs_user_list(&self) {
        ConsoleUI::print_header("Server Logs - Select User");

        match get_all_user_ids(&self.redis_url).await {
            Ok(user_ids) => {
                if user_ids.is_empty() {
                    ConsoleUI::print_info("No users found.");
                } else {
                    for (i, user_id) in user_ids.iter().enumerate() {
                        ConsoleUI::print_option((i + 1) as i32, &format!("User ID: {}", user_id));
                    }
                }
            }
            Err(e) => ConsoleUI::print_error(&format!("Failed to fetch users: {}", e)),
        }

        println!();
        ConsoleUI::print_exit_option('0', "Back to Main Menu");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_server_logs_user_list_input(&mut self, input: &str) {
        if input == "0" {
            self.state = MenuState::MainMenu;
            return;
        }
        if let Ok(choice) = input.parse::<usize>() {
            if choice > 0 {
                if let Ok(user_ids) = get_all_user_ids(&self.redis_url).await {
                    if let Some(user_id) = user_ids.get(choice - 1) {
                        self.state = MenuState::UserLogTypeSelection { user_id: *user_id };
                        return;
                    }
                }
            }
        }
    }

    fn display_user_log_type_selection(&self, user_id: i64) {
        ConsoleUI::print_header(&format!("Logs for User ID: {}", user_id));
        ConsoleUI::print_option(1, "See Global Logs");
        ConsoleUI::print_option(2, "See User TX Logs");
        ConsoleUI::print_option(3, "See User Task Logs");
        println!();
        ConsoleUI::print_exit_option('0', "Back to User List");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_user_log_type_selection_input(&mut self, input: &str) {
        if let MenuState::UserLogTypeSelection { user_id } = self.state {
            match input {
                "1" => self.state = MenuState::GlobalLogsUserDetail { user_id, page: 0 },
                "2" => self.state = MenuState::TxLogList { user_id, page: 0 },
                "3" => self.state = MenuState::UserTaskLogList { user_id },
                "0" => self.state = MenuState::ServerLogsUserList,
                _ => {}
            }
        }
    }

    fn format_log_entry(&self, log: &str) -> String {
        if let Some(end_bracket_pos) = log.find(']') {
            if let Some(start_bracket_pos) = log.find('[') {
                if end_bracket_pos > start_bracket_pos {
                    let timestamp_str = &log[start_bracket_pos + 1..end_bracket_pos];
                    let message = &log[end_bracket_pos + 1..].trim();
                    if let Ok(datetime) = DateTime::parse_from_rfc3339(timestamp_str) {
                        let formatted_timestamp = datetime.format("%Y-%m-%d %H:%M:%S").to_string();
                        return format!("[{}] {}", formatted_timestamp, message);
                    }
                }
            }
        }
        log.to_string()
    }

    async fn display_global_logs_user_detail(&self, user_id: i64, page: usize) {
        ConsoleUI::print_header(&format!("Global Logs for User ID: {}", user_id));
        match get_user_logs(&self.redis_url, user_id).await {
            Ok(mut logs) => {
                if logs.is_empty() {
                    ConsoleUI::print_info("No logs found for this user.");
                } else {
                    logs.reverse();
                    let logs_per_page = 10;
                    let total_logs = logs.len();
                    let total_pages = (total_logs as f64 / logs_per_page as f64).ceil() as usize;
                    let current_page = std::cmp::min(page, std::cmp::max(1, total_pages) - 1);
                    let start = current_page * logs_per_page;
                    let end = std::cmp::min(start + logs_per_page, total_logs);
                    if start >= total_logs && total_logs > 0 {
                        ConsoleUI::print_info("No more logs.");
                    } else {
                        let paginated_logs = &logs[start..end];
                        for log in paginated_logs {
                            let formatted_log = self.format_log_entry(log);
                            println!("  {}", formatted_log.white());
                        }
                    }
                    println!();
                    ConsoleUI::print_info(&format!(
                        "Page {}/{}",
                        current_page + 1,
                        std::cmp::max(1, total_pages)
                    ));
                }
            }
            Err(e) => ConsoleUI::print_error(&format!("Failed to fetch logs: {}", e)),
        }
        println!();
        println!(
            "  {} {}",
            "»".bright_cyan(),
            "[n] next • [p] prev • [c] clear logs • [0] back".truecolor(150, 150, 150)
        );
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_global_logs_user_detail_input(&mut self, input: &str) {
        if let MenuState::GlobalLogsUserDetail { user_id, page } = self.state {
            match input.to_lowercase().as_str() {
                "0" => self.state = MenuState::UserLogTypeSelection { user_id },
                "n" => {
                    self.state = MenuState::GlobalLogsUserDetail {
                        user_id,
                        page: page + 1,
                    }
                }
                "p" => {
                    let new_page = if page > 0 { page - 1 } else { 0 };
                    self.state = MenuState::GlobalLogsUserDetail {
                        user_id,
                        page: new_page,
                    };
                }
                "c" => {
                    ConsoleUI::clear_screen();
                    println!(
                        "\n  {} {}",
                        "⚠".bright_yellow().bold(),
                        "Are you sure you want to delete all global logs for this user? (y/n)"
                            .bright_yellow()
                    );
                    ConsoleUI::print_prompt();
                    let mut confirm = String::new();
                    let mut reader = BufReader::new(io::stdin());
                    if reader.read_line(&mut confirm).await.is_ok() {
                        if confirm.trim().eq_ignore_ascii_case("y") {
                            match clear_user_logs(&self.redis_url, user_id).await {
                                Ok(_) => {
                                    ConsoleUI::print_success("Logs cleared successfully.");
                                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                                }
                                Err(e) => {
                                    ConsoleUI::print_error(&format!("Failed to clear logs: {}", e));
                                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                                }
                            }
                            self.state = MenuState::GlobalLogsUserDetail { user_id, page: 0 };
                        } else {
                            ConsoleUI::print_info("Clear logs operation cancelled.");
                            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn parse_tx_details_from_logs(&self, logs: &[String]) -> TxDisplayInfo {
        let mut details = "N/A".to_string();
        let mut fee = "N/A".to_string();
        let mut decimals: Option<u8> = None;
        let details_re = Regex::new(r"DETAILS: (Buy ([\d.]+) SOL|Sell ([\d]+) Tokens)").unwrap();
        let fee_re = Regex::new(r"FEE: ([\d.]+) SOL").unwrap();
        let decimals_re = Regex::new(r"DECIMALS: (\d+)").unwrap();
        for log in logs {
            if let Some(caps) = decimals_re.captures(log) {
                if let Some(d_str) = caps.get(1) {
                    decimals = d_str.as_str().parse::<u8>().ok();
                }
            }
        }
        for log in logs {
            if let Some(caps) = details_re.captures(log) {
                if let Some(buy_amount_str) = caps.get(2) {
                    details = format!("Buy {} SOL", buy_amount_str.as_str());
                } else if let Some(sell_amount_str) = caps.get(3) {
                    if let Ok(amount) = sell_amount_str.as_str().parse::<u64>() {
                        let corrected_amount = amount * 1000;
                        details = format!(
                            "Sell {} Tokens",
                            format_token_amount(corrected_amount, decimals.unwrap_or(9))
                        );
                    }
                }
            }
            if let Some(caps) = fee_re.captures(log) {
                if let Some(fee_amount_str) = caps.get(1) {
                    fee = fee_amount_str.as_str().to_string();
                }
            }
        }
        if fee != "N/A" {
            fee = format!("{} SOL", fee);
        }
        TxDisplayInfo { details, fee }
    }

    async fn display_tx_log_list(&self, user_id: i64, page: usize) {
        ConsoleUI::print_header(&format!("TX Logs for User ID: {}", user_id));
        match get_user_tx_signatures(&self.redis_url, user_id).await {
            Ok(mut signatures) => {
                if signatures.is_empty() {
                    ConsoleUI::print_info("No transaction logs found for this user.");
                } else {
                    signatures.sort();
                    signatures.reverse();
                    let items_per_page = 10;
                    let total_items = signatures.len();
                    let total_pages = (total_items as f64 / items_per_page as f64).ceil() as usize;
                    let current_page = std::cmp::min(page, std::cmp::max(1, total_pages) - 1);
                    let start = current_page * items_per_page;
                    let end = std::cmp::min(start + items_per_page, total_items);
                    println!();
                    println!(
                        "  {:<4} {:<35} {:<25} {}",
                        "ID".cyan(),
                        "SIGNATURE".cyan(),
                        "DETAILS".cyan(),
                        "USED FEE".cyan()
                    );
                    println!("  {}", "─".repeat(85));
                    if start >= total_items && total_items > 0 {
                        ConsoleUI::print_info("No more transactions.");
                    } else {
                        let paginated_sigs = &signatures[start..end];
                        for (i, sig) in paginated_sigs.iter().enumerate() {
                            let display_sig = format!("{}...", &sig[..15]);
                            let logs = get_tx_logs(&self.redis_url, user_id, sig)
                                .await
                                .unwrap_or_default();
                            let info = self.parse_tx_details_from_logs(&logs);
                            println!(
                                "  {:<4} {:<35} {:<25} {}",
                                format!("[{}]", i + 1).blue(),
                                display_sig.white(),
                                info.details.yellow(),
                                info.fee.white()
                            );
                        }
                    }
                    println!();
                    ConsoleUI::print_info(&format!(
                        "Page {}/{}",
                        current_page + 1,
                        std::cmp::max(1, total_pages)
                    ));
                }
            }
            Err(e) => {
                ConsoleUI::print_error(&format!("Failed to fetch transaction signatures: {}", e))
            }
        }
        println!();
        println!(
            "  {} {}",
            "»".bright_cyan(),
            "Enter number to view • [n] next • [p] prev • [0] back".truecolor(150, 150, 150)
        );
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_tx_log_list_input(&mut self, input: &str) {
        if let MenuState::TxLogList { user_id, page } = self.state {
            match input.to_lowercase().as_str() {
                "0" => self.state = MenuState::UserLogTypeSelection { user_id },
                "n" => {
                    self.state = MenuState::TxLogList {
                        user_id,
                        page: page + 1,
                    }
                }
                "p" => {
                    let new_page = if page > 0 { page - 1 } else { 0 };
                    self.state = MenuState::TxLogList {
                        user_id,
                        page: new_page,
                    };
                }
                _ => {
                    if let Ok(choice) = input.parse::<usize>() {
                        if choice > 0 {
                            if let Ok(mut signatures) =
                                get_user_tx_signatures(&self.redis_url, user_id).await
                            {
                                signatures.sort();
                                signatures.reverse();
                                let items_per_page = 10;
                                let page_relative_index = choice - 1;
                                if page_relative_index < items_per_page {
                                    let absolute_index =
                                        page * items_per_page + page_relative_index;
                                    if let Some(signature) = signatures.get(absolute_index) {
                                        self.state = MenuState::TxPerformanceSummary {
                                            user_id,
                                            signature: signature.clone(),
                                        };
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn calculate_performance_metrics_measured(&self, logs: &[String]) -> PerformanceMetrics {
        let mut metrics = PerformanceMetrics::default();
        let re = Regex::new(r#"PERF_METRIC::\{ "stage": "([^"]+)", "duration_ms": ([\d.]+) \}"#)
            .unwrap();
        for log in logs {
            if let Some(caps) = re.captures(log) {
                let stage = &caps[1];
                if let Ok(duration) = caps[2].parse::<f64>() {
                    match stage {
                        "blockhash_fetch" => metrics.blockhash_fetch_ms = Some(duration),
                        "dex_params_fetch" => metrics.dex_params_fetch_ms = Some(duration),
                        "preparation" => metrics.preparation_ms = Some(duration),
                        "submission" => metrics.submission_ms = Some(duration),
                        "confirmation" => metrics.confirmation_ms = Some(duration),
                        "total_duration" => metrics.total_duration_ms = Some(duration),
                        _ => {}
                    }
                }
            }
        }
        metrics
    }

    async fn display_tx_performance_summary(&self, user_id: i64, signature: &str) {
        let short_signature = if signature.len() > 15 {
            format!("{}...", &signature[..15])
        } else {
            signature.to_string()
        };
        ConsoleUI::print_header(&format!("Performance for TX: {}", short_signature));
        match get_tx_logs(&self.redis_url, user_id, signature).await {
            Ok(logs) => {
                if logs.is_empty() {
                    ConsoleUI::print_info("No logs found for this transaction to analyze.");
                } else {
                    let metrics = self.calculate_performance_metrics_measured(&logs);
                    println!(
                        "  {:<35} {}",
                        "Metric".cyan().bold(),
                        "Duration".cyan().bold()
                    );
                    println!("  {}", "─".repeat(50));
                    let display_metric = |label: &str, value: Option<f64>| {
                        let val_str = value
                            .map(|v| format!("{:.4} ms", v))
                            .unwrap_or_else(|| "N/A".to_string());
                        println!("  {:<35} {}", label.white(), val_str.green());
                    };
                    display_metric("Blockhash Fetch Time:", metrics.blockhash_fetch_ms);
                    display_metric("DEX Parameters Fetch Time:", metrics.dex_params_fetch_ms);
                    display_metric("Preparation Time:", metrics.preparation_ms);
                    display_metric("Submission (HTTP) Time:", metrics.submission_ms);
                    display_metric("Confirmation (RPC) Time:", metrics.confirmation_ms);
                    println!("  {}", "─".repeat(50));
                    let total_val = metrics
                        .total_duration_ms
                        .map(|v| format!("{:.4} ms", v))
                        .unwrap_or_else(|| "N/A".to_string());
                    println!(
                        "  {:<35} {}",
                        "Total Measured Duration:".white().bold(),
                        total_val.yellow().bold()
                    );
                }
            }
            Err(e) => ConsoleUI::print_error(&format!("Failed to fetch logs: {}", e)),
        }
        println!();
        ConsoleUI::print_option(1, "See Raw TX Logs");
        println!();
        ConsoleUI::print_exit_option('0', "Back to TX List");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_tx_performance_summary_input(&mut self, input: &str) {
        if let MenuState::TxPerformanceSummary {
            user_id,
            ref signature,
        } = self.state
        {
            match input {
                "1" => {
                    self.state = MenuState::TxRawLogs {
                        user_id,
                        signature: signature.clone(),
                    }
                }
                "0" => self.state = MenuState::TxLogList { user_id, page: 0 },
                _ => {}
            }
        }
    }

    async fn display_tx_raw_logs(&self, user_id: i64, signature: &str) {
        let short_signature = if signature.len() > 15 {
            format!("{}...", &signature[..15])
        } else {
            signature.to_string()
        };
        ConsoleUI::print_header(&format!("Raw Logs for TX: {}", short_signature));
        match get_tx_logs(&self.redis_url, user_id, signature).await {
            Ok(logs) => {
                if logs.is_empty() {
                    ConsoleUI::print_info("No logs found for this transaction.");
                } else {
                    for log in logs {
                        let formatted_log = self.format_log_entry(&log);
                        println!("  {}", formatted_log.white());
                    }
                }
            }
            Err(e) => ConsoleUI::print_error(&format!("Failed to fetch logs: {}", e)),
        }
        println!();
        println!(
            "  {} {}",
            "»".bright_cyan(),
            "[c] clear logs • [0] back".truecolor(150, 150, 150)
        );
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn display_user_task_log_list(&self, user_id: i64) {
        ConsoleUI::print_header(&format!("Tasks for User ID: {}", user_id));
        match get_user_tasks(&self.redis_url, user_id).await {
            Ok(tasks) => {
                if tasks.is_empty() {
                    ConsoleUI::print_info("No tasks available for this user.");
                } else {
                    for (index, task) in tasks.iter().enumerate() {
                        let status_icon = if task.active { "🟢" } else { "🔴" };
                        ConsoleUI::print_option(
                            (index + 1) as i32,
                            &format!("{} {}", status_icon, task.name),
                        );
                    }
                }
            }
            Err(e) => ConsoleUI::print_error(&format!("Failed to fetch tasks: {}", e)),
        }
        println!();
        ConsoleUI::print_exit_option('0', "Back to Log Types");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_tx_raw_logs_input(&mut self, input: &str) {
        if let MenuState::TxRawLogs {
            user_id,
            ref signature,
        } = self.state
        {
            match input.to_lowercase().as_str() {
                "0" => {
                    self.state = MenuState::TxPerformanceSummary {
                        user_id,
                        signature: signature.clone(),
                    }
                }
                "c" => {
                    ConsoleUI::clear_screen();
                    println!(
                        "\n  {} {}",
                        "⚠".bright_yellow().bold(),
                        "Are you sure you want to delete logs for this transaction? (y/n)"
                            .bright_yellow()
                    );
                    ConsoleUI::print_prompt();
                    let mut confirm = String::new();
                    let mut reader = BufReader::new(io::stdin());
                    if reader.read_line(&mut confirm).await.is_ok() {
                        if confirm.trim().eq_ignore_ascii_case("y") {
                            match clear_user_tx_log(&self.redis_url, user_id, signature).await {
                                Ok(_) => {
                                    ConsoleUI::print_success("TX logs cleared successfully.");
                                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                                }
                                Err(e) => {
                                    ConsoleUI::print_error(&format!("Failed to clear logs: {}", e));
                                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                                }
                            }
                            self.state = MenuState::TxLogList { user_id, page: 0 };
                        } else {
                            ConsoleUI::print_info("Clear logs operation cancelled.");
                            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    async fn handle_user_task_log_list_input(&mut self, input: &str) {
        if let MenuState::UserTaskLogList { user_id } = self.state {
            match input {
                "0" => self.state = MenuState::UserLogTypeSelection { user_id },
                _ => {
                    if let Ok(index) = input.parse::<usize>() {
                        if index > 0 {
                            match get_user_tasks(&self.redis_url, user_id).await {
                                Ok(tasks) => {
                                    if let Some(task) = tasks.get(index - 1) {
                                        self.state = MenuState::TaskLiveLogs {
                                            user_id,
                                            task_name: task.name.clone(),
                                        };
                                        return;
                                    }
                                }
                                Err(e) => {
                                    ConsoleUI::print_error(&format!("Failed to load tasks: {}", e));
                                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn run_task_live_logs(&mut self, user_id: i64, task_name: String) {
        let _log_guard = suppress_stdout_logs();

        ConsoleUI::clear_screen();
        ConsoleUI::print_header(&format!("Live Logs • {}", task_name));

        let (history, mut receiver) = subscribe_task_logs(user_id, &task_name);

        fn draw_status_line() {
            println!(
                "  {} {}",
                "»".bright_cyan(),
                "Real-time stream active. Type [q] or [0] to return.".truecolor(150, 150, 150)
            );
        }

        fn draw_prompt_line() {
            print!("  {} ", "❯".bright_green().bold());
            let _ = std::io::stdout().flush();
        }

        fn render_footer() {
            draw_status_line();
            draw_prompt_line();
        }

        fn clear_footer() {
            print!("\x1B[1F\x1B[0J");
            let _ = std::io::stdout().flush();
        }

        fn clear_prompt_line() {
            print!("\x1B[1F\x1B[0J");
            let _ = std::io::stdout().flush();
        }

        if history.is_empty() {
            ConsoleUI::print_info("No logs yet. Waiting for new events...\n");
        } else {
            for entry in history {
                println!("  {}", entry.white());
            }
            println!();
        }

        render_footer();

        let mut reader = BufReader::new(io::stdin());
        let mut input = String::new();

        loop {
            input.clear();
            select! {
                read_res = reader.read_line(&mut input) => {
                    match read_res {
                        Ok(0) => {
                            clear_footer();
                            self.state = MenuState::UserTaskLogList { user_id };
                            self.skip_input_cycle = true;
                            break;
                        }
                        Ok(_) => {
                            let trimmed = input.trim().to_lowercase();
                            if trimmed == "q" || trimmed == "0" {
                                clear_footer();
                                self.state = MenuState::UserTaskLogList { user_id };
                                self.skip_input_cycle = true;
                                break;
                            }
                            clear_prompt_line();
                            draw_prompt_line();
                        }
                        Err(e) => {
                            clear_footer();
                            ConsoleUI::print_error(&format!("Input error: {}", e));
                            self.state = MenuState::UserTaskLogList { user_id };
                            self.skip_input_cycle = true;
                            break;
                        }
                    }
                }
                recv_res = receiver.recv() => {
                    match recv_res {
                        Ok(log_line) => {
                            clear_footer();
                            println!("  {}", log_line.white());
                            let _ = std::io::stdout().flush();
                            render_footer();
                        }
                        Err(_) => {
                            clear_footer();
                            ConsoleUI::print_info("Log stream ended.");
                            self.state = MenuState::UserTaskLogList { user_id };
                            self.skip_input_cycle = true;
                            break;
                        }
                    }
                }
            }
        }
    }

    async fn display_redis_index(&self) {
        ConsoleUI::print_header("Redis CA Index Management");

        let (ram_shards, ram_refs, ram_unique_mints, ram_bytes) = ram_index_stats();
        let ram_mb = (ram_bytes as f64) / 1048576.0;
        let dup_refs = if ram_refs > ram_shards {
            ram_refs - ram_shards
        } else {
            0
        };
        let dup_exists = dup_refs > 0;

        let (redis_hlen, redis_unique_mints, redis_bytes) =
            match redis_index_stats(&self.redis_url).await {
                Ok(stats) => stats,
                Err(e) => {
                    log::error!("Failed to get Redis stats: {}", e);
                    (0, 0, 0)
                }
            };
        let redis_mb = (redis_bytes as f64) / 1048576.0;

        let total_space = 58f64.powi(7);
        let prob_percent = if ram_unique_mints > 1 {
            ((ram_unique_mints - 1) as f64 / total_space) * 100.0
        } else {
            0.0
        };

        let col_w = 20usize;
        let sep = "─".repeat(col_w * 4 + 6);

        println!();
        println!(
            "  {:<col_w$}  {:<col_w$}  {:<col_w$}  {:<col_w$}",
            "In-Memory Keys".bold().cyan(),
            "In-Memory Refs".bold().cyan(),
            "In-Memory Uniq".bold().cyan(),
            "In-Memory Size".bold().cyan(),
            col_w = col_w
        );
        println!("  {}", sep);
        println!(
            "  {:<col_w$}  {:<col_w$}  {:<col_w$}  {:<col_w$}",
            format!("{}", ram_shards).white(),
            format!("{}", ram_refs).white(),
            format!("{}", ram_unique_mints).white(),
            format!("{:.2} MB", ram_mb).white(),
            col_w = col_w
        );

        println!();
        println!(
            "  {:<col_w$}  {:<col_w$}  {:<col_w$}  {:<col_w$}",
            "Redis Fields".bold().cyan(),
            "Redis Uniq".bold().cyan(),
            "Redis Size".bold().cyan(),
            "Dup Shard Pieces".bold().cyan(),
            col_w = col_w
        );
        println!("  {}", sep);
        println!(
            "  {:<col_w$}  {:<col_w$}  {:<col_w$}  {:<col_w$}",
            format!("{}", redis_hlen).white(),
            format!("{}", redis_unique_mints).white(),
            format!("{:.2} MB", redis_mb).white(),
            if dup_exists {
                format!("YES ({})", dup_refs).yellow().to_string()
            } else {
                "NO (0)".green().to_string()
            },
            col_w = col_w
        );

        println!();
        let info_line = format!(
            "7-char shard collision across mints P≈{:.10}% [p:{} m:{}]",
            prob_percent, ram_shards, ram_unique_mints
        );
        ConsoleUI::print_info(&info_line);

        println!();
        ConsoleUI::print_exit_option('0', "Back to Main Menu");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_redis_index_input(&mut self, input: &str) {
        match input {
            "0" => self.state = MenuState::MainMenu,
            _ => {}
        }
    }
}
