use crate::interfaces::bot::data::BloomWalletInfo;
use teloxide::types::MessageId;

#[derive(Clone, Default)]
pub enum State {
    #[default]
    Start,
    SettingsMenu,
    WalletsMenu,
    TasksMenu,
    TaskSettingsMenu {
        _task_name: String,
        _menu_message_id: MessageId,
    },
    TaskSelectBloomWallet {
        task_name: String,
        menu_message_id: MessageId,
        wallets: Vec<BloomWalletInfo>,
        page: usize,
    },
    TaskSelectChannelSearch {
        task_name: String,
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
    },
    TaskSelectChannelFromList {
        task_name: String,
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
        all_channels: Vec<(String, i64)>,
        page: usize,
    },
    TaskSelectUsersFromList {
        task_name: String,
        menu_message_id: MessageId,
        channel_id: i64,
        all_users: Vec<(String, i64, String)>,
        selected_users: Vec<i64>,
        page: usize,
    },
    TaskReceiveName {
        task_name: String,
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
    },
    TaskReceiveBuyAmount {
        task_name: String,
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
    },
    TaskReceiveBuyFee {
        task_name: String,
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
    },
    TaskReceiveBuySlippage {
        task_name: String,
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
    },
    TaskReceiveBlacklist {
        task_name: String,
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
    },
    TaskReceiveDiscordToken {
        task_name: String,
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
    },
    TaskReceiveDiscordChannelId {
        task_name: String,
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
    },
    TaskReceiveDiscordUsers {
        task_name: String,
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
    },
    ReceiveImportKey {
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
    },
    ReceiveWalletName {
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
        private_key: String,
    },
    ReceiveSlippage {
        menu_message_id: MessageId,
        prompt_message_id: MessageId,
    },
}
