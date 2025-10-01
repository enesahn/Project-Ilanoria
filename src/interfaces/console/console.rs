use colored::*;
use unicode_width::UnicodeWidthStr;

pub struct ConsoleUI;

impl ConsoleUI {
    pub fn clear_screen() {
        print!("\x1B[2J\x1B[1;1H");
        use std::io::{self, Write};
        io::stdout().flush().unwrap();
    }

    pub fn print_header(title: &str) {
        Self::clear_screen();
        let min_width = 78;
        let title_width = UnicodeWidthStr::width(title);
        let width = min_width.max(title_width);
        let padding_total = width.saturating_sub(title_width);
        let padding_left = padding_total / 2;
        let padding_right = padding_total - padding_left;
        let horizontal = "═".repeat(width);
        println!("{}", format!("╔{}╗", horizontal).bright_cyan());
        println!(
            "║{}{}{}║",
            " ".repeat(padding_left),
            title.bright_yellow().bold(),
            " ".repeat(padding_right)
        );
        println!("{}", format!("╚{}╝", horizontal).bright_cyan());
        println!();
    }

    pub fn print_option(number: i32, text: &str) {
        println!(
            "  {} {} {}",
            format!("[{}]", number).bright_blue().bold(),
            "»".bright_cyan(),
            text.bright_white()
        );
    }

    pub fn print_exit_option(key: char, text: &str) {
        println!(
            "  {} {} {}",
            format!("[{}]", key).bright_red().bold(),
            "»".bright_cyan(),
            text.truecolor(150, 150, 150)
        );
    }

    pub fn print_refresh_hint() {
        println!(
            "\n  {} {}",
            "↻".bright_green(),
            "Press [Enter] to refresh, [0] to go back".truecolor(150, 150, 150)
        );
    }

    pub fn print_prompt() {
        print!("\n  {} ", "❯".bright_green().bold());
        use std::io::{self, Write};
        io::stdout().flush().unwrap();
    }

    pub fn print_error(message: &str) {
        println!("\n  {} {}", "✗".bright_red().bold(), message.bright_red());
    }

    pub fn print_success(message: &str) {
        println!(
            "\n  {} {}",
            "✓".bright_green().bold(),
            message.bright_green()
        );
    }

    pub fn print_info(message: &str) {
        println!(
            "  {} {}",
            "[ℹ]".bright_cyan().bold(),
            message.truecolor(180, 180, 180)
        );
    }
}
