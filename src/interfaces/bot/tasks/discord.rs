use crate::BloomBuyAck;
use crate::PENDING_BLOOM_RESPONSES;
use crate::infrastructure::blockchain::bloom_buy;
use crate::interfaces::bot::escape_markdown;
use crate::interfaces::bot::tasks::{append_task_log, resolve_task_wallet, state};
use crate::interfaces::bot::{Task, UserData, log_buffer_to_ca_detection};
use chrono::Local;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tokio::sync::{Mutex, RwLock, oneshot};
use tokio::time::{Duration, sleep};
use tokio_tungstenite::{connect_async, tungstenite::Message};

fn log_task_event(chat_id: i64, task_name: &str, message: impl Into<String>) {
    let message = message.into();
    log::info!("task.discord[{}:{}] {}", chat_id, task_name, message);
    append_task_log(chat_id, task_name, message);
}

async fn send_notification_markdown(chat_id: i64, message: String) {
    let bot = Bot::from_env();
    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "🗑️ Remove",
        "rm",
    )]]);
    let t_send_start = Instant::now();
    let res = bot
        .send_message(ChatId(chat_id), message.clone())
        .parse_mode(ParseMode::MarkdownV2)
        .disable_web_page_preview(true)
        .reply_markup(keyboard.clone())
        .await;
    let send_us = t_send_start.elapsed().as_micros();
    match res {
        Ok(_) => {
            log::info!("perf.telegram_send_us={} chat_id={}", send_us, chat_id);
        }
        Err(e) => {
            let plain = strip_markdown_for_fallback(&message);
            let t_send2 = Instant::now();
            let _ = bot
                .send_message(ChatId(chat_id), plain.clone())
                .disable_web_page_preview(true)
                .reply_markup(keyboard)
                .await;
            let send2_us = t_send2.elapsed().as_micros();
            log::warn!(
                "perf.telegram_send_fallback_us={} err=\"{}\" chat_id={}",
                send2_us,
                e,
                chat_id
            );
        }
    }
}

fn strip_markdown_for_fallback(s: &str) -> String {
    let mut t = s
        .replace("\\|", "|")
        .replace("\\-", "-")
        .replace("\\_", "_")
        .replace("\\*", "*")
        .replace("\\[", "[")
        .replace("\\]", "]")
        .replace("\\(", "(")
        .replace("\\)", ")")
        .replace("\\~", "~")
        .replace("\\`", "`")
        .replace("\\>", ">")
        .replace("\\#", "#")
        .replace("\\+", "+")
        .replace("\\=", "=")
        .replace("\\{", "{")
        .replace("\\}", "}")
        .replace("\\.", ".")
        .replace("\\!", "!");
    t = t.replace("*", "");
    t
}

fn build_buy_success_message(
    mint: &str,
    amount_sol: f64,
    api_ms: u128,
    prio_fee_sol: f64,
    processor_tip_sol: f64,
    ack: Option<BloomBuyAck>,
) -> String {
    let t0 = Instant::now();
    let time_str = escape_markdown(&Local::now().format("%H:%M:%S").to_string());
    let header = format!("🟢 *Buy Successful \\| {}*", time_str);

    let token_line = if let Some(ref a) = ack {
        if let Some(ref t) = a.token_name {
            format!("🔹Token: *{}*\n", escape_markdown(t))
        } else {
            "".to_string()
        }
    } else {
        "".to_string()
    };

    let mint_line = format!("🔹`{}`", escape_markdown(mint));

    let amount_str = escape_markdown(&format!("{:.3}", amount_sol));
    let prio_str = escape_markdown(&format!("{:.3}", prio_fee_sol));
    let tip_str = escape_markdown(&format!("{:.3}", processor_tip_sol));
    let value_line = format!(
        "💰 Value: *{} SOL* \\- Prio Fee: *{} SOL* \\- Processor Tip: *{} SOL*\n",
        amount_str, prio_str, tip_str
    );

    let durations_line = if let Some(ref a) = ack {
        let conf_ms = a.success_time.duration_since(a.pending_time).as_millis();
        let api_s = escape_markdown(&api_ms.to_string());
        let conf_s = escape_markdown(&conf_ms.to_string());
        format!(
            "⏱️ Durations: *API {} ms* • *Confirmation {} ms*\n",
            api_s, conf_s
        )
    } else {
        let api_s = escape_markdown(&api_ms.to_string());
        format!("⏱️ Durations: *API {} ms*\n", api_s)
    };

    let sig_block = if let Some(ref a) = ack {
        if let Some(sig) = &a.signature {
            let sig_esc = escape_markdown(sig);
            let link = format!("https://solscan.io/tx/{}", sig);
            format!("\n`{}`\n🔗 [View on Solscan]({})", sig_esc, link)
        } else {
            "".to_string()
        }
    } else {
        "".to_string()
    };

    let s = format!(
        "{}\n\n{}{}{}\n{}{}",
        header, token_line, mint_line, "\n", value_line, durations_line
    ) + &sig_block;
    let render_us = t0.elapsed().as_micros();
    log::info!("perf.render_us={} mint={}", render_us, mint);
    s
}

