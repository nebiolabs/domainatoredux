//! `--progress`: upstream's tqdm bars and `matrix_report`'s stage messages.
//!
//! A [`Progress`] bar is drawn only when it was asked for, messages are not silenced
//! (`-q`), and stderr is a terminal. A bar redrawn into a log file or the desktop
//! app's job log is noise, and the app's worker writes to a pipe, so it never draws.
//! [`Stages`] are plain lines, written wherever stderr goes, as upstream writes them
//! (but not under `-q`).

use std::io::IsTerminal;
use std::time::Instant;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};

/// A count of finished work items, drawn as a bar on stderr, or nothing. Cheap to
/// share across rayon workers: [`Progress::inc`] takes `&self`.
pub struct Progress(Option<ProgressBar>);

impl Progress {
    /// A bar of `total` items labeled `desc`, if `enabled` and stderr can show it.
    pub fn new(enabled: bool, total: usize, desc: &str) -> Progress {
        if !enabled
            || !crate::log::enabled(crate::log::Level::Info)
            || !std::io::stderr().is_terminal()
        {
            return Progress(None);
        }
        let bar = ProgressBar::with_draw_target(Some(total as u64), ProgressDrawTarget::stderr());
        // tqdm's layout: `desc:  45%|████▌     | 45/100 [00:01<00:01, 30.00it/s]`.
        let style = ProgressStyle::with_template(
            "{msg}: {percent:>3}%|{wide_bar}| {pos}/{len} [{elapsed_precise}<{eta_precise}, {rate}]",
        )
        .expect("a valid progress template")
        .with_key("rate", |state: &ProgressState, w: &mut dyn std::fmt::Write| {
            let _ = write!(w, "{:.2}it/s", state.per_sec());
        })
        .progress_chars("█▉▊▋▌▍▎▏ ");
        bar.set_style(style);
        bar.set_message(desc.to_string());
        Progress(Some(bar))
    }

    /// A count of items with no known total, as tqdm draws an iterable without a
    /// length: `desc: 1234it [00:01, 30.00it/s]`.
    pub fn counter(enabled: bool, desc: &str) -> Progress {
        if !enabled
            || !crate::log::enabled(crate::log::Level::Info)
            || !std::io::stderr().is_terminal()
        {
            return Progress(None);
        }
        let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr());
        let style = ProgressStyle::with_template("{msg}: {pos}it [{elapsed_precise}, {rate}]")
            .expect("a valid progress template")
            .with_key(
                "rate",
                |state: &ProgressState, w: &mut dyn std::fmt::Write| {
                    let _ = write!(w, "{:.2}it/s", state.per_sec());
                },
            );
        bar.set_style(style);
        bar.set_message(desc.to_string());
        Progress(Some(bar))
    }

    /// No bar.
    pub fn hidden() -> Progress {
        Progress(None)
    }

    /// `n` more items are done.
    pub fn inc(&self, n: u64) {
        if let Some(bar) = &self.0 {
            bar.inc(n);
        }
    }

    /// Items up to `pos` are done; never moves the bar back.
    pub fn advance_to(&self, pos: u64) {
        if let Some(bar) = &self.0 {
            if pos > bar.position() {
                bar.set_position(pos);
            }
        }
    }
}

impl Drop for Progress {
    /// Leave the finished bar on screen, as tqdm's `leave=True` does.
    fn drop(&mut self) {
        if let Some(bar) = &self.0 {
            bar.abandon();
            // indicatif leaves the cursor at the end of the bar's line; end it, so the
            // next message starts on its own.
            let _ = std::io::Write::write_all(&mut std::io::stderr(), b"\n");
        }
    }
}

/// One finished stage: its name, seconds, and the metrics reported beside them.
struct StageTiming {
    name: String,
    seconds: f64,
    metrics: Vec<(&'static str, String)>,
}

/// `matrix_report --progress`/`--profile_stages`: a line as each stage starts, and the
/// stages' timings at the end (upstream `_make_progress_callback`,
/// `_record_stage_timing` and `_emit_stage_timings`).
pub struct Stages {
    tool: &'static str,
    start: Instant,
    progress: bool,
    profile: bool,
    timings: Vec<StageTiming>,
}

impl Stages {
    /// Print stage lines if `progress` or `profile`; keep timings if `profile`.
    pub fn new(tool: &'static str, progress: bool, profile: bool) -> Stages {
        Stages {
            tool,
            start: Instant::now(),
            progress: progress || profile,
            profile,
            timings: Vec::new(),
        }
    }

    /// Neither messages nor timings.
    pub fn silent() -> Stages {
        Stages::new("", false, false)
    }

    /// A stage is starting: `[matrix_report +1.2s] building edge table`. Returns its
    /// start, for [`Stages::record`].
    pub fn begin(&self, message: &str) -> Instant {
        if self.progress {
            crate::log::info(format_args!(
                "[{} +{:.1}s] {message}",
                self.tool,
                self.start.elapsed().as_secs_f64()
            ));
        }
        Instant::now()
    }

    /// The stage `name`, started at `since`, is done, with `metrics` to report
    /// (computed only when profiling).
    pub fn record(
        &mut self,
        name: &str,
        since: Instant,
        metrics: impl FnOnce() -> Vec<(&'static str, String)>,
    ) {
        if self.profile {
            let elapsed = since.elapsed().as_secs_f64();
            self.timings.push(StageTiming {
                name: name.to_string(),
                seconds: elapsed,
                metrics: metrics(),
            });
        }
    }

    /// Print the recorded timings, upstream's block:
    /// `  build_edge_table: 0.012s (n_edges=190, n_nodes=20)`.
    pub fn emit_timings(&self) {
        if !self.profile {
            return;
        }
        let mut text = format!("{} stage timings:\n", self.tool);
        for StageTiming {
            name,
            seconds,
            metrics,
        } in &self.timings
        {
            text.push_str(&format!("  {name}: {seconds:.3}s"));
            if !metrics.is_empty() {
                let list: Vec<String> = metrics.iter().map(|(k, v)| format!("{k}={v}")).collect();
                text.push_str(&format!(" ({})", list.join(", ")));
            }
            text.push('\n');
        }
        crate::log::info(text.trim_end());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests run with stderr captured, not a terminal, so a requested bar is not drawn,
    /// and the calls are no-ops rather than errors.
    #[test]
    fn a_bar_is_not_drawn_off_a_terminal() {
        let bar = Progress::new(true, 10, "queries");
        if !std::io::stderr().is_terminal() {
            assert!(bar.0.is_none());
        }
        bar.inc(3);
        bar.advance_to(2);
        assert!(Progress::new(false, 10, "queries").0.is_none());
    }

    #[test]
    fn stage_timings_are_kept_only_when_profiling() {
        let mut stages = Stages::new("matrix_report", false, true);
        let t = stages.begin("building edge table");
        stages.record("build_edge_table", t, || vec![("n_edges", "3".into())]);
        assert_eq!(stages.timings.len(), 1);
        assert_eq!(stages.timings[0].metrics, [("n_edges", "3".to_string())]);

        let mut quiet = Stages::new("matrix_report", true, false);
        let t = quiet.begin("building edge table");
        quiet.record("build_edge_table", t, || panic!("metrics are not computed"));
        assert!(quiet.timings.is_empty());
    }
}
