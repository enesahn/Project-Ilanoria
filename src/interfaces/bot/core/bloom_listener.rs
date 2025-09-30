use crate::PENDING_BLOOM_INFO;
use crate::interfaces::bot::update_bus;
use crate::interfaces::bot::user::client::UserClientHandle;
use grammers_client::Update;
use grammers_tl_types::enums::MessageEntity;
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    static ref MINT_REGEX: Regex = Regex::new(r"([1-9A-HJ-NP-Za-km-z]{32,44})").unwrap();
    static ref TOKEN_LINE_RE: Regex = Regex::new(r"(?m)^\s*🔹\s*Token:\s*(.+?)\s*$").unwrap();
    static ref SIG_RE: Regex = Regex::new(r"(?m)^[1-9A-HJ-NP-Za-km-z]{60,}$").unwrap();
    static ref SOLSCAN_TX_URL_RE: Regex =
        Regex::new(r"https?://(?:www\.)?solscan\.io/tx/([A-Za-z0-9]+)").unwrap();
}

fn reconstruct_text_with_markdown(text: &str, entities: Option<&Vec<MessageEntity>>) -> String {
    if entities.is_none() {
        return text.to_string();
    }
    let mut entities = entities.unwrap().clone();
    entities.sort_by_key(|e| e.offset());
    let mut result = String::new();
    let mut last_offset = 0;
    let text_utf16: Vec<u16> = text.encode_utf16().collect();
    for entity in entities {
        let offset = entity.offset() as usize;
        let length = entity.length() as usize;
        if offset > last_offset {
            result.push_str(&String::from_utf16_lossy(&text_utf16[last_offset..offset]));
        }
        let entity_text = String::from_utf16_lossy(&text_utf16[offset..offset + length]);
        match entity {
            MessageEntity::TextUrl(data) => {
                result.push_str(&format!("[{}]({})", entity_text, data.url));
            }
            _ => {
                result.push_str(&entity_text);
            }
        }
        last_offset = offset + length;
    }
    if last_offset < text_utf16.len() {
        result.push_str(&String::from_utf16_lossy(&text_utf16[last_offset..]));
    }
    result
}

pub async fn run_bloom_listener(client: UserClientHandle) {
    log::info!("bloom_listener: starting");
    let bloom_bot = match client.resolve_username("BloomSolana_bot").await {
        Ok(Some(bot)) => bot,
        _ => {
            log::error!("bloom_listener: resolve username failed");
            return;
        }
    };
    let bloom_bot_id = bloom_bot.id();
    log::info!("bloom_listener: resolved id={}", bloom_bot_id);
    let mut rx = update_bus::subscribe();

    loop {
        match rx.recv().await {
            Ok(u) => {
                if let Update::NewMessage(message) = &u.update {
                    if message.chat().id() != bloom_bot_id {
                        continue;
                    }
                    let raw_text = message.text();
                    let reconstructed =
                        reconstruct_text_with_markdown(raw_text, message.fmt_entities());
                    log::info!(
                        "bloom_listener: incoming message len={} text_preview={}",
                        raw_text.len(),
                        raw_text.lines().next().unwrap_or("")
                    );
                    let mut mint_opt = MINT_REGEX
                        .captures(raw_text)
                        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()));
                    if mint_opt.is_none() {
                        mint_opt = MINT_REGEX
                            .captures(&reconstructed)
                            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()));
                    }
                    if mint_opt.is_none() {
                        log::warn!("bloom_listener: mint not found in message");
                        continue;
                    }
                    let mint = mint_opt.unwrap();
                    log::info!("bloom_listener: extracted mint={}", mint);

                    if raw_text.contains("🟢 Spot Buy Success") {
                        let token_name = TOKEN_LINE_RE
                            .captures(raw_text)
                            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()));
                        let signature =
                            if let Some(cap) = SOLSCAN_TX_URL_RE.captures(&reconstructed) {
                                cap.get(1).map(|m| m.as_str().to_string())
                            } else {
                                SIG_RE.find(&reconstructed).map(|m| m.as_str().to_string())
                            };
                        log::info!(
                            "bloom_listener: success token_name={:?} signature_len={}",
                            token_name,
                            signature.as_ref().map(|s| s.len()).unwrap_or(0)
                        );
                    }

                    if raw_text.contains("Market Cap:") && raw_text.contains("Price:") {
                        let mut info_waiters = PENDING_BLOOM_INFO.lock();
                        if let Some(sender) = info_waiters.remove(&mint) {
                            let _ = sender.send(reconstructed.clone());
                            log::info!("bloom_listener: info delivered mint={}", mint);
                        }
                    }
                }
            }
            Err(e) => {
                log::error!("bloom_listener: bus error {}", e);
                break;
            }
        }
    }
}
