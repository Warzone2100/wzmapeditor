//! Structured console for the editor's bottom-dock Output tab.
//!
//! Bounded ring buffer of entries, receives asynchronous entries from the
//! global logger over an `mpsc` channel, and renders a toolbar (severity
//! toggles, source filter, search, copy/clear) above a scrollable log view.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use web_time::{SystemTime, UNIX_EPOCH};

/// Bounds memory in long editing sessions; ~1 MB worst case at 5000 entries.
const MAX_ENTRIES: usize = 5000;

/// Cap drained channel entries per frame to keep a runaway logger thread
/// from stalling the UI.
const MAX_DRAIN_PER_FRAME: usize = 256;

/// Severity of a log entry. Ordered Info < Warn < Error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogSeverity {
    Info,
    Warn,
    Error,
}

impl LogSeverity {
    fn short_label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERR ",
        }
    }

    fn color(self) -> egui::Color32 {
        match self {
            Self::Info => egui::Color32::from_gray(190),
            Self::Warn => egui::Color32::from_rgb(230, 190, 60),
            Self::Error => egui::Color32::from_rgb(230, 90, 90),
        }
    }
}

/// Logical origin of a log entry, used for source-based filtering.
///
/// `Internal` covers warnings and errors captured automatically from the internal
/// workspace crates (`wzmapeditor`, `wz_maplib`, `wz_pie`, `wz_stats`) via the
/// custom `log::Log` implementation in [`crate::logger`]. All other variants
/// are produced explicitly by editor code via `EditorApp::log*` helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogSource {
    Editor,
    MapIo,
    Tool,
    Generator,
    TestGame,
    Internal,
}

impl LogSource {
    pub const ALL: [Self; 6] = [
        Self::Editor,
        Self::MapIo,
        Self::Tool,
        Self::Generator,
        Self::TestGame,
        Self::Internal,
    ];

    fn short_label(self) -> &'static str {
        match self {
            Self::Editor => "editor",
            Self::MapIo => "map",
            Self::Tool => "tool",
            Self::Generator => "gen",
            Self::TestGame => "test",
            Self::Internal => "internal",
        }
    }

    fn display_label(self) -> &'static str {
        match self {
            Self::Editor => "Editor",
            Self::MapIo => "Map I/O",
            Self::Tool => "Tools",
            Self::Generator => "Generator",
            Self::TestGame => "Test Game",
            Self::Internal => "Internal crates",
        }
    }
}

/// A single structured log entry rendered in the Output panel.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: SystemTime,
    pub severity: LogSeverity,
    pub source: LogSource,
    pub message: String,
}

impl LogEntry {
    pub fn new(severity: LogSeverity, source: LogSource, message: String) -> Self {
        Self {
            timestamp: SystemTime::now(),
            severity,
            source,
            message,
        }
    }
}

/// User-adjustable filter state controlling what is visible in the panel.
#[derive(Debug, Clone)]
pub struct OutputLogFilter {
    pub show_info: bool,
    pub show_warn: bool,
    pub show_error: bool,
    pub sources: [bool; LogSource::ALL.len()],
    pub search: String,
    pub show_timestamp_col: bool,
    pub show_source_col: bool,
}

impl Default for OutputLogFilter {
    fn default() -> Self {
        // Internal crates (captured log-crate output) is off by default:
        // the noise drowns out editor messages, and it's easy to turn
        // back on from the source-filter dropdown when debugging.
        let mut sources = [true; LogSource::ALL.len()];
        sources[source_index(LogSource::Internal)] = false;
        Self {
            show_info: true,
            show_warn: true,
            show_error: true,
            sources,
            search: String::new(),
            show_timestamp_col: true,
            show_source_col: false,
        }
    }
}

impl OutputLogFilter {
    fn severity_visible(&self, s: LogSeverity) -> bool {
        match s {
            LogSeverity::Info => self.show_info,
            LogSeverity::Warn => self.show_warn,
            LogSeverity::Error => self.show_error,
        }
    }

