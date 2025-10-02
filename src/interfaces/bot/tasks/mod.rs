pub mod discord;
pub mod scraper;
pub mod state;
pub mod tg;

use crate::interfaces::bot::data::{Task, UserData};

pub use state::{append_task_log, subscribe_task_logs};

pub fn resolve_task_wallet(task: &Task, _user_data: &UserData) -> Option<(String, String)> {
    task.bloom_wallet.as_ref().map(|wallet| {
        let label = wallet
            .label
            .as_deref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or("Bloom Wallet");
        (wallet.address.clone(), label.to_string())
    })
}
