use serde::Serialize;

#[derive(Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    pub id: String,
    pub text: String,
    pub source_language: String,
    pub target_languages: Vec<String>,
    pub translations: Vec<String>,
    pub date: String,
    pub is_favorite: bool,
    pub is_pinned: bool,
    #[serde(default)]
    pub source_app: Option<serde_json::Value>,
}

pub fn initial_history_records() -> Vec<HistoryRecord> {
    #[cfg(feature = "ui-tests")]
    {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/translation-history.json"
        ))
        .expect("valid isolated UI test history")
    }
    #[cfg(not(feature = "ui-tests"))]
    Vec::new()
}

#[cfg(test)]
#[derive(serde::Deserialize)]
pub struct TranslationSample {
    pub id: String,
    pub text: String,
    pub translations: Vec<String>,
}

#[cfg(test)]
pub fn translation_sample_text(id: &str, part: &str) -> Option<String> {
    let samples: Vec<TranslationSample> = serde_json::from_str(include_str!(
        "../../tests/fixtures/translation-history.json"
    ))
    .ok()?;
    let record = samples.into_iter().find(|record| record.id == id)?;
    match part {
        "source" => Some(record.text),
        "primary" => record.translations.first().cloned(),
        "secondary" => record.translations.get(1).cloned(),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordView {
    #[default]
    Clipboard,
    Translation,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    pub record_view: RecordView,
    pub records_session: u64,
    pub records_focus: u64,
    #[serde(skip)]
    pub records_open: bool,
    pub translation_visible: bool,
    pub history_visible: bool,
    pub translation_pinned: bool,
    pub history_pinned: bool,
    pub clipboard_pinned: bool,
    pub history_width: f64,
    pub translation_width: f64,
    pub history_extent: f64,
    pub translation_extent: f64,
    pub secondary_enabled: bool,
    #[serde(skip)]
    pub motion_revision: u64,
    #[serde(skip)]
    pub motion_anchor: Option<f64>,
    #[serde(skip)]
    pub motion_velocity: PanelFrame,
    #[serde(skip)]
    pub native_resizing: bool,
    #[serde(skip)]
    pub history_width_initialized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            record_view: RecordView::Clipboard,
            records_session: 0,
            records_focus: 0,
            records_open: false,
            translation_visible: true,
            history_visible: false,
            translation_pinned: false,
            history_pinned: false,
            clipboard_pinned: false,
            history_width: 360.0,
            translation_width: 920.0,
            history_extent: 0.0,
            translation_extent: 920.0,
            secondary_enabled: true,
            motion_revision: 0,
            motion_anchor: None,
            motion_velocity: PanelFrame::default(),
            native_resizing: false,
            history_width_initialized: false,
        }
    }
}

impl WindowState {
    #[cfg(test)]
    pub fn region_frames(&self) -> ((f64, f64), (f64, f64)) {
        (
            (self.history_extent - self.history_width, self.history_width),
            (self.history_extent, self.translation_width),
        )
    }

    pub fn toggle_history(&mut self) {
        self.history_visible = self.record_view != RecordView::Translation || !self.history_visible;
        self.record_view = RecordView::Translation;
        self.translation_visible = true;
    }

    pub fn show_translation(&mut self) {
        self.translation_visible = true;
    }

    pub fn summon_translation(&mut self) {
        self.record_view = RecordView::Translation;
        self.history_visible = false;
        self.translation_visible = true;
    }

    pub fn set_record_view(&mut self, view: RecordView) {
        self.record_view = view;
        self.history_visible = true;
        if view == RecordView::Clipboard {
            self.translation_visible = false;
        }
    }

    pub fn select_history_record(&mut self, _repeated: bool, _retranslate: bool) {
        self.record_view = RecordView::Translation;
        self.history_visible = true;
        self.translation_visible = true;
    }

