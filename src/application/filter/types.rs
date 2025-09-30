use std::collections::HashSet;
use std::sync::{OnceLock, RwLock};

pub(crate) static WORDS: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();

pub(crate) fn normalize_token(s: &str) -> String {
    s.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}
