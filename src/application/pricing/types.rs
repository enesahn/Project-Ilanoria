use std::sync::Arc;
use tokio::sync::RwLock;

pub type SolPriceState = Arc<RwLock<Option<u32>>>;
