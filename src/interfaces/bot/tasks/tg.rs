use crate::BloomBuyAck;
use crate::UserClientHandle;
use crate::infrastructure::blockchain::bloom_buy;
use crate::interfaces::bot::core::update_bus;
use crate::interfaces::bot::escape_markdown;
use crate::interfaces::bot::tasks::{append_task_log, resolve_task_wallet, state};
use crate::interfaces::bot::{Task, UserData, log_buffer_to_ca_detection};
use crate::{PENDING_BLOOM_RESPONSES, USER_CLIENT_HANDLE};
use anyhow::{Result as AnyhowResult, anyhow};
use chrono::Local;
use grammers_client::types::Chat;
use grammers_client::{InvocationError, grammers_tl_types as tl};
use regex::Regex;
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
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

fn join_link_regex() -> &'static Regex {
    static JOIN_LINK_REGEX: OnceLock<Regex> = OnceLock::new();
    JOIN_LINK_REGEX.get_or_init(|| {
        Regex::new(
            r"https?://(?:www\.)?(?:t\.me|telegram\.me|telegram\.dog|tg\.dev|telesco\.pe)/(?:joinchat/|\+)([A-Za-z0-9_-]+)",
        )
        .expect("valid Telegram invite regex")
    })
}

fn extract_invite_hashes(text: &str) -> Vec<String> {
    join_link_regex()
        .captures_iter(text)
        .filter_map(|captures| captures.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

enum JoinResult {
    Joined(Option<Chat>),
    AlreadyJoined,
}

fn chat_from_updates(updates: tl::enums::Updates) -> Option<Chat> {
    match updates {
        tl::enums::Updates::Combined(data) => data.chats.into_iter().next(),
        tl::enums::Updates::Updates(data) => data.chats.into_iter().next(),
        _ => None,
    }
    .map(Chat::from_raw)
}

async fn try_join_invite(client: &UserClientHandle, hash: &str) -> AnyhowResult<JoinResult> {
    match client
        .invoke(&tl::functions::messages::ImportChatInvite {
            hash: hash.to_string(),
        })
        .await
    {
        Ok(updates) => Ok(JoinResult::Joined(chat_from_updates(updates))),
        Err(InvocationError::Rpc(rpc)) => {
            let name = rpc.name.as_str();
            match name {
                "USER_ALREADY_PARTICIPANT" | "ALREADY_PARTICIPANT" => Ok(JoinResult::AlreadyJoined),
                "INVITE_HASH_EXPIRED" => Err(anyhow!("Telegram invite expired (hash={})", hash)),
                "INVITE_HASH_INVALID" => Err(anyhow!("Telegram invite invalid (hash={})", hash)),
                other => Err(anyhow!(
                    "Telegram RPC error {} while joining (hash={})",
                    other,
                    hash
                )),
            }
        }
        Err(err) => Err(anyhow!(
            "Failed to import chat invite hash {}: {}",
            hash,
            err
        )),
    }
}

async fn handle_auto_join_links(chat_id: i64, task_name: &str, message_text: &str) {
    let hashes = extract_invite_hashes(message_text);
    if hashes.is_empty() {
        return;
    }

    let handle = USER_CLIENT_HANDLE.lock().clone();
    let Some(client) = handle else {
        log::warn!(
            "task.tg: auto-join skipped no user client chat_id={} task={}",
            chat_id,
            task_name
        );
        return;
    };

    let mut seen = HashSet::new();
    let mut joined_channels: Vec<(String, Option<i64>, String)> = Vec::new();
    let mut already_joined: Vec<String> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();

    for hash in hashes {
        if !seen.insert(hash.clone()) {
            continue;
        }

        match try_join_invite(&client, &hash).await {
            Ok(JoinResult::Joined(chat_opt)) => {
                let (channel_name, channel_id) = if let Some(chat) = chat_opt.as_ref() {
                    (
                        chat.name().unwrap_or("Unknown Channel").to_string(),
                        Some(chat.id()),
                    )
                } else {
                    ("Unknown Channel".to_string(), None)
                };
                log_task_event(
                    chat_id,
                    task_name,
                    format!(
                        "Joined Telegram channel via invite hash={} name={}",
                        hash, channel_name
                    ),
                );
                joined_channels.push((channel_name, channel_id, hash));
            }
            Ok(JoinResult::AlreadyJoined) => {
                log_task_event(
                    chat_id,
                    task_name,
                    format!("Invite hash={} already joined", hash),
                );
                already_joined.push(hash);
            }
            Err(err) => {
                let err_str = err.to_string();
                log::warn!(
                    "task.tg: auto-join failed hash={} chat_id={} err={}",
                    hash,
                    chat_id,
                    err_str
                );
                log_task_event(
                    chat_id,
                    task_name,
                    format!("Failed to join via invite hash={} err={}", hash, err_str),
                );
                failures.push((hash, err_str));
            }
        }
    }

    if joined_channels.is_empty() && already_joined.is_empty() && failures.is_empty() {
        return;
    }

    let mut sections = Vec::new();
    sections.push("🤖 *Auto-Join Update*".to_string());

    if !joined_channels.is_empty() {
        sections.push(String::new());
        sections.push("✅ *Joined Channels:*".to_string());
        for (name, id_opt, hash) in &joined_channels {
            let name_md = escape_markdown(name);
            let hash_md = escape_markdown(hash);
            let entry = if let Some(id) = id_opt {
                let id_md = escape_markdown(&id.to_string());
                format!("• {} (`hash:{}` · `id:{}`)", name_md, hash_md, id_md)
            } else {
                format!("• {} (`hash:{}`)", name_md, hash_md)
            };
            sections.push(entry);
        }
    }

    if !already_joined.is_empty() {
        sections.push(String::new());
        sections.push("ℹ️ *Already Joined:*".to_string());
        for hash in &already_joined {
            let hash_md = escape_markdown(hash);
            sections.push(format!("• `hash:{}`", hash_md));
        }
    }

    if !failures.is_empty() {
        sections.push(String::new());
        sections.push("⚠️ *Failed Invites:*".to_string());
        for (hash, err) in &failures {
            let hash_md = escape_markdown(hash);
            let err_md = escape_markdown(err);
            sections.push(format!("• `hash:{}` — {}", hash_md, err_md));
        }
    }

    let message = sections.join("\n");
    send_notification_markdown(chat_id, message).await;
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

    let task_name = task.name.clone();
    let t_blacklist = Instant::now();
    let should_listen =
        task.listen_users.is_empty() || task.listen_users.contains(&effective_sender_id);
    let message_text = msg.text();
    handle_auto_join_links(chat_id, &task_name, &message_text).await;
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
    let blacklist_ms = (blacklist_check_us as f64 / 1000.0).max(0.01);
    log::info!(
        "perf.blacklist_check_us={} hit={}",
        blacklist_check_us,
        if detected_words.is_empty() { 0 } else { 1 }
    );
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
