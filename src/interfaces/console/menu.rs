use crate::application::health::worker::{WarmerState, WarmupStatus};
use crate::application::indexer::{
    IndexerMintLogEntry, indexer_mint_log_counters, ram_index_stats, recent_indexer_mint_logs,
    redis_index_stats, subscribe_indexer_mint_logs,
};
use crate::infrastructure::logging::suppress_stdout_logs;
use crate::interfaces::bot::data::storage::get_user_tasks;
use crate::interfaces::bot::tasks::subscribe_task_logs;
use crate::interfaces::bot::{clear_user_logs, get_all_user_ids, get_user_logs};
use crate::interfaces::console::console::ConsoleUI;
use crate::{BLOOM_WS_CONNECTION, BloomWsConnectionStatus};
use chrono::{DateTime, Local, SecondsFormat};
use colored::*;
use std::io::Write as _;
use tokio::io::{self, AsyncBufReadExt, BufReader};
use tokio::select;

enum MenuState {
    MainMenu,
    ServerLogsRoot,
    WarmerStatus,
    IndexerLogOverview,
    ServerLogsUserList,
    UserLogTypeSelection { user_id: i64 },
    GlobalLogsUserDetail { user_id: i64, page: usize },
    UserTaskLogList { user_id: i64 },
    TaskLiveLogs { user_id: i64, task_name: String },
    IndexerLiveLogs,
    RedisIndex,
    Exiting,
}

pub struct MenuManager {
    state: MenuState,
    warmer_state: WarmerState,
    redis_url: String,
    skip_input_cycle: bool,
}

