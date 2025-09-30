use super::types::{WORDS, normalize_token};
use std::time::Instant;

pub fn filter_text_and_measure(input: &str) -> Result<(String, usize, u128), &'static str> {
    let t0 = Instant::now();

    let words = WORDS
        .get()
        .ok_or("Word filter not initialized")?
        .read()
        .map_err(|_| "Failed to acquire read lock")?;

    if words.is_empty() {
        return Ok((input.to_string(), 0, t0.elapsed().as_micros()));
    }

    let mut removed_count = 0usize;
    let mut filtered_output = String::with_capacity(input.len());

    for token in input.split_whitespace() {
        let normalized = normalize_token(token);

        if normalized.is_empty() {
            continue;
        }

        if words.contains(&normalized) {
            removed_count += 1;
            continue;
        }

        if !filtered_output.is_empty() {
            filtered_output.push(' ');
        }
        filtered_output.push_str(token);
    }

    let elapsed_us = t0.elapsed().as_micros();
    Ok((filtered_output, removed_count, elapsed_us))
}