    pub fn sync_records_session(&mut self) -> bool {
        let open = self.translation_visible || self.history_visible;
        let ended = self.records_open && !open;
        if ended {
            self.records_session += 1;
        }
        self.records_open = open;
        ended
    }

    pub fn blur(&mut self) {
        self.translation_visible &= self.translation_pinned;
        self.history_visible &= self.history_pinned;
    }

    pub fn dismiss(&mut self) {
        self.translation_visible = false;
        self.history_visible = false;
    }

    pub fn translation_min_width(&self) -> f64 {
        if self.secondary_enabled {
            700.0
        } else {
            480.0
        }
    }

    pub fn resize_visible_width(&mut self, visible_width: f64) {
        if self.history_visible && !self.translation_visible {
            self.history_width = (visible_width
                - if self.translation_visible {
                    self.translation_width
                } else {
                    0.0
                })
            .clamp(280.0, 720.0);
            self.history_extent = self.history_width;
        } else if self.translation_visible {
            self.translation_width = (visible_width
                - if self.history_visible {
                    self.history_width
                } else {
                    0.0
                })
            .max(self.translation_min_width());
            self.translation_extent = self.translation_width;
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PanelFrame {
    pub left: f64,
    pub history: f64,
    pub translation: f64,
}

impl PanelFrame {
    pub fn spring(self, target: Self, velocity: Self, elapsed: f64) -> (Self, Self) {
        // The analytic critically damped solution preserves velocity when retargeted.
        let sample = |start: f64, end: f64, speed: f64| {
            let delta = start - end;
            let decay = (-32.0 * elapsed).exp();
            let coefficient = speed + 32.0 * delta;
            (
                end + (delta + coefficient * elapsed) * decay,
                (speed - 32.0 * coefficient * elapsed) * decay,
            )
        };
        let (history, history_speed) = sample(self.history, target.history, velocity.history);
        let (translation, translation_speed) =
            sample(self.translation, target.translation, velocity.translation);
        let (anchor, anchor_speed) = sample(
            self.left + self.history,
            target.left + target.history,
            velocity.left + velocity.history,
        );
        let frame = Self {
            left: anchor - history,
            history,
            translation,
        };
        let speed = Self {
            left: anchor_speed - history_speed,
            history: history_speed,
            translation: translation_speed,
        };
        if frame.settled(target, speed) {
            (target, Self::default())
        } else {
            (frame, speed)
        }
    }

    pub fn settled(self, target: Self, velocity: Self) -> bool {
        (self.left - target.left).abs() < 0.25
            && (self.history - target.history).abs() < 0.25
            && (self.translation - target.translation).abs() < 0.25
            && velocity.left.abs() < 8.0
            && velocity.history.abs() < 8.0
            && velocity.translation.abs() < 8.0
    }
}

pub fn sample(id: &str) -> Option<&'static str> {
    match id {
        "welcome" => Some("Hello from Nami — synthetic clipboard sample."),
        "multiline" => Some("第一行 / First line\n\n第三行 / Third line\n"),
        "unicode" => Some("中文、English、emoji 🌊 — 100%_literal"),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    pub status: &'static str,
    pub copy_status: &'static str,
    pub code: Option<&'static str>,
}

impl OperationResult {
    pub fn copied() -> Self {
        Self {
            status: "copied",
            copy_status: "current",
            code: None,
        }
    }

    pub fn failed(code: &'static str, version: Option<i64>, current: i64) -> Self {
        Self {
            status: "failed",
            copy_status: match version {
                None => "not_written",
                Some(value) if value == current => "current",
                Some(_) => "superseded",
            },
            code: Some(code),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn repeated_history_selection_keeps_details_open() {
        let mut state = super::WindowState::default();
        for _ in 0..20 {
            state.select_history_record(true, false);
            assert!(state.translation_visible && state.history_visible);
            assert_eq!(state.record_view, super::RecordView::Translation);
        }
    }

    #[test]
    fn record_modes_share_size_and_pin_but_clipboard_collapses_details() {
        let mut state = super::WindowState {
            history_width: 430.0,
            history_pinned: true,
            ..Default::default()
        };
        state.set_record_view(super::RecordView::Clipboard);
        assert!(state.history_visible && !state.translation_visible);
        state.set_record_view(super::RecordView::Translation);
        assert!(state.history_visible && !state.translation_visible);
        assert_eq!(state.history_width, 430.0);
        assert!(state.history_pinned);
        state.show_translation();
        state.toggle_history();
        assert!(!state.history_visible && state.translation_visible);
        state.set_record_view(super::RecordView::Clipboard);
        state.toggle_history();
        assert!(state.history_visible && state.translation_visible);
        assert_eq!(state.record_view, super::RecordView::Translation);
    }

    #[test]
    fn browsing_session_only_ends_when_both_regions_hide() {
        let mut state = super::WindowState::default();
        assert!(!state.sync_records_session());
        state.history_visible = true;
        state.sync_records_session();
        state.history_visible = false;
        assert!(!state.sync_records_session());
        assert_eq!(state.records_session, 0);
        state.dismiss();
        assert!(state.sync_records_session());
        assert_eq!(state.records_session, 1);
        assert!(!state.sync_records_session());
        state.summon_translation();
        assert!(!state.history_visible && state.translation_visible);
    }

    use super::*;

    #[cfg(not(feature = "ui-tests"))]
    #[test]
    fn production_history_starts_empty() {
        assert!(initial_history_records().is_empty());
    }

    #[test]
    fn history_records_keep_fixture_compatibility_and_source_application() {
        let mut records: Vec<HistoryRecord> = serde_json::from_str(include_str!(
            "../../tests/fixtures/translation-history.json"
        ))
        .unwrap();
        assert_eq!(records.len(), 20);
        assert!(records.iter().all(|record| record.source_app.is_none()));
        let record = &mut records[0];
        record.source_app = Some(serde_json::json!({
            "bundleId": "dev.local.test-source",
            "name": "Test Source",
            "iconDataUrl": "data:image/png;base64,dGVzdA=="
        }));
        let value = serde_json::to_value(&*record).unwrap();
        assert_eq!(value["sourceApp"]["name"], "Test Source");
        let restored: HistoryRecord = serde_json::from_value(value).unwrap();
        assert_eq!(restored.source_app, record.source_app);
        assert_eq!(restored.translations, record.translations);
    }

    #[test]
    fn translation_samples_share_exact_text_and_reject_arbitrary_copy() {
        let samples: Vec<TranslationSample> = serde_json::from_str(include_str!(
            "../../tests/fixtures/translation-history.json"
        ))
        .unwrap();
        assert_eq!(samples.len(), 20);
        for sample in &samples {
            assert_eq!(
                translation_sample_text(&sample.id, "source"),
                Some(sample.text.clone())
            );
        }
        assert!(translation_sample_text("sample-06", "source")
            .unwrap()
            .ends_with('\n'));
        assert!(translation_sample_text("sample-07", "secondary").is_none());
        assert!(translation_sample_text("arbitrary personal text", "source").is_none());
        assert!(translation_sample_text("sample-01", "invalid_part").is_none());
    }

    #[test]
    fn h04_blur_retains_only_visible_pinned_regions() {
        for translation_pinned in [false, true] {
            for history_pinned in [false, true] {
                let mut state = WindowState {
                    translation_visible: true,
                    history_visible: true,
                    translation_pinned,
                    history_pinned,
                    ..WindowState::default()
                };
                state.blur();
                assert_eq!(state.translation_visible, translation_pinned);
                assert_eq!(state.history_visible, history_pinned);
                state.history_visible = false;
                state.blur();
                assert!(!state.history_visible, "pin must not reopen hidden history");
            }
        }
    }

    #[test]
    fn h02_twenty_reverse_toggles_do_not_accumulate_width() {
        let anchor = 580.0;
        let mut frame = PanelFrame {
            left: anchor,
            history: 0.0,
            translation: 920.0,
        };
        for _ in 0..20 {
            let expanded = PanelFrame {
                left: anchor - 340.0,
                history: 340.0,
                translation: 920.0,
            };
            let (opening, velocity) = frame.spring(expanded, PanelFrame::default(), 0.1);
            frame = opening;
            assert!((frame.left + frame.history - anchor).abs() < 1e-9);
            let collapsed = PanelFrame {
                left: anchor,
                history: 0.0,
                translation: 920.0,
            };
            let (reversed, reversed_velocity) = frame.spring(collapsed, velocity, 0.0);
            assert_eq!(reversed.left, frame.left);
            assert_eq!(reversed.history, frame.history);
            assert!((reversed_velocity.history - velocity.history).abs() < 1e-9);
            frame = frame.spring(collapsed, velocity, 0.5).0;
            assert_eq!(frame.left, anchor);
            assert_eq!(frame.history, 0.0);
            assert_eq!(frame.translation, 920.0);
        }
    }

    #[test]
    fn history_reversal_preserves_speed_and_is_independent_of_tick_spacing() {
        let closed = PanelFrame {
            left: 570.0,
            history: 0.0,
            translation: 920.0,
        };
        let open = PanelFrame {
            left: 129.0,
            history: 441.0,
            translation: 920.0,
        };
        let (frame, velocity) = closed.spring(open, PanelFrame::default(), 0.1);
        let delta_time = 0.000_000_01;
        let (reversed, speed) = frame.spring(closed, velocity, delta_time);
        assert!(((reversed.history - frame.history) / delta_time - velocity.history).abs() < 0.01);
        assert!((speed.history - velocity.history).abs() < 0.01);
        let direct = closed.spring(open, PanelFrame::default(), 0.15);
        let delayed_tick = frame.spring(open, velocity, 0.05);
        assert!((direct.0.history - delayed_tick.0.history).abs() < 1e-9);
        assert!((direct.1.history - delayed_tick.1.history).abs() < 1e-9);
        assert_eq!(frame.spring(closed, velocity, 0.5).0.history, 0.0);
        assert_eq!(
            closed.spring(open, PanelFrame::default(), 0.5).0.history,
            441.0
        );
    }

    #[test]
    fn rapid_history_toggles_use_current_backend_state() {
        let mut state = WindowState {
            history_pinned: true,
            ..WindowState::default()
        };
        for _ in 0..20 {
            state.toggle_history();
        }
        assert!(!state.history_visible);
        assert!(state.translation_visible);
        assert!(state.history_pinned);
        state.toggle_history();
        assert!(state.history_visible);
    }

    #[test]
    fn sibling_views_preserve_translation_position_and_layout_during_history_animation() {
        let history_width = 441.0;
        let translation_width = 920.0;
        let mut start = PanelFrame {
            left: 570.0,
            history: 0.0,
            translation: translation_width,
        };
        for _ in 0..20 {
            let open = PanelFrame {
                left: 570.0 - history_width,
                history: history_width,
                translation: translation_width,
            };
            for tick in 0..=16 {
                let frame = start
                    .spring(open, PanelFrame::default(), tick as f64 * 0.35 / 16.0)
                    .0;
                let state = WindowState {
                    history_width,
                    history_extent: frame.history,
                    translation_width,
                    ..WindowState::default()
                };
                let ((history_x, history_view_width), (translation_x, translation_view_width)) =
                    state.region_frames();
                assert!((frame.left + translation_x - 570.0).abs() < 1e-9);
                assert!((history_x + history_view_width - translation_x).abs() < 1e-9);
                assert_eq!(translation_view_width, translation_width);
                assert_eq!(history_view_width, history_width);
            }
            let (frame, velocity) = start.spring(open, PanelFrame::default(), 0.1);
            start = frame;
            let closed = PanelFrame {
                left: 570.0,
                history: 0.0,
                translation: translation_width,
            };
            start = start.spring(closed, velocity, 0.5).0;
            assert_eq!(start.left, 570.0);
        }
    }

    #[test]
    fn divider_width_changes_use_sibling_views_without_dom_compensation() {
        for history_width in [280.0, 373.0, 441.0, 720.0, 360.0] {
            let state = WindowState {
                history_visible: true,
                history_width,
                history_extent: history_width,
                ..WindowState::default()
            };
            let ((history_x, history_view_width), (translation_x, translation_view_width)) =
                state.region_frames();
            assert_eq!(history_x, 0.0);
            assert_eq!(history_view_width, history_width);
            assert_eq!(translation_x, history_width);
            assert_eq!(translation_view_width, 920.0);
        }
    }

    #[test]
    fn native_window_notifications_keep_translation_width_across_history_toggles() {
        let mut state = WindowState {
            history_width: 441.0,
            ..WindowState::default()
        };
        for _ in 0..20 {
            state.toggle_history();
            state.resize_visible_width(if state.history_visible { 1361.0 } else { 920.0 });
            assert_eq!(state.translation_width, 920.0);
            assert_eq!(state.history_width, 441.0);
        }
        state.toggle_history();
        state.resize_visible_width(1441.0);
        assert_eq!(state.translation_width, 1000.0);
        state.translation_visible = false;
        state.resize_visible_width(500.0);
        assert_eq!(state.history_width, 500.0);
        assert_eq!(state.translation_width, 1000.0);
    }

    #[test]
    fn native_outer_edges_match_finderx_and_resize_translation_when_visible() {
        let mut state = WindowState {
            history_visible: true,
            history_width: 441.0,
            history_extent: 441.0,
            native_resizing: true,
            ..WindowState::default()
        };
        for visible in [1441.0, 1141.0, 1720.0, 1361.0] {
            state.resize_visible_width(visible);
            assert_eq!(state.history_width, 441.0);
            assert_eq!(state.translation_width, visible - 441.0);
        }
        state.translation_visible = false;
        for (requested, expected) in [(100.0, 280.0), (500.0, 500.0), (900.0, 720.0)] {
            state.resize_visible_width(requested);
            assert_eq!(state.history_width, expected);
            assert_eq!(state.translation_width, 920.0);
        }
    }

    #[test]
    fn h03_history_only_returns_to_translation_preserving_history() {
        let mut state = WindowState {
            translation_visible: false,
            history_visible: true,
            history_pinned: true,
            ..WindowState::default()
        };
        state.show_translation();
        assert!(state.translation_visible);
        assert!(state.history_visible);
        assert!(state.history_pinned);
    }

    #[test]
    fn explicit_dismiss_hides_both_even_when_pinned_and_preserves_sizes() {
        for pinned in [false, true] {
            let mut state = WindowState {
                history_visible: true,
                history_pinned: pinned,
                ..WindowState::default()
            };
            state.dismiss();
            assert!(!state.translation_visible);
            assert!(!state.history_visible);
            assert_eq!(state.translation_min_width(), 700.0);
            state.secondary_enabled = false;
            assert_eq!(state.translation_min_width(), 480.0);
        }
    }

    #[test]
    fn c09_copy_status_never_suggests_paste_before_write_or_after_overwrite() {
        assert_eq!(
            OperationResult::failed("resource_unavailable", None, 4).copy_status,
            "not_written"
        );
        assert_eq!(
            OperationResult::failed("permission_required", Some(4), 4).copy_status,
            "current"
        );
        assert_eq!(
            OperationResult::failed("clipboard_changed", Some(4), 5).copy_status,
            "superseded"
        );
        assert!(sample("arbitrary user text").is_none());
        assert!(sample("multiline").unwrap().ends_with('\n'));
    }
}
