use crate::infrastructure::blockchain::config::QUICKNODE_RPC_URL;
use crate::infrastructure::blockchain::types::RpcClients;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use std::sync::Arc;

pub fn create_rpc_clients() -> RpcClients {
    RpcClients {
        quicknode_client: Arc::new(RpcClient::new(QUICKNODE_RPC_URL.to_string())),
    }
}
