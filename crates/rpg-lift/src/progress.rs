//! Terminal progress display for autonomous lifting.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::Arc;

/// Manages progress bars for the lifting pipeline.
pub struct LiftProgress {
    multi: Arc<MultiProgress>,
    phase_bar: ProgressBar,
    cost_bar: ProgressBar,
}

impl Default for LiftProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl LiftProgress {
    pub fn new() -> Self {
        let multi = Arc::new(MultiProgress::new());

        let phase_bar = multi.add(ProgressBar::new(0));
        phase_bar.set_style(
            ProgressStyle::default_bar()
                .template("  {prefix:.bold} [{bar:30.cyan/blue}] {pos}/{len} {msg}")
                .expect("valid template")
                .progress_chars("##-"),
        );

        let cost_bar = multi.add(ProgressBar::new_spinner());
        cost_bar.set_style(
            ProgressStyle::default_spinner()
                .template("  {spinner:.green} {msg}")
                .expect("valid template"),
        );

        Self {
            multi,
            phase_bar,
            cost_bar,
        }
    }

    /// Start a new phase with a given name and total count.
    pub fn start_phase(&self, name: &str, total: u64) {
        self.phase_bar.set_prefix(name.to_string());
        self.phase_bar.set_length(total);
        self.phase_bar.set_position(0);
        self.phase_bar.set_message("");
    }

    /// Increment the phase progress by 1.
    pub fn tick_phase(&self) {
        self.phase_bar.inc(1);
    }

    /// Increment the phase progress by `n`.
    pub fn tick_phase_by(&self, n: u64) {
        self.phase_bar.inc(n);
    }

    /// Set a message on the phase bar.
    pub fn set_phase_message(&self, msg: &str) {
        self.phase_bar.set_message(msg.to_string());
    }

    /// Update the cost display.
    pub fn update_cost(&self, spent: f64, estimated_total: f64) {
        self.cost_bar.set_message(format!(
            "${:.4} spent (est. ${:.4} total)",
            spent, estimated_total
        ));
        self.cost_bar.tick();
    }

    /// Finish all bars.
    pub fn finish(&self) {
        self.phase_bar.finish_and_clear();
        self.cost_bar.finish_and_clear();
    }

    /// Suspend progress bars for clean eprintln output, then resume.
    pub fn suspend<F: FnOnce()>(&self, f: F) {
        self.multi.suspend(f);
    }
}
