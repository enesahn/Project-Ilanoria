use crate::BloomBuyAck;
use crate::infrastructure::blockchain::bloom_buy;
use crate::interfaces::bot::core::update_bus;
use crate::interfaces::bot::escape_markdown;
use crate::interfaces::bot::tasks::{append_task_log, state};
use crate::interfaces::bot::{Task, UserData, log_buffer_to_ca_detection};
use crate::{PENDING_BLOOM_RESPONSES, USER_CLIENT_HANDLE};
use chrono::Local;
use grammers_client::types::Chat;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tokio::sync::{Mutex, oneshot};
use tokio::time::Duration;

fn log_task_event(chat_id: i64, task_name: &str, message: impl Into<String>) {
    let message = message.into();
    log::info!("task.telegram[{}:{}] {}", chat_id, task_name, message);
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

fn format_sender_name(sender: Option<&Chat>) -> String {
    match sender {
        Some(Chat::User(user)) => {
            let mut name = user.first_name().unwrap_or("").to_string();
            if let Some(last_name) = user.last_name() {
                name.push(' ');
                name.push_str(last_name);
            }
            name
        }
        Some(chat) => chat.name().unwrap_or("Unknown Sender").to_string(),
        None => "Unknown Sender".to_string(),
    }
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

async fn process_message(
    msg: grammers_client::types::update::Message,
    task: Task,
    chat_id: i64,
    user_data_option: Option<UserData>,
    sent_cas: Arc<Mutex<HashSet<String>>>,
    arrival_ts: Instant,
) {
    let hub_queue_us = Instant::now().duration_since(arrival_ts).as_micros();
    log::info!(
        "perf.hub_queue_us={} chat_id={} chan_id={} msg_id={}",
        hub_queue_us,
        chat_id,
        msg.chat().id(),
        msg.id()
    );

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i128;
    let sent_ms = msg.date().timestamp_millis() as i128;
    let srv_ts_delta_ms = (now_ms - sent_ms).max(0) as u128;
    log::info!(
        "perf.srv_ts_delta_ms={} chat_id={} msg_id={}",
        srv_ts_delta_ms,
        chat_id,
        msg.id()
    );

    let t_all_start = Instant::now();
    let sender = msg.sender();
    let message_chat_id = msg.chat().id();
    let effective_sender_id = match sender.as_ref() {
        Some(s) => s.id(),
        None => message_chat_id,
    };

    let t_blacklist = Instant::now();
    let should_listen =
        task.listen_users.is_empty() || task.listen_users.contains(&effective_sender_id);
    let message_text = msg.text();
    let sender_name = format_sender_name(sender.as_ref());
    let channel_name = task
        .listen_channel_name
        .as_deref()
        .unwrap_or("Unknown Channel");
    let detected_words: Vec<_> = task
        .blacklist_words
        .iter()
        .filter(|word| !word.is_empty() && message_text.to_lowercase().contains(*word))
        .cloned()
        .collect();
    let blacklist_check_us = t_blacklist.elapsed().as_micros();
    log::info!(
        "perf.blacklist_check_us={} hit={}",
        blacklist_check_us,
        if detected_words.is_empty() { 0 } else { 1 }
    );
    let task_name = task.name.clone();
    log_task_event(
        chat_id,
        &task_name,
        format!(
            "Incoming message in channel {} (sender {}, {} chars)",
            message_chat_id,
            sender_name,
            message_text.len()
        ),
    );

    if !should_listen {
        log::info!("perf.skip_not_in_listening_set=1");
        log_task_event(
            chat_id,
            &task_name,
            format!(
                "Ignored message from {} (not monitored)",
                format_sender_name(sender.as_ref())
            ),
        );
        return;
    }

    if !detected_words.is_empty() {
        let words_str = detected_words.join(", ");
        log_task_event(
            chat_id,
            &task_name,
            format!(
                "Blocked message from {} due to blacklist hit: {}",
                format_sender_name(sender.as_ref()),
                words_str
            ),
        );
        let notification = format!(
            "🚫 Blacklist word detected in `{}` from `{}`\\. Skipping\\.\\.\n\n`{}`",
            escape_markdown(channel_name),
            escape_markdown(&sender_name),
            escape_markdown(&words_str)
        );
        send_notification_markdown(chat_id, notification).await;
        return;
    }

    let mut log_buffer = Vec::new();
    let t_ca_start = Instant::now();
    let mint_opt =
        crate::interfaces::bot::tasks::scraper::find_mint_in_text(message_text, &mut log_buffer)
            .await;
    let ca_extract_us = t_ca_start.elapsed().as_micros();
    let used_llm = log_buffer
        .iter()
        .any(|s| s.contains("Falling back to Groq LLM"));
    log::info!(
        "perf.ca_extract_us={} ca.used_llm={} msg_id={}",
        ca_extract_us,
        if used_llm { 1 } else { 0 },
        msg.id()
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
            format!("Detected potential mint {} from {}", mint, sender_name),
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
            let blacklist_ms = (blacklist_check_us as f64 / 1000.0).max(0.01);
            let ca_extract_ms = (ca_extract_us as f64 / 1000.0).max(0.01);
            let dedup_ms = (dedup_us as f64 / 1000.0).max(0.01);

            let header = format!("🔍 *Token Detected \\| {}*", time_str);
            let mint_line = format!("🪙 `{}`", escape_markdown(&mint));
            let channel_line = format!("📢 Channel: *{}*", escape_markdown(channel_name));
            let sender_line = format!("👤 Sender: *{}*", escape_markdown(&sender_name));

            let perf_lines = format!(
                "⚡ *Performance Metrics*\n\
                 ├ Blacklist Check: `{:.2} ms`\n\
                 ├ CA Detection: `{:.2} ms`\n\
                 ├ Dedup Check: `{:.2} ms`\n\
                 └ Total: `{} ms`",
                blacklist_ms, ca_extract_ms, dedup_ms, total_ms
            );

            let preview_text = if message_text.len() <= 200 {
                message_text.to_string()
            } else {
                let mut end = 200.min(message_text.len());
                while end > 0 && !message_text.is_char_boundary(end) {
                    end -= 1;
                }
                message_text[..end].to_string()
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
                format!("Inform-only alert dispatched for mint {}", mint),
            );
            send_notification_markdown(chat_id, notification).await;
        } else {
            if let Some(user_data) = user_data_option {
                if let Some(wallet) = user_data.get_default_wallet() {
                    let (tx, rx) = oneshot::channel();
                    PENDING_BLOOM_RESPONSES.lock().insert(mint.clone(), tx);

                    let api_request_start_time = Instant::now();
                    let buy_result = bloom_buy(
                        &mint,
                        task.buy_amount_sol,
                        task.buy_slippage_percent,
                        task.buy_priority_fee_sol,
                        &wallet.public_key,
                    )
                    .await;
                    let api_duration = api_request_start_time.elapsed();
                    let api_us = api_duration.as_micros();
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
                    log::info!("perf.total_us={} no_default_wallet=1", total_us);
                    let no_wallet_msg =
                        "❌ *Buy Error*\n\nNo default wallet found for auto\\-buy task\\."
                            .to_string();
                    log_task_event(
                        chat_id,
                        &task_name,
                        "Bloom buy skipped: no default wallet configured".to_string(),
                    );
                    send_notification_markdown(chat_id, no_wallet_msg).await;
                }
            }
        }
    } else {
        let total_us = t_all_start.elapsed().as_micros();
        log::info!("perf.total_us={} ca_not_found=1", total_us);
        log_task_event(
            chat_id,
            &task_name,
            "No contract address detected in Telegram message".to_string(),
        );
    }
}

pub async fn start_task_monitor(initial_task: Task, chat_id: i64) {
    let handle = USER_CLIENT_HANDLE.lock().clone();
    if let Some(_client) = handle {
        if initial_task.listen_channels.is_empty() {
            log::warn!("task.tg: task has no channels name={}", initial_task.name);
            log_task_event(
                chat_id,
                &initial_task.name,
                "Task has no channels configured; cannot start monitor".to_string(),
            );
            return;
        }

        let task_name = initial_task.name.clone();
        let task_state = state::ensure_task_state(chat_id, initial_task).await;
        let user_data_state = state::get_user_data_state(chat_id);

        let session_id = uuid::Uuid::new_v4();
        {
            let mut sessions = crate::ACTIVE_TASK_SESSIONS.lock();
            sessions.insert((chat_id, task_name.clone()), session_id);
        }
        log::info!(
            "task.tg: registered session_id={} task={}",
            session_id,
            task_name
        );
        log_task_event(
            chat_id,
            &task_name,
            "Telegram task monitor started".to_string(),
        );

        tokio::spawn(async move {
            let sent_cas = Arc::new(Mutex::new(HashSet::<String>::new()));
            let mut rx = update_bus::subscribe();
            log::info!(
                "task.tg: worker start user_chat={} task={} session={}",
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
                        "task.tg: session invalidated stopping session={}",
                        session_id
                    );
                    log_task_event(
                        chat_id,
                        &task_name,
                        "Telegram task monitor stopped (session replaced)".to_string(),
                    );
                    break;
                }

                let update = match rx.recv().await {
                    Ok(u) => u,
                    Err(e) => {
                        log::error!("task.tg: bus recv error {}", e);
                        log_task_event(chat_id, &task_name, format!("Update bus error: {}", e));
                        break;
                    }
                };

                let task_snapshot = { task_state.read().await.clone() };

                if !task_snapshot.active {
                    log::info!("task.tg: task inactive stopping");
                    log_task_event(
                        chat_id,
                        &task_name,
                        "Task deactivated, stopping monitor".to_string(),
                    );
                    break;
                }

                let target_channel = match task_snapshot.listen_channels.first().copied() {
                    Some(id) => id,
                    None => {
                        log::warn!("task.tg: no channel configured task={}", task_snapshot.name);
                        log_task_event(
                            chat_id,
                            &task_name,
                            "No Telegram channel configured; stopping monitor".to_string(),
                        );
                        continue;
                    }
                };

                if let grammers_client::Update::NewMessage(msg) = &update.update {
                    if msg.chat().id() == target_channel {
                        let user_data_option = match user_data_state.as_ref() {
                            Some(state) => Some(state.read().await.clone()),
                            None => None,
                        };
                        let sent_cas_clone = Arc::clone(&sent_cas);
                        let arrival_ts = update.ts;
                        tokio::spawn(process_message(
                            msg.clone(),
                            task_snapshot.clone(),
                            chat_id,
                            user_data_option,
                            sent_cas_clone,
                            arrival_ts,
                        ));
                    }
                }
            }
        });
    } else {
        log::warn!("task.tg: user client not logged in");
        log_task_event(
            chat_id,
            &initial_task.name,
            "Telegram user client not logged in; task monitor not started".to_string(),
        );
    }
}