async fn process_discord_message(
    message_content: String,
    message_author: String,
    channel_id: String,
    task: Task,
    chat_id: i64,
    user_data_option: Option<UserData>,
    sent_cas: Arc<Mutex<HashSet<String>>>,
    arrival_ts: Instant,
) {
    let hub_queue_us = Instant::now().duration_since(arrival_ts).as_micros();
    log::info!(
        "perf.discord_hub_queue_us={} chat_id={} channel_id={}",
        hub_queue_us,
        chat_id,
        channel_id
    );

    let t_all_start = Instant::now();
    let task_name = task.name.clone();
    log_task_event(
        chat_id,
        &task_name,
        format!(
            "Incoming message from {} in channel {} ({} chars)",
            message_author,
            channel_id,
            message_content.len()
        ),
    );

    let should_listen = task.discord_users.is_empty()
        || task
            .discord_users
            .iter()
            .any(|u| u.to_lowercase() == message_author.to_lowercase());

    if !should_listen {
        log::info!("perf.skip_not_in_listening_set=1 author={}", message_author);
        log_task_event(
            chat_id,
            &task_name,
            format!("Ignored message from {} (not monitored)", message_author),
        );
        return;
    }

    let t_blacklist = Instant::now();
    let detected_words: Vec<_> = task
        .blacklist_words
        .iter()
        .filter(|word| !word.is_empty() && message_content.to_lowercase().contains(*word))
        .cloned()
        .collect();
    let blacklist_check_us = t_blacklist.elapsed().as_micros();
    let blacklist_ms = (blacklist_check_us as f64 / 1000.0).max(0.01);
    log::info!(
        "perf.blacklist_check_us={} hit={}",
        blacklist_check_us,
        if detected_words.is_empty() { 0 } else { 1 }
    );

    if !detected_words.is_empty() {
        let words_str = detected_words.join(", ");
        log_task_event(
            chat_id,
            &task_name,
            format!(
                "Blocked message from {} due to blacklist hit: {}",
                message_author, words_str
            ),
        );
        let notification = format!(
            "🚫 Blacklist word detected in Discord from `{}`\\. Skipping\\.\\.\n\n`{}`",
            escape_markdown(&message_author),
            escape_markdown(&words_str)
        );
        send_notification_markdown(chat_id, notification).await;
        return;
    }

    let mut log_buffer = Vec::new();
    let t_ca_start = Instant::now();
    let mint_opt = crate::interfaces::bot::tasks::scraper::find_mint_in_text(
        &message_content,
        &mut log_buffer,
    )
    .await;
    let ca_extract_us = t_ca_start.elapsed().as_micros();
    let used_llm = log_buffer
        .iter()
        .any(|s| s.contains("Falling back to Groq LLM"));
    let ca_extract_ms = (ca_extract_us as f64 / 1000.0).max(0.01);
    let llm_flag = if used_llm { "yes" } else { "no" };
    let detection_summary = match mint_opt.as_ref() {
        Some(_) => format!(
            "CA detection completed in {:.2} ms (LLM fallback: {})",
            ca_extract_ms, llm_flag
        ),
        None => format!(
            "CA detection completed in {:.2} ms (LLM fallback: {}) - no mint found",
            ca_extract_ms, llm_flag
        ),
    };
    log_task_event(chat_id, &task_name, detection_summary);
    log::info!(
        "perf.ca_extract_us={} ca.used_llm={}",
        ca_extract_us,
        if used_llm { 1 } else { 0 }
    );

    if let Some(mint) = mint_opt {
        tokio::spawn(log_buffer_to_ca_detection(
            chat_id,
            mint.clone(),
            log_buffer,
        ));
        log_task_event(
            chat_id,
            &task_name,
            format!("Detected potential mint {} from {}", mint, message_author),
        );

        let t_dedup = Instant::now();
        let mut sent_cas_guard = sent_cas.lock().await;
        let is_dup = sent_cas_guard.contains(&mint);
        if !is_dup {
            sent_cas_guard.insert(mint.clone());
        }
        drop(sent_cas_guard);
        let dedup_us = t_dedup.elapsed().as_micros();
        log::info!(
            "perf.dedup_us={} dup={}",
            dedup_us,
            if is_dup { 1 } else { 0 }
        );
        let dedup_ms = (dedup_us as f64 / 1000.0).max(0.01);
        log_task_event(
            chat_id,
            &task_name,
            format!(
                "CA deduplication completed in {:.2} ms (duplicate: {})",
                dedup_ms,
                if is_dup { "yes" } else { "no" }
            ),
        );
        if is_dup {
            log::info!("perf.duplicate_ca=1 mint={}", mint);
            log_task_event(
                chat_id,
                &task_name,
                format!("Duplicate mint {} ignored", mint),
            );
            return;
        }

        if task.inform_only {
            let time_str = escape_markdown(&Local::now().format("%H:%M:%S").to_string());
            let total_ms = t_all_start.elapsed().as_millis();

            let header = format!("🔍 *Token Detected \\| {}*", time_str);
            let mint_line = format!("🪙 `{}`", escape_markdown(&mint));
            let channel_line = format!("📢 Discord Channel: *{}*", escape_markdown(&channel_id));
            let sender_line = format!("👤 Sender: *{}*", escape_markdown(&message_author));

            let perf_lines = format!(
                "⚡ *Performance Metrics*\n\
                 ├ Blacklist Check: `{:.2} ms`\n\
                 ├ CA Detection: `{:.2} ms`\n\
                 ├ Dedup Check: `{:.2} ms`\n\
                 └ Total: `{} ms`",
                blacklist_ms, ca_extract_ms, dedup_ms, total_ms
            );

            let preview_text = if message_content.len() <= 200 {
                message_content.to_string()
            } else {
                let mut end = 200.min(message_content.len());
                while end > 0 && !message_content.is_char_boundary(end) {
                    end -= 1;
                }
                message_content[..end].to_string()
            };

            let notification = format!(
                "{}\n\n{}\n{}\n{}\n\n{}\n\n📝 *Message Preview*\n```\n{}\n```",
                header,
                mint_line,
                channel_line,
                sender_line,
                perf_lines,
                escape_markdown(&preview_text)
            );

            log::info!(
                "perf.total_us={} mode=inform_only mint={}",
                total_ms * 1000,
                mint
            );
            log_task_event(
                chat_id,
                &task_name,
                format!(
                    "Processing timings -> blacklist: {:.2} ms | CA detection: {:.2} ms | dedup: {:.2} ms | total: {} ms",
                    blacklist_ms, ca_extract_ms, dedup_ms, total_ms
                ),
            );
            log_task_event(
                chat_id,
                &task_name,
                format!("Inform-only alert dispatched for mint {}", mint),
            );
            send_notification_markdown(chat_id, notification).await;
        } else {
            if let Some(user_data) = user_data_option {
                if let Some((wallet_address, wallet_label)) = resolve_task_wallet(&task, &user_data)
                {
                    let (tx, rx) = oneshot::channel();
                    PENDING_BLOOM_RESPONSES.lock().insert(mint.clone(), tx);

                    let api_request_start_time = Instant::now();
                    let buy_result = bloom_buy(
                        &mint,
                        task.buy_amount_sol,
                        task.buy_slippage_percent,
                        task.buy_priority_fee_sol,
                        wallet_address.as_str(),
                        wallet_label.as_str(),
                    )
                    .await;
                    let api_duration = api_request_start_time.elapsed();
                    let api_us = api_duration.as_micros();
                    let api_ms = (api_us as f64 / 1000.0).max(0.01);
                    log_task_event(
                        chat_id,
                        &task_name,
                        format!(
                            "Bloom buy initiated for mint {} ({} SOL, slippage {}%)",
                            mint, task.buy_amount_sol, task.buy_slippage_percent
                        ),
                    );

                    match buy_result {
                        Ok(_) => {
                            let ack_wait_start = Instant::now();
                            let bot_response_result =
                                tokio::time::timeout(Duration::from_secs(20), rx).await;
                            match bot_response_result {
                                Ok(Ok(ack)) => {
                                    let ack_wait_us = ack_wait_start.elapsed().as_micros();
                                    let signature_opt = ack.signature.clone();
                                    let msg_text = build_buy_success_message(
                                        &mint,
                                        task.buy_amount_sol,
                                        api_duration.as_millis(),
                                        task.buy_priority_fee_sol,
                                        task.buy_priority_fee_sol,
                                        Some(ack),
                                    );
                                    let total_us = t_all_start.elapsed().as_micros();
                                    log::info!(
                                        "perf.api_us={} perf.ack_wait_us={} perf.total_us={} mint={}",
                                        api_us,
                                        ack_wait_us,
                                        total_us,
                                        mint
                                    );
                                    let ack_wait_ms = (ack_wait_us as f64 / 1000.0).max(0.01);
                                    let total_ms = (total_us as f64 / 1000.0).max(0.01);
                                    log_task_event(
                                        chat_id,
                                        &task_name,
                                        format!(
                                            "Bloom buy timings -> API: {:.2} ms | ACK: {:.2} ms (success) | Pipeline total: {:.2} ms",
                                            api_ms, ack_wait_ms, total_ms
                                        ),
                                    );
                                    if let Some(signature) = signature_opt {
                                        log_task_event(
                                            chat_id,
                                            &task_name,
                                            format!(
                                                "Bloom buy confirmed for mint {} (signature {})",
                                                mint, signature
                                            ),
                                        );
                                    } else {
                                        log_task_event(
                                            chat_id,
                                            &task_name,
                                            format!("Bloom buy confirmed for mint {}", mint),
                                        );
                                    }
                                    send_notification_markdown(chat_id, msg_text).await;
                                }
                                Ok(Err(_)) => {
                                    let ack_wait_us = ack_wait_start.elapsed().as_micros();
                                    let msg_text = build_buy_success_message(
                                        &mint,
                                        task.buy_amount_sol,
                                        api_duration.as_millis(),
                                        task.buy_priority_fee_sol,
                                        task.buy_priority_fee_sol,
                                        None,
                                    );
                                    let total_us = t_all_start.elapsed().as_micros();
                                    log::info!(
                                        "perf.api_us={} perf.ack_wait_us={} perf.total_us={} ack_channel_closed=1 mint={}",
                                        api_us,
                                        ack_wait_us,
                                        total_us,
                                        mint
                                    );
                                    let ack_wait_ms = (ack_wait_us as f64 / 1000.0).max(0.01);
                                    let total_ms = (total_us as f64 / 1000.0).max(0.01);
                                    log_task_event(
                                        chat_id,
                                        &task_name,
                                        format!(
                                            "Bloom buy timings -> API: {:.2} ms | ACK: {:.2} ms (channel closed) | Pipeline total: {:.2} ms",
                                            api_ms, ack_wait_ms, total_ms
                                        ),
                                    );
                                    log_task_event(
                                        chat_id,
                                        &task_name,
                                        format!(
                                            "Bloom buy finished for mint {} (ACK channel closed)",
                                            mint
                                        ),
                                    );
                                    send_notification_markdown(chat_id, msg_text).await;
                                }
                                Err(_) => {
                                    let ack_wait_us = ack_wait_start.elapsed().as_micros();
                                    let msg_text = build_buy_success_message(
                                        &mint,
                                        task.buy_amount_sol,
                                        api_duration.as_millis(),
                                        task.buy_priority_fee_sol,
                                        task.buy_priority_fee_sol,
                                        None,
                                    );
                                    let total_us = t_all_start.elapsed().as_micros();
                                    log::info!(
                                        "perf.api_us={} perf.ack_wait_us={} perf.total_us={} ack_timeout=1 mint={}",
                                        api_us,
                                        ack_wait_us,
                                        total_us,
                                        mint
                                    );
                                    let ack_wait_ms = (ack_wait_us as f64 / 1000.0).max(0.01);
                                    let total_ms = (total_us as f64 / 1000.0).max(0.01);
                                    log_task_event(
                                        chat_id,
                                        &task_name,
                                        format!(
                                            "Bloom buy timings -> API: {:.2} ms | ACK: {:.2} ms (timeout) | Pipeline total: {:.2} ms",
                                            api_ms, ack_wait_ms, total_ms
                                        ),
                                    );
                                    log_task_event(
                                        chat_id,
                                        &task_name,
                                        format!(
                                            "Bloom buy pending for mint {} (ACK timeout)",
                                            mint
                                        ),
                                    );
                                    send_notification_markdown(chat_id, msg_text).await;
                                }
                            };
                        }
                        Err(e) => {
                            PENDING_BLOOM_RESPONSES.lock().remove(&mint);
                            let total_us = t_all_start.elapsed().as_micros();
                            log::info!(
                                "perf.api_us={} perf.total_us={} buy_error=1 err=\"{}\" mint={}",
                                api_us,
                                total_us,
                                e,
                                mint
                            );
                            let total_ms = (total_us as f64 / 1000.0).max(0.01);
                            log_task_event(
                                chat_id,
                                &task_name,
                                format!(
                                    "Bloom buy timings -> API: {:.2} ms | Pipeline total: {:.2} ms (failure)",
                                    api_ms, total_ms
                                ),
                            );
                            log_task_event(
                                chat_id,
                                &task_name,
                                format!("Bloom buy failed for mint {}: {}", mint, e),
                            );
                            let error_msg = format!(
                                "❌ *Buy Request Failed*\n\n*Token:* `{}`\n*Error:* `{}`",
                                escape_markdown(&mint),
                                escape_markdown(&e.to_string())
                            );
                            send_notification_markdown(chat_id, error_msg).await;
                        }
                    }
                } else {
                    let total_us = t_all_start.elapsed().as_micros();
                    log::info!("perf.total_us={} no_bloom_wallet=1", total_us);
                    let no_wallet_msg =
                        "❌ *Buy Error*\n\nNo Bloom wallet configured for this task.".to_string();
                    log_task_event(
                        chat_id,
                        &task_name,
                        "Bloom buy skipped: no Bloom wallet configured".to_string(),
                    );
                    send_notification_markdown(chat_id, no_wallet_msg).await;
                }
            }
        }
    } else {
        let total_us = t_all_start.elapsed().as_micros();
        log::info!("perf.total_us={} ca_not_found=1", total_us);
    }
}

