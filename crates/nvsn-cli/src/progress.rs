//! Download progress indicator on stderr (indicatif, ADR-011).
//!
//! It is only drawn in a TTY without `--quiet`, `--no-progress`, `--json` or `NO_COLOR`.
//! Otherwise it is a no-op.
//!
//! [`ByteBar`] implements `nvsn_net::download::ProgressSink`.

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use nvsn_net::download::ProgressSink;
use std::borrow::Cow;

/// Byte bar for a download. If `enabled` is false it draws nothing.
#[derive(Debug)]
pub struct ByteBar {
    bar: ProgressBar,
}

impl ByteBar {
    /// Creates the bar with `message` if `enabled`; otherwise it is a no-op.
    pub fn new(enabled: bool, message: impl Into<Cow<'static, str>>) -> Self {
        if !enabled {
            return Self {
                bar: ProgressBar::with_draw_target(None, ProgressDrawTarget::hidden()),
            };
        }
        let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr());
        let style = ProgressStyle::with_template(
            "{msg} [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} {bytes_per_sec} ETA {eta}",
        )
        .unwrap_or_else(|_| ProgressStyle::default_bar());
        bar.set_style(style.progress_chars("=> "));
        bar.set_message(message);
        Self { bar }
    }

    /// Clears the bar from the screen.
    pub fn finish(self) {
        self.bar.finish_and_clear();
    }
}

impl ProgressSink for ByteBar {
    fn on_start(&self, total: u64) {
        // A total of 0 means "unknown": the bar only shows bytes.
        self.bar.set_length(total);
        self.bar.set_position(0);
    }

    fn on_bytes(&self, n: u64) {
        self.bar.inc(n);
    }
}

#[cfg(test)]
mod tests {
    use super::ByteBar;
    use nvsn_net::download::ProgressSink;

    #[test]
    fn disabled_bar_draws_nothing_and_accepts_updates() {
        let bar = ByteBar::new(false, "Downloading 20");
        assert!(bar.bar.is_hidden());
        bar.on_start(1024);
        bar.on_bytes(512);
        assert_eq!(bar.bar.position(), 512);
        bar.finish();
    }
}
