use serde::{Deserialize, Serialize};
use solana_sdk::signer::{Signer, keypair::Keypair};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Platform {
    Telegram,
    Discord,
}

impl Default for Platform {
    fn default() -> Self {
        Platform::Telegram
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Wallet {
    pub name: String,
    pub public_key: String,
    pub private_key: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BloomWalletInfo {
    pub address: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserConfig {
    pub slippage_percent: u32,
    pub buy_priority_fee_sol: f64,
    pub sell_priority_fee_sol: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Task {
    pub name: String,
    #[serde(default)]
    pub platform: Platform,
    pub listen_channels: Vec<i64>,
    pub listen_channel_name: Option<String>,
    pub listen_users: Vec<i64>,
    #[serde(default)]
    pub listen_usernames: Vec<String>,
    #[serde(default)]
    pub telegram_channel_is_broadcast: bool,
    #[serde(default)]
    pub grammers_session_data: Option<String>,
    #[serde(default)]
    pub telegram_username: Option<String>,
    pub discord_token: Option<String>,
    pub discord_channel_id: Option<String>,
    pub discord_username: Option<String>,
    pub discord_users: Vec<String>,
    pub active: bool,
    pub buy_amount_sol: f64,
    pub buy_priority_fee_sol: f64,
    pub buy_slippage_percent: u32,
    pub blacklist_words: Vec<String>,
    pub inform_only: bool,
    #[serde(default)]
    pub bloom_wallet: Option<BloomWalletInfo>,
}

impl Task {
    pub fn has_telegram_user_session(&self) -> bool {
        self.grammers_session_data
            .as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    }

    pub fn telegram_username_display(&self) -> Option<&str> {
        self.telegram_username
            .as_deref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserData {
    pub wallets: Vec<Wallet>,
    pub default_wallet_index: usize,
    pub config: UserConfig,
    pub tasks: Vec<Task>,
}

impl UserData {
    pub fn get_default_wallet(&self) -> Option<&Wallet> {
        self.wallets.get(self.default_wallet_index)
    }
}

pub fn create_new_wallet(name: String) -> Wallet {
    let keypair = Keypair::new();
    Wallet {
        name,
        public_key: keypair.pubkey().to_string(),
        private_key: keypair.to_base58_string(),
    }
}