pub async fn start_task_monitor(initial_task: Task, chat_id: i64) {
    let task_name = initial_task.name.clone();
    let task_state = state::ensure_task_state(chat_id, initial_task).await;

    let session_id = uuid::Uuid::new_v4();
    {
        let mut sessions = crate::ACTIVE_TASK_SESSIONS.lock();
        sessions.insert((chat_id, task_name.clone()), session_id);
    }
    log::info!(
        "task.discord: registered session_id={} task={}",
        session_id,
        task_name
    );
    log_task_event(
        chat_id,
        &task_name,
        "Discord task monitor started".to_string(),
    );

    let user_data_state = state::get_user_data_state(chat_id);

    tokio::spawn(async move {
        let sent_cas = Arc::new(Mutex::new(HashSet::<String>::new()));
        log::info!(
            "task.discord: worker start user_chat={} task={} session={}",
            chat_id,
            task_name,
            session_id
        );

        loop {
            let current_session_id = {
                let sessions = crate::ACTIVE_TASK_SESSIONS.lock();
                sessions.get(&(chat_id, task_name.clone())).copied()
            };

            if current_session_id != Some(session_id) {
                log::info!(
                    "task.discord: session invalidated stopping session={}",
                    session_id
                );
                log_task_event(
                    chat_id,
                    &task_name,
                    "Discord task monitor stopped (session replaced)".to_string(),
                );
                break;
            }

            let task_snapshot = { task_state.read().await.clone() };

            if !task_snapshot.active {
                log::info!("task.discord: task inactive stopping");
                log_task_event(
                    chat_id,
                    &task_name,
                    "Task deactivated, stopping monitor".to_string(),
                );
                break;
            }

            let token = match task_snapshot.discord_token.clone() {
                Some(t) => t,
                None => {
                    log::warn!("task.discord: no token task={}", task_snapshot.name);
                    log_task_event(
                        chat_id,
                        &task_name,
                        "No Discord token configured; stopping monitor".to_string(),
                    );
                    break;
                }
            };

            let channel_id = match task_snapshot.discord_channel_id.clone() {
                Some(c) => c,
                None => {
                    log::warn!("task.discord: no channel_id task={}", task_snapshot.name);
                    log_task_event(
                        chat_id,
                        &task_name,
                        "No Discord channel configured; stopping monitor".to_string(),
                    );
                    break;
                }
            };

            match connect_discord_gateway(
                token,
                channel_id,
                Arc::clone(&task_state),
                user_data_state.clone(),
                chat_id,
                session_id,
                Arc::clone(&sent_cas),
            )
            .await
            {
                Ok(_) => {
                    log::info!("task.discord: gateway connection ended cleanly");
                    log_task_event(
                        chat_id,
                        &task_name,
                        "Discord gateway connection ended".to_string(),
                    );
                }
                Err(e) => {
                    log::error!("task.discord: gateway error {} - reconnecting in 5s", e);
                    sleep(Duration::from_secs(5)).await;
                    log_task_event(
                        chat_id,
                        &task_name,
                        format!("Discord gateway error: {}. Retrying...", e),
                    );
                }
            }

            let still_active = { task_state.read().await.active };

            if !still_active {
                log::info!("task.discord: task deactivated stopping");
                log_task_event(
                    chat_id,
                    &task_name,
                    "Task deactivated, stopping monitor".to_string(),
                );
                break;
            }
        }
    });
}

