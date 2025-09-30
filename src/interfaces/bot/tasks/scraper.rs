use crate::application::filter;
use crate::application::indexer::threshold_detect_from_text;

fn log_step(buffer: &mut Vec<String>, message: String) {
    let timestamp = chrono::Utc::now().to_rfc3339();
    buffer.push(format!("[{}] {}", timestamp, message));
}

fn fast_clean_ascii(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == ' ' || ch == '\n' {
            out.push(ch);
        }
    }
    out
}

pub async fn find_mint_in_text(text: &str, log_buffer: &mut Vec<String>) -> Option<String> {
    log_step(
        log_buffer,
        format!("Original Text Received: \n---\n{}\n---", text),
    );

    let cleaned_text = fast_clean_ascii(text);
    log_step(
        log_buffer,
        format!(
            "Cleaned Text for Processing: '{}'",
            cleaned_text.replace('\n', "\\n")
        ),
    );

    let (filtered_text, removed_count, filter_us) =
        match filter::filter_text_and_measure(&cleaned_text) {
            Ok(result) => result,
            Err(e) => {
                log::error!("Word filter error: {}", e);
                (cleaned_text.clone(), 0, 0)
            }
        };
    log::info!(
        "perf.word_filter_us={} removed_words={}",
        filter_us,
        removed_count
    );
    log_step(
        log_buffer,
        format!(
            "After Word Filter: '{}'",
            filtered_text.replace('\n', "\\n")
        ),
    );

    let threshold = 1usize;
    let redis_url = match std::env::var("REDIS_URL") {
        Ok(v) => v,
        Err(_) => "".to_string(),
    };
    if !redis_url.is_empty() {
        match threshold_detect_from_text(&redis_url, &filtered_text, threshold).await {
            Ok(Some(hit)) => {
                log_step(
                    log_buffer,
                    format!("Shard Threshold Detect SUCCESS: '{}'", hit),
                );
                return Some(hit);
            }
            Ok(None) => {
                log_step(
                    log_buffer,
                    "Shard Threshold Detect FAILED, continuing.".to_string(),
                );
            }
            Err(e) => {
                log::error!("Threshold detect error: {}", e);
                log_step(log_buffer, format!("Shard Threshold Detect ERROR: {}", e));
            }
        }
    }

    for part in filtered_text.split_whitespace() {
        if part.len() >= 32 && part.len() <= 44 {
            return Some(part.to_string());
        }
    }

    None
}