    fn source_visible(&self, s: LogSource) -> bool {
        self.sources[source_index(s)]
    }

    fn entry_visible(&self, entry: &LogEntry) -> bool {
        if !self.severity_visible(entry.severity) || !self.source_visible(entry.source) {
            return false;
        }
        if self.search.is_empty() {
            return true;
        }
        entry
            .message
            .to_lowercase()
            .contains(&self.search.to_lowercase())
    }
}

fn source_index(s: LogSource) -> usize {
    match s {
        LogSource::Editor => 0,
        LogSource::MapIo => 1,
        LogSource::Tool => 2,
        LogSource::Generator => 3,
        LogSource::TestGame => 4,
        LogSource::Internal => 5,
    }
}

/// The Output panel state: ring buffer, channel receiver, and filter.
///
/// Construct with [`OutputLog::new`], which returns the panel together with
/// an `mpsc::Sender<LogEntry>` to hand to the logger. Push editor-side
/// entries via [`OutputLog::push`]; drain logger-sourced entries each frame
/// via [`OutputLog::pump`].
#[derive(Debug)]
pub struct OutputLog {
    entries: VecDeque<LogEntry>,
    rx: Receiver<LogEntry>,
    pub filter: OutputLogFilter,
}

impl OutputLog {
    pub fn new() -> (Self, Sender<LogEntry>) {
        let (tx, rx) = mpsc::channel();
        let log = Self {
            entries: VecDeque::with_capacity(256),
            rx,
            filter: OutputLogFilter::default(),
        };
        (log, tx)
    }

    /// Number of entries currently held (pre-filter).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Push an entry produced directly by editor code.
    pub fn push(&mut self, entry: LogEntry) {
        self.entries.push_back(entry);
        while self.entries.len() > MAX_ENTRIES {
            self.entries.pop_front();
        }
    }

    /// Drain up to [`MAX_DRAIN_PER_FRAME`] pending entries from the logger channel.
    ///
    /// Call every frame so that warnings/errors captured by [`crate::logger`]
    /// reach the panel promptly even if the tab is not the active one.
    pub fn pump(&mut self) {
        for _ in 0..MAX_DRAIN_PER_FRAME {
            match self.rx.try_recv() {
                Ok(entry) => self.push(entry),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
    }

    /// Wipe the in-memory buffer. Does not affect the on-disk log file.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Render the panel into `ui` (toolbar + scrollable entry list).
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.pump();

        self.toolbar_ui(ui);
        ui.separator();
        self.entries_ui(ui);
    }

    fn toolbar_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            severity_chip(ui, "Info", LogSeverity::Info, &mut self.filter.show_info);
            severity_chip(ui, "Warn", LogSeverity::Warn, &mut self.filter.show_warn);
            severity_chip(ui, "Error", LogSeverity::Error, &mut self.filter.show_error);

            ui.separator();

            egui::ComboBox::from_id_salt("output_log_source_filter")
                .selected_text(source_filter_summary(&self.filter.sources))
                .show_ui(ui, |ui| {
                    for (idx, source) in LogSource::ALL.iter().enumerate() {
                        ui.checkbox(&mut self.filter.sources[idx], source.display_label());
                    }
                });

            ui.separator();

            ui.add(
                egui::TextEdit::singleline(&mut self.filter.search)
                    .hint_text("Search…")
                    .desired_width(160.0),
            );

            ui.separator();

            let copy_clicked = ui
                .button("Copy")
                .on_hover_text("Copy visible entries to clipboard")
                .clicked();
            let clear_clicked = ui
                .button("Clear")
                .on_hover_text("Clear the in-memory buffer (disk log is untouched)")
                .clicked();

            ui.menu_button("Columns", |ui| {
                ui.checkbox(&mut self.filter.show_timestamp_col, "Show timestamps");
                ui.checkbox(&mut self.filter.show_source_col, "Show source tag");
            });

            if copy_clicked {
                let text = self.visible_entries_as_text();
                ui.ctx().copy_text(text);
            }
            if clear_clicked {
                self.clear();
            }
        });
    }

    fn entries_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let monospace = egui::FontId::monospace(12.0);
                for entry in &self.entries {
                    if !self.filter.entry_visible(entry) {
                        continue;
                    }
                    ui.horizontal_wrapped(|ui| {
                        if self.filter.show_timestamp_col {
                            ui.colored_label(
                                egui::Color32::from_gray(140),
                                format_time_hms(entry.timestamp),
                            );
                        }
                        ui.colored_label(
                            entry.severity.color(),
                            egui::RichText::new(entry.severity.short_label())
                                .font(monospace.clone()),
                        );
                        if self.filter.show_source_col {
                            ui.colored_label(
                                egui::Color32::from_gray(150),
                                egui::RichText::new(format!("[{}]", entry.source.short_label()))
                                    .font(monospace.clone()),
                            );
                        }
                        ui.label(&entry.message);
                    });
                }
            });
    }

    fn visible_entries_as_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for entry in &self.entries {
            if !self.filter.entry_visible(entry) {
                continue;
            }
            let _ = writeln!(
                out,
                "{} {} [{}] {}",
                format_time_hms(entry.timestamp),
                entry.severity.short_label().trim(),
                entry.source.short_label(),
                entry.message,
            );
        }
        out
    }
}

