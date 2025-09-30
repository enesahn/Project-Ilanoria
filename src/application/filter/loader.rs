use super::types::{WORDS, normalize_token};
use std::collections::HashSet;
use std::path::Path;
use std::sync::RwLock;
use std::time::Instant;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};

pub async fn init_word_filter() -> Result<usize, Box<dyn std::error::Error>> {
    let t0 = Instant::now();
    let dir = std::env::var("WORDS_DIR").unwrap_or_else(|_| "src/words".to_string());
    let path = Path::new(&dir);

    let mut word_set = HashSet::new();
    let mut file_count = 0usize;

    if path.is_dir() {
        let mut entries = fs::read_dir(path).await?;

        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();

            let is_txt_file = entry_path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("txt"))
                .unwrap_or(false);

            if is_txt_file {
                if let Ok(file) = fs::File::open(&entry_path).await {
                    file_count += 1;
                    let reader = BufReader::new(file);
                    let mut lines = reader.lines();

                    while let Some(line) = lines.next_line().await? {
                        let normalized = normalize_token(line.trim());
                        if !normalized.is_empty() {
                            word_set.insert(normalized);
                        }
                    }
                }
            }
        }
    }

    let dur_us = t0.elapsed().as_micros();
    let word_count = word_set.len();

    WORDS
        .set(RwLock::new(word_set))
        .map_err(|_| "Word filter already initialized")?;

    log::info!(
        "words.loaded={} files={} perf.load_us={}",
        word_count,
        file_count,
        dur_us
    );

    Ok(word_count)
}
