use crate::infrastructure::blockchain::config::HELIUS_RPC_URL;
use crate::infrastructure::blockchain::types::RpcClients;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use std::sync::Arc;

pub fn create_rpc_clients() -> RpcClients {
    RpcClients {
        helius_client: Arc::new(RpcClient::new(HELIUS_RPC_URL.to_string())),
    }
}
