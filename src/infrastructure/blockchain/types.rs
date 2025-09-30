use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use std::sync::Arc;

#[derive(Clone)]
pub struct RpcClients {
    pub quicknode_client: Arc<RpcClient>,
}
