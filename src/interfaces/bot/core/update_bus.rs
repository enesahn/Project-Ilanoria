use once_cell::sync::OnceCell;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::broadcast;

pub struct TimedUpdate {
    pub ts: Instant,
    pub update: grammers_client::Update,
}

pub type UpdateArc = Arc<TimedUpdate>;

static UPDATE_TX: OnceCell<broadcast::Sender<UpdateArc>> = OnceCell::new();

pub fn init_with_sender(tx: broadcast::Sender<UpdateArc>) {
    let _ = UPDATE_TX.set(tx);
}

pub fn subscribe() -> broadcast::Receiver<UpdateArc> {
    UPDATE_TX
        .get()
        .expect("UPDATE_TX not initialized")
        .subscribe()
}