impl MenuManager {
    pub fn new(warmer_state: WarmerState, redis_url: String) -> Self {
        MenuManager {
            state: MenuState::MainMenu,
            warmer_state,
            redis_url,
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
            MenuState::ServerLogsRoot => self.display_server_logs_root(),
            MenuState::WarmerStatus => self.display_warmer_status(),
            MenuState::IndexerLogOverview => self.display_indexer_log_overview().await,
            MenuState::ServerLogsUserList => self.display_server_logs_user_list().await,
            MenuState::UserLogTypeSelection { user_id } => {
                self.display_user_log_type_selection(*user_id)
            }
            MenuState::GlobalLogsUserDetail { user_id, page } => {
                self.display_global_logs_user_detail(*user_id, *page).await
            }
            MenuState::UserTaskLogList { user_id } => {
                self.display_user_task_log_list(*user_id).await
            }
            MenuState::TaskLiveLogs { user_id, task_name } => {
                let user_id = *user_id;
                let task_name = task_name.clone();
                self.run_task_live_logs(user_id, task_name).await;
            }
            MenuState::IndexerLiveLogs => {
                self.run_indexer_live_logs().await;
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
            MenuState::ServerLogsRoot => self.handle_server_logs_root_input(choice).await,
            MenuState::WarmerStatus => self.handle_warmer_status_input(choice),
            MenuState::IndexerLogOverview => self.handle_indexer_log_overview_input(choice).await,
            MenuState::ServerLogsUserList => self.handle_server_logs_user_list_input(choice).await,
            MenuState::UserLogTypeSelection { .. } => {
                self.handle_user_log_type_selection_input(choice).await
            }
            MenuState::GlobalLogsUserDetail { .. } => {
                self.handle_global_logs_user_detail_input(choice).await
            }
            MenuState::UserTaskLogList { .. } => self.handle_user_task_log_list_input(choice).await,
            MenuState::TaskLiveLogs { .. } => {}
            MenuState::IndexerLiveLogs => {}
            MenuState::RedisIndex => self.handle_redis_index_input(choice).await,
            MenuState::Exiting => {}
        }
    }

    fn display_main_menu(&self) {
        ConsoleUI::print_header("Bot Control Panel");
        ConsoleUI::print_info("Telegram Bot is running in the background.");
        let (status, message) = {
            let state = BLOOM_WS_CONNECTION.lock();
            (state.status, state.message.clone())
        };

        match status {
            BloomWsConnectionStatus::Connected => {
                let text = if message.trim().is_empty() {
                    "Bloom WebSocket connection is healthy."
                } else {
                    message.trim()
                };
                println!("  {} {}", "✓".bright_green().bold(), text.bright_green());
            }
            BloomWsConnectionStatus::Connecting => {
                if message.trim().is_empty() {
                    ConsoleUI::print_info("Bloom WebSocket connection is starting up.");
                } else {
                    ConsoleUI::print_info(message.trim());
                }
            }
            BloomWsConnectionStatus::Unavailable | BloomWsConnectionStatus::Disconnected => {
                if message.trim().is_empty() {
                    ConsoleUI::print_warning("Bloom WebSocket connection is unavailable.");
                } else {
                    ConsoleUI::print_warning(message.trim());
                }
            }
        }
        println!();
        ConsoleUI::print_option(1, "Server Logs");
        ConsoleUI::print_option(2, "Warmer Status");
        ConsoleUI::print_option(3, "Redis CA Index Management");
        println!();
        ConsoleUI::print_exit_option('q', "Exit Server");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    fn display_server_logs_root(&self) {
        ConsoleUI::print_header("Server Logs");
        ConsoleUI::print_option(1, "Indexer Mint Logs");
        ConsoleUI::print_option(2, "User Logs");
        println!();
        ConsoleUI::print_exit_option('0', "Back to Main Menu");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_server_logs_root_input(&mut self, input: &str) {
        match input {
            "1" => self.state = MenuState::IndexerLogOverview,
            "2" => self.state = MenuState::ServerLogsUserList,
            "0" => self.state = MenuState::MainMenu,
            _ => {}
        }
    }

    async fn handle_main_menu_input(&mut self, input: &str) {
        match input {
            "1" => self.state = MenuState::ServerLogsRoot,
            "2" => self.state = MenuState::WarmerStatus,
            "3" => self.state = MenuState::RedisIndex,
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

    async fn display_indexer_log_overview(&self) {
        ConsoleUI::print_header("Indexer Mint Logs");

        let counters = indexer_mint_log_counters();
        let (ram_shards, ram_refs, ram_unique_mints, ram_bytes) = ram_index_stats();
        let ram_mb = (ram_bytes as f64) / 1048576.0;

        let (redis_fields, redis_unique_mints, redis_bytes) =
            match redis_index_stats(&self.redis_url).await {
                Ok(stats) => stats,
                Err(error) => {
                    ConsoleUI::print_error(&format!("Failed to load Redis stats: {}", error));
                    (0, 0, 0)
                }
            };
        let redis_mb = (redis_bytes as f64) / 1048576.0;

        println!("  {}", "Activity Summary".bold().cyan());
        println!(
            "  {:<28}{}",
            "Total indexed mints:",
            format!("{}", counters.total).white()
        );
        println!(
            "  {:<28}{}",
            "pumpfun.ws events:",
            format!("{}", counters.pumpfun).white()
        );
        println!(
            "  {:<28}{}",
            "raydium.ws events:",
            format!("{}", counters.raydium).white()
        );
        println!(
            "  {:<28}{}",
            "other sources:",
            format!("{}", counters.other).white()
        );

        println!();
        println!("  {}", "In-Memory Index".bold().cyan());
        println!(
            "  {:<28}{}",
            "Tracked shard keys:",
            format!("{}", ram_shards).white()
        );
        println!(
            "  {:<28}{}",
            "Stored references:",
            format!("{}", ram_refs).white()
        );
        println!(
            "  {:<28}{}",
            "Unique mints:",
            format!("{}", ram_unique_mints).white()
        );
        println!("  {:<28}{:.2} MB", "Memory footprint:", ram_mb);

        println!();
        println!("  {}", "Redis Index".bold().cyan());
        println!(
            "  {:<28}{}",
            "Stored shard fields:",
            format!("{}", redis_fields).white()
        );
        println!(
            "  {:<28}{}",
            "Redis unique mints:",
            format!("{}", redis_unique_mints).white()
        );
        println!("  {:<28}{:.2} MB", "Redis footprint:", redis_mb);

        let recent = recent_indexer_mint_logs(5);
        println!();
        if recent.is_empty() {
            ConsoleUI::print_info("No indexer activity recorded yet.");
        } else {
            println!("  {}", "Most recent indexed mints".bold().cyan());
            for entry in recent {
                println!("    {}", self.format_indexer_overview_entry(&entry).white());
            }
        }

        println!();
        ConsoleUI::print_option(1, "Live Indexer Logs");
        println!();
        ConsoleUI::print_exit_option('0', "Back to Server Logs");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_indexer_log_overview_input(&mut self, input: &str) {
        match input {
            "1" => self.state = MenuState::IndexerLiveLogs,
            "0" => self.state = MenuState::ServerLogsRoot,
            _ => {}
        }
    }

    fn format_indexer_overview_entry(&self, entry: &IndexerMintLogEntry) -> String {
        let timestamp = entry
            .timestamp
            .with_timezone(&Local)
            .format("%H:%M:%S")
            .to_string();
        let windows_text = if entry.windows.is_empty() {
            "[]".to_string()
        } else {
            let joined = entry
                .windows
                .iter()
                .map(|window| format!("\"{}\"", window))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{}]", joined)
        };
        let status = if entry.was_inserted { "" } else { " (cached)" };
        format!(
            "[{}] {} mint={} shards={} perf={}µs{} windows={}",
            timestamp, entry.source, entry.mint, entry.shards, entry.perf_us, status, windows_text
        )
    }

    async fn display_server_logs_user_list(&self) {
        ConsoleUI::print_header("Server Logs • User Logs");

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
        ConsoleUI::print_exit_option('0', "Back to Server Logs");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_server_logs_user_list_input(&mut self, input: &str) {
        if input == "0" {
            self.state = MenuState::ServerLogsRoot;
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
        ConsoleUI::print_option(2, "See User Task Logs");
        println!();
        ConsoleUI::print_exit_option('0', "Back to User List");
        ConsoleUI::print_refresh_hint();
        ConsoleUI::print_prompt();
    }

    async fn handle_user_log_type_selection_input(&mut self, input: &str) {
        if let MenuState::UserLogTypeSelection { user_id } = self.state {
            match input {
                "1" => self.state = MenuState::GlobalLogsUserDetail { user_id, page: 0 },
                "2" => self.state = MenuState::UserTaskLogList { user_id },
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
                "Real-time stream active. Type [q] or [0] to return, [c] to clear."
                    .truecolor(150, 150, 150)
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
                            } else if trimmed == "c" {
                                clear_footer();
                                ConsoleUI::clear_screen();
                                ConsoleUI::print_header(&format!("Live Logs • {}", task_name));
                                ConsoleUI::print_info("Log view cleared. Waiting for new events...\n");
                                render_footer();
                            } else {
                                clear_prompt_line();
                                draw_prompt_line();
                            }
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

    async fn run_indexer_live_logs(&mut self) {
        let _log_guard = suppress_stdout_logs();

        ConsoleUI::clear_screen();
        ConsoleUI::print_header("Live Indexer Mint Logs");

        let (history, mut receiver) = subscribe_indexer_mint_logs();

        fn windows_line(entry: &IndexerMintLogEntry) -> String {
            if entry.windows.is_empty() {
                "windows=[]".to_string()
            } else {
                let joined = entry
                    .windows
                    .iter()
                    .map(|window| format!("\"{}\"", window))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("windows=[{}]", joined)
            }
        }

        fn status_label(entry: &IndexerMintLogEntry) -> &'static str {
            if entry.was_inserted {
                "new"
            } else {
                "duplicate"
            }
        }

        fn render_entry(entry: &IndexerMintLogEntry) {
            let windows_text = windows_line(entry);
            let timestamp = entry.timestamp.to_rfc3339_opts(SecondsFormat::Secs, true);
            println!("  {}", windows_text.white());
            println!(
                "  {}",
                format!(
                    "{} indexer.index source={} mint={} shards={} perf={}µs status={}",
                    timestamp,
                    entry.source,
                    entry.mint,
                    entry.shards,
                    entry.perf_us,
                    status_label(entry)
                )
                .white()
            );
            println!();
        }

        fn draw_status_line() {
            println!(
                "  {} {}",
                "»".bright_cyan(),
                "Real-time stream active. Type [q] or [0] to return, [c] to clear."
                    .truecolor(150, 150, 150)
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
            ConsoleUI::print_info("No index events yet. Waiting for new mints...\n");
        } else {
            for entry in history {
                render_entry(&entry);
            }
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
                            self.state = MenuState::IndexerLogOverview;
                            self.skip_input_cycle = true;
                            break;
                        }
                        Ok(_) => {
                            let trimmed = input.trim().to_lowercase();
                            if trimmed == "q" || trimmed == "0" {
                                clear_footer();
                                self.state = MenuState::IndexerLogOverview;
                                self.skip_input_cycle = true;
                                break;
                            } else if trimmed == "c" {
                                clear_footer();
                                ConsoleUI::clear_screen();
                                ConsoleUI::print_header("Live Indexer Mint Logs");
                                ConsoleUI::print_info("Log view cleared. Waiting for new mints...\n");
                                render_footer();
                            } else {
                                clear_prompt_line();
                                draw_prompt_line();
                            }
                        }
                        Err(error) => {
                            clear_footer();
                            ConsoleUI::print_error(&format!("Input error: {}", error));
                            self.state = MenuState::IndexerLogOverview;
                            self.skip_input_cycle = true;
                            break;
                        }
                    }
                }
                recv_res = receiver.recv() => {
                    match recv_res {
                        Ok(entry) => {
                            clear_footer();
                            render_entry(&entry);
                            render_footer();
                        }
                        Err(_) => {
                            clear_footer();
                            ConsoleUI::print_error("Indexer log stream ended.");
                            self.state = MenuState::IndexerLogOverview;
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
