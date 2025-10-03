use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use std::sync::Arc;

#[derive(Clone)]
pub struct RpcClients {
    pub helius_client: Arc<RpcClient>,
}