async fn connect_discord_gateway(
    token: String,
    channel_id: String,
    task_state: Arc<RwLock<Task>>,
    user_data_state: Option<Arc<RwLock<UserData>>>,
    chat_id: i64,
    session_id: uuid::Uuid,
    sent_cas: Arc<Mutex<HashSet<String>>>,
) -> Result<(), String> {
    let (ws_stream, _) = connect_async("wss://gateway.discord.gg/?v=10&encoding=json")
        .await
        .map_err(|e| format!("WebSocket connection error: {}", e))?;
    let (write, mut read) = ws_stream.split();
    let write = Arc::new(Mutex::new(write));
    let task_name = { task_state.read().await.name.clone() };
    log_task_event(
        chat_id,
        &task_name,
        format!("Connected to Discord gateway for channel {}", channel_id),
    );
    let mut active_channel_id = channel_id;

    while let Some(msg) = read.next().await {
        let msg = msg.map_err(|e| format!("Message receive error: {}", e))?;
        if let Message::Text(text) = msg {
            let payload: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| format!("JSON parse error: {}", e))?;
            let op = payload["op"].as_u64().unwrap_or(0);

            match op {
                10 => {
                    let heartbeat_interval = payload["d"]["heartbeat_interval"].as_u64();
                    let identify = json!({
                        "op": 2,
                        "d": {
                            "token": token.clone(),
                            "properties": {
                                "$os": "linux",
                                "$browser": "rust",
                                "$device": "rust"
                            },
                            "intents": 513
                        }
                    });
                    write
                        .lock()
                        .await
                        .send(Message::Text(identify.to_string()))
                        .await
                        .map_err(|e| format!("Send identify error: {}", e))?;

                    if let Some(interval) = heartbeat_interval {
                        let write_clone = Arc::clone(&write);
                        tokio::spawn(async move {
                            let mut interval_timer =
                                tokio::time::interval(Duration::from_millis(interval));
                            loop {
                                interval_timer.tick().await;
                                let heartbeat = json!({"op": 1, "d": null});
                                let send_result = write_clone
                                    .lock()
                                    .await
                                    .send(Message::Text(heartbeat.to_string()))
                                    .await;
                                if send_result.is_err() {
                                    break;
                                }
                            }
                        });
                    }
                }
                0 => {
                    let event_type = payload["t"].as_str().unwrap_or("");

                    if event_type == "MESSAGE_CREATE" {
                        let current_session_id = {
                            let sessions = crate::ACTIVE_TASK_SESSIONS.lock();
                            sessions.get(&(chat_id, task_name.clone())).copied()
                        };

                        if current_session_id != Some(session_id) {
                            log::info!(
                                "task.discord: session invalidated during message, closing WS session={}",
                                session_id
                            );
                            return Ok(());
                        }

                        let task_snapshot = { task_state.read().await.clone() };

                        if !task_snapshot.active {
                            log::info!("task.discord: task inactive during message, closing WS");
                            return Ok(());
                        }

                        if let Some(current_token) = task_snapshot.discord_token.clone() {
                            if current_token != token {
                                log::info!("task.discord: token changed, restarting session");
                                return Ok(());
                            }
                        }

                        let desired_channel_id = match task_snapshot.discord_channel_id.clone() {
                            Some(id) => id,
                            None => {
                                log::warn!("task.discord: missing channel_id during message");
                                continue;
                            }
                        };

                        if desired_channel_id != active_channel_id {
                            log::info!(
                                "task.discord: channel updated from {} to {}",
                                active_channel_id,
                                desired_channel_id
                            );
                            let previous_channel = active_channel_id.clone();
                            active_channel_id = desired_channel_id.clone();
                            log_task_event(
                                chat_id,
                                &task_name,
                                format!(
                                    "Switched Discord channel from {} to {}",
                                    previous_channel, desired_channel_id
                                ),
                            );
                        }

                        let d = &payload["d"];
                        let msg_channel_id = d["channel_id"].as_str().unwrap_or("");
                        if msg_channel_id != active_channel_id {
                            continue;
                        }

                        let msg_content = d["content"].as_str().unwrap_or("").to_string();
                        let msg_author = d["author"]["username"]
                            .as_str()
                            .unwrap_or("Unknown")
                            .to_string();
                        let arrival_ts = Instant::now();
                        let task_clone = task_snapshot.clone();
                        let user_data_option = match user_data_state.as_ref() {
                            Some(state) => Some(state.read().await.clone()),
                            None => None,
                        };
                        let sent_cas_clone = Arc::clone(&sent_cas);
                        let channel_id_str = msg_channel_id.to_string();

                        tokio::spawn(process_discord_message(
                            msg_content,
                            msg_author,
                            channel_id_str,
                            task_clone,
                            chat_id,
                            user_data_option,
                            sent_cas_clone,
                            arrival_ts,
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}
