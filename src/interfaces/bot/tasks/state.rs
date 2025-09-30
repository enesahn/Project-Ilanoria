use crate::interfaces::bot::{Task, UserData};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

pub static ACTIVE_TASK_STATES: Lazy<DashMap<(i64, String), Arc<RwLock<Task>>>> =
    Lazy::new(|| DashMap::new());
pub static USER_DATA_STATES: Lazy<DashMap<i64, Arc<RwLock<UserData>>>> =
    Lazy::new(|| DashMap::new());

pub async fn ensure_task_state(chat_id: i64, task: Task) -> Arc<RwLock<Task>> {
    let key_name = task.name.clone();
    let key = (chat_id, key_name);
    if let Some(existing) = ACTIVE_TASK_STATES.get(&key) {
        let state = Arc::clone(existing.value());
        drop(existing);
        {
            let mut guard = state.write().await;
            *guard = task;
        }
        return state;
    }
    let state = Arc::new(RwLock::new(task));
    ACTIVE_TASK_STATES.insert(key, Arc::clone(&state));
    state
}

pub fn get_user_data_state(chat_id: i64) -> Option<Arc<RwLock<UserData>>> {
    USER_DATA_STATES
        .get(&chat_id)
        .map(|entry| Arc::clone(entry.value()))
}

pub async fn sync_user_data(chat_id: i64, user_data: &UserData) {
    if let Some(existing) = USER_DATA_STATES.get(&chat_id) {
        let state = Arc::clone(existing.value());
        drop(existing);
        let mut guard = state.write().await;
        *guard = user_data.clone();
    } else {
        let state = Arc::new(RwLock::new(user_data.clone()));
        USER_DATA_STATES.insert(chat_id, state);
    }
    sync_active_tasks(chat_id, &user_data.tasks).await;
}

pub async fn sync_active_tasks(chat_id: i64, tasks: &[Task]) {
    let mut present_names = HashSet::new();
    let mut active_names = HashSet::new();
    for task in tasks {
        present_names.insert(task.name.clone());
        if task.active {
            active_names.insert(task.name.clone());
        }
        ensure_task_state(chat_id, task.clone()).await;
    }
    let mut removals = Vec::new();
    for entry in ACTIVE_TASK_STATES.iter() {
        if entry.key().0 == chat_id {
            let key = entry.key().clone();
            let name = key.1.clone();
            if !present_names.contains(&name) || !active_names.contains(&name) {
                removals.push((key, Arc::clone(entry.value())));
            }
        }
    }
    for (_, state) in &removals {
        let mut guard = state.write().await;
        guard.active = false;
    }
    for (key, _) in removals {
        ACTIVE_TASK_STATES.remove(&key);
    }
}