fn severity_chip(ui: &mut egui::Ui, label: &str, severity: LogSeverity, on: &mut bool) {
    let text = egui::RichText::new(label).color(if *on {
        severity.color()
    } else {
        egui::Color32::from_gray(100)
    });
    if ui
        .selectable_label(*on, text)
        .on_hover_text(format!("Toggle {label} visibility"))
        .clicked()
    {
        *on = !*on;
    }
}

fn source_filter_summary(sources: &[bool]) -> String {
    let enabled: usize = sources.iter().filter(|v| **v).count();
    if enabled == sources.len() {
        "Sources: all".to_owned()
    } else if enabled == 0 {
        "Sources: none".to_owned()
    } else {
        format!("Sources: {enabled}/{}", sources.len())
    }
}

/// Format `t` as `HH:MM:SS` in UTC for compact in-panel display.
///
/// UTC is used because the editor has no timezone database dependency; the
/// panel is a developer console, not an end-user activity log.
fn format_time_hms(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let s = (secs % 86_400) as u32;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_drops_oldest_when_full() {
        let (mut log, _tx) = OutputLog::new();
        for i in 0..(MAX_ENTRIES + 50) {
            log.push(LogEntry::new(
                LogSeverity::Info,
                LogSource::Editor,
                format!("msg {i}"),
            ));
        }
        assert_eq!(log.len(), MAX_ENTRIES);
        assert!(log.entries.front().unwrap().message.starts_with("msg 50"));
        assert!(log.entries.back().unwrap().message.contains("5049"));
    }

    #[test]
    fn filter_hides_severity() {
        let (mut log, _tx) = OutputLog::new();
        log.push(LogEntry::new(
            LogSeverity::Info,
            LogSource::Editor,
            "hi".into(),
        ));
        log.push(LogEntry::new(
            LogSeverity::Warn,
            LogSource::Editor,
            "warn".into(),
        ));
        log.filter.show_info = false;
        let visible: Vec<_> = log
            .entries
            .iter()
            .filter(|e| log.filter.entry_visible(e))
            .collect();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].severity, LogSeverity::Warn);
    }

    #[test]
    fn filter_search_is_case_insensitive() {
        let (mut log, _tx) = OutputLog::new();
        log.push(LogEntry::new(
            LogSeverity::Info,
            LogSource::Editor,
            "Loaded MAP.wz".into(),
        ));
        log.push(LogEntry::new(
            LogSeverity::Info,
            LogSource::Editor,
            "saved config".into(),
        ));
        log.filter.search = "map".into();
        let visible: Vec<_> = log
            .entries
            .iter()
            .filter(|e| log.filter.entry_visible(e))
            .collect();
        assert_eq!(visible.len(), 1);
    }
}
