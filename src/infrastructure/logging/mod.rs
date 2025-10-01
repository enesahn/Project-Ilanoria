use std::io::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};

use log::Level;
use pretty_env_logger::env_logger;

static STDOUT_SUPPRESSED: AtomicBool = AtomicBool::new(false);

pub fn init() {
    let mut builder = pretty_env_logger::formatted_timed_builder();
    builder.parse_default_env();

    builder.format(|buf, record| {
        if STDOUT_SUPPRESSED.load(Ordering::Acquire) {
            return Ok(());
        }

        let mut level_style = buf.style();
        level_style.set_color(match record.level() {
            Level::Error => env_logger::fmt::Color::Red,
            Level::Warn => env_logger::fmt::Color::Yellow,
            Level::Info => env_logger::fmt::Color::Green,
            Level::Debug => env_logger::fmt::Color::Blue,
            Level::Trace => env_logger::fmt::Color::Magenta,
        });
        level_style.set_bold(true);

        writeln!(
            buf,
            "{} {} {} > {}",
            buf.timestamp(),
            level_style.value(format!("{:<5}", record.level())),
            record.target(),
            record.args()
        )
    });

    builder.init();
}

pub fn suppress_stdout_logs() -> LogSuppressionGuard {
    let previous = STDOUT_SUPPRESSED.swap(true, Ordering::SeqCst);
    LogSuppressionGuard { previous }
}

pub struct LogSuppressionGuard {
    previous: bool,
}

impl Drop for LogSuppressionGuard {
    fn drop(&mut self) {
        STDOUT_SUPPRESSED.store(self.previous, Ordering::SeqCst);
    }
}
