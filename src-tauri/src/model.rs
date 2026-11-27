use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageDetection {
    pub raw_language: String,
    pub language: String,
    pub score: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationMetadata {
    pub requested_source: String,
    pub engine: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_engines: Vec<String>,
    pub strategy_version: u32,
    pub detections: Vec<LanguageDetection>,
}

#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationOutput {
    pub id: String,
    pub text: String,
    pub source_language: String,
    pub translation: TranslationMetadata,
    pub target_languages: Vec<String>,
    pub translations: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranslationScope {
    #[default]
    Standalone,
    Clipboard,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowPositionMode {
    #[default]
    LastPosition,
    Center,
    Mouse,
    Cursor,
}

impl WindowPositionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LastPosition => "lastPosition",
            Self::Center => "center",
            Self::Mouse => "mouse",
            Self::Cursor => "cursor",
        }
    }

    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("center") => Self::Center,
            Some("mouse") => Self::Mouse,
            Some("cursor") => Self::Cursor,
            _ => Self::LastPosition,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacementAnchor {
    pub visible_frame: ScreenRect,
    pub mouse: Option<ScreenPoint>,
    pub caret: Option<ScreenRect>,
}

/// Computes the target window origin (bottom-left in AppKit coordinates, logical points)
/// clamped within `anchor.visible_frame`. Returns `None` for `LastPosition`.
pub fn compute_window_origin(
    mode: WindowPositionMode,
    width: f64,
    height: f64,
    anchor: PlacementAnchor,
) -> Option<ScreenPoint> {
    const MOUSE_X_OFFSET: f64 = 24.0;
    const MOUSE_Y_OFFSET: f64 = 16.0;
    const CARET_GAP: f64 = 8.0;

    if mode == WindowPositionMode::LastPosition {
        return None;
    }
    let screen = anchor.visible_frame;
    if !width.is_finite()
        || !height.is_finite()
        || !screen.x.is_finite()
        || !screen.y.is_finite()
        || !screen.width.is_finite()
        || !screen.height.is_finite()
        || screen.width <= 0.0
        || screen.height <= 0.0
    {
        return None;
    }
    let clamped_w = width.max(0.0).min(screen.width);
    let clamped_h = height.max(0.0).min(screen.height);
    let max_x = (screen.x + screen.width - clamped_w).max(screen.x);
    let max_y = (screen.y + screen.height - clamped_h).max(screen.y);

    let clamp = |x: f64, y: f64| ScreenPoint {
        x: x.clamp(screen.x, max_x).round(),
        y: y.clamp(screen.y, max_y).round(),
    };
    let center = || {
        clamp(
            screen.x + (screen.width - clamped_w) / 2.0,
            screen.y + (screen.height - clamped_h) / 2.0,
        )
    };
    let from_mouse = |mouse: ScreenPoint| {
        clamp(
            mouse.x - MOUSE_X_OFFSET,
            mouse.y + MOUSE_Y_OFFSET - clamped_h,
        )
    };

    match mode {
        WindowPositionMode::LastPosition => None,
        WindowPositionMode::Center => Some(center()),
        WindowPositionMode::Mouse => Some(anchor.mouse.map(from_mouse).unwrap_or_else(center)),
        WindowPositionMode::Cursor => {
            if let Some(caret) = anchor.caret {
                let below_y = caret.y - CARET_GAP - clamped_h;
                let above_y = caret.y + caret.height + CARET_GAP;
                let space_below = caret.y - screen.y;
                let space_above = (screen.y + screen.height) - (caret.y + caret.height);
                let y = if below_y >= screen.y || space_below >= space_above {
                    below_y
                } else {
                    above_y
                };
                Some(clamp(caret.x, y))
            } else {
                Some(anchor.mouse.map(from_mouse).unwrap_or_else(center))
            }
        }
    }
}

pub const CLIPBOARD_MIN_WIDTH: f64 = 280.0;
pub const CLIPBOARD_MAX_WIDTH: f64 = 720.0;
pub const CLIPBOARD_DEFAULT_WIDTH: f64 = 360.0;
pub const CLIPBOARD_DETAIL_MIN_WIDTH: f64 = 360.0;
pub const CLIPBOARD_DETAIL_DEFAULT_WIDTH: f64 = 480.0;
pub const CLIPBOARD_TRANSLATION_MIN_WIDTH: f64 = 360.0;
pub const CLIPBOARD_TRANSLATION_DEFAULT_WIDTH: f64 = 440.0;
pub const STANDALONE_TRANSLATION_DEFAULT_WIDTH: f64 = 920.0;
pub const STANDALONE_TRANSLATION_SINGLE_MIN_WIDTH: f64 = 480.0;
pub const STANDALONE_TRANSLATION_DUAL_MIN_WIDTH: f64 = 700.0;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    pub records_browsing: bool,
    pub clipboard_detail_id: Option<String>,
    #[serde(skip)]
    pub preview_qr_revision: u64,
    pub clipboard_translation_record_id: Option<String>,
    pub records_editor_open: bool,
    pub records_session: u64,
    pub records_focus: u64,
    #[serde(skip)]
    pub records_open: bool,
    pub standalone_translation_visible: bool,
    pub clipboard_visible: bool,
    pub clipboard_detail_visible: bool,
    pub clipboard_translation_visible: bool,
    pub translation_pinned: bool,
    pub clipboard_pinned: bool,
    pub clipboard_width: f64,
    pub clipboard_detail_width: f64,
    pub clipboard_translation_width: f64,
    pub translation_width: f64,
    pub clipboard_extent: f64,
    pub right_extent: f64,
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
    pub clipboard_width_initialized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            records_browsing: true,
            clipboard_detail_id: None,
            preview_qr_revision: 0,
            clipboard_translation_record_id: None,
            records_editor_open: false,
            records_session: 0,
            records_focus: 0,
            records_open: false,
            standalone_translation_visible: false,
            clipboard_visible: false,
            clipboard_detail_visible: false,
            clipboard_translation_visible: false,
            translation_pinned: false,
            clipboard_pinned: false,
            clipboard_width: CLIPBOARD_DEFAULT_WIDTH,
            clipboard_detail_width: CLIPBOARD_DETAIL_DEFAULT_WIDTH,
            clipboard_translation_width: CLIPBOARD_TRANSLATION_DEFAULT_WIDTH,
            translation_width: STANDALONE_TRANSLATION_DEFAULT_WIDTH,
            clipboard_extent: 0.0,
            right_extent: 0.0,
            secondary_enabled: true,
            motion_revision: 0,
            motion_anchor: None,
            motion_velocity: PanelFrame::default(),
            native_resizing: false,
            clipboard_width_initialized: false,
        }
    }
}

impl WindowState {
    pub fn right_visible(&self) -> bool {
        self.standalone_translation_visible
            || (self.clipboard_visible
                && (self.clipboard_detail_visible || self.clipboard_translation_visible))
    }

    pub fn any_visible(&self) -> bool {
        self.standalone_translation_visible || self.clipboard_visible
    }

    pub fn right_target_width(&self) -> f64 {
        if self.standalone_translation_visible {
            self.translation_width
        } else if self.clipboard_visible {
            (if self.clipboard_detail_visible {
                self.clipboard_detail_width
            } else {
                0.0
            }) + (if self.clipboard_translation_visible {
                self.clipboard_translation_width
            } else {
                0.0
            })
        } else {
            0.0
        }
    }

    pub fn standalone_translation_min_width(&self) -> f64 {
        if self.secondary_enabled {
            STANDALONE_TRANSLATION_DUAL_MIN_WIDTH
        } else {
            STANDALONE_TRANSLATION_SINGLE_MIN_WIDTH
        }
    }

    pub fn right_min_width(&self) -> f64 {
        if self.standalone_translation_visible {
            self.standalone_translation_min_width()
        } else if self.clipboard_visible {
            (if self.clipboard_detail_visible {
                CLIPBOARD_DETAIL_MIN_WIDTH
            } else {
                0.0
            }) + (if self.clipboard_translation_visible {
                CLIPBOARD_TRANSLATION_MIN_WIDTH
            } else {
                0.0
            })
        } else {
            0.0
        }
    }

    pub fn summon_standalone_translation(&mut self) {
        self.records_browsing = false;
        self.clipboard_visible = false;
        self.set_clipboard_detail_visible(false);
        self.clipboard_translation_visible = false;
        self.select_clipboard_detail(None);
        self.clipboard_translation_record_id = None;
        self.standalone_translation_visible = true;
    }

    pub fn show_clipboard(&mut self) {
        let was_clipboard = self.clipboard_visible && !self.standalone_translation_visible;
        self.standalone_translation_visible = false;
        self.clipboard_visible = true;
        self.records_browsing = true;
        if !was_clipboard {
            self.set_clipboard_detail_visible(false);
            self.clipboard_translation_visible = false;
            self.select_clipboard_detail(None);
            self.clipboard_translation_record_id = None;
        }
    }

    pub fn invalidate_preview_qr(&mut self) -> u64 {
        self.preview_qr_revision = self.preview_qr_revision.wrapping_add(1);
        self.preview_qr_revision
    }

    pub fn select_clipboard_detail(&mut self, id: Option<String>) {
        if self.clipboard_detail_id != id {
            self.clipboard_detail_id = id;
            self.invalidate_preview_qr();
        }
    }

    fn set_clipboard_detail_visible(&mut self, visible: bool) {
        if self.clipboard_detail_visible != visible {
            self.clipboard_detail_visible = visible;
            self.invalidate_preview_qr();
        }
    }

    pub fn toggle_clipboard_details(&mut self) {
        if !self.clipboard_visible {
            return;
        }
        self.records_browsing = true;
        self.set_clipboard_detail_visible(!self.clipboard_detail_visible);
        if self.clipboard_detail_visible {
            self.clipboard_detail_width =
                self.clipboard_detail_width.max(CLIPBOARD_DETAIL_MIN_WIDTH);
        }
    }

    pub fn preview_clipboard_record(&mut self, record_id: String) {
        if !self.clipboard_visible {
            return;
        }
        if !self.clipboard_detail_visible || self.clipboard_detail_id.as_ref() == Some(&record_id) {
            self.toggle_clipboard_details();
        }
        self.records_browsing = true;
        self.select_clipboard_detail(Some(record_id));
    }

    pub fn open_clipboard_translation(&mut self, record_id: String) {
        self.standalone_translation_visible = false;
        self.clipboard_visible = true;
        self.clipboard_translation_visible = true;
        self.clipboard_translation_record_id = Some(record_id);
        self.clipboard_translation_width = self
            .clipboard_translation_width
            .max(CLIPBOARD_TRANSLATION_MIN_WIDTH);
    }

    pub fn close_clipboard_translation(&mut self) {
        self.clipboard_translation_visible = false;
        self.clipboard_translation_record_id = None;
    }

    pub fn clear_clipboard_translation_record(&mut self) -> bool {
        self.clipboard_translation_record_id.take().is_some()
    }

    pub fn sync_records_session(&mut self) -> bool {
        let open = self.clipboard_visible;
        let ended = self.records_open && !open;
        if ended {
            self.records_session += 1;
            self.records_browsing = true;
            self.records_editor_open = false;
            self.select_clipboard_detail(None);
            self.set_clipboard_detail_visible(false);
            self.clipboard_translation_visible = false;
            self.clipboard_translation_record_id = None;
        }
        self.records_open = open;
        ended
    }

    pub fn blur(&mut self) {
        if self.standalone_translation_visible {
            self.standalone_translation_visible &= self.translation_pinned;
        }
        if self.clipboard_visible && !self.clipboard_pinned {
            self.clipboard_visible = false;
            self.set_clipboard_detail_visible(false);
            self.clipboard_translation_visible = false;
            self.clipboard_translation_record_id = None;
        }
    }

    pub fn dismiss(&mut self) {
        self.standalone_translation_visible = false;
        self.clipboard_visible = false;
        self.set_clipboard_detail_visible(false);
        self.clipboard_translation_visible = false;
        self.clipboard_translation_record_id = None;
    }

    pub fn clamp_right_panes_to_available(&mut self, max_total_width: f64) {
        if !self.clipboard_visible || !max_total_width.is_finite() || max_total_width <= 0.0 {
            return;
        }
        let available_right = (max_total_width - self.clipboard_width).max(0.0);
        match (
            self.clipboard_detail_visible,
            self.clipboard_translation_visible,
        ) {
            (true, true) => {
                let total = self.clipboard_detail_width + self.clipboard_translation_width;
                if total > available_right {
                    let min_total = CLIPBOARD_DETAIL_MIN_WIDTH + CLIPBOARD_TRANSLATION_MIN_WIDTH;
                    if available_right <= min_total {
                        self.clipboard_detail_width = CLIPBOARD_DETAIL_MIN_WIDTH;
                        self.clipboard_translation_width = CLIPBOARD_TRANSLATION_MIN_WIDTH;
                    } else {
                        let extra = available_right - min_total;
                        let current_extra = (total - min_total).max(1.0);
                        let detail_extra =
                            ((self.clipboard_detail_width - CLIPBOARD_DETAIL_MIN_WIDTH).max(0.0)
                                / current_extra)
                                * extra;
                        let detail = (CLIPBOARD_DETAIL_MIN_WIDTH + detail_extra).round();
                        let translate = (available_right - detail)
                            .max(CLIPBOARD_TRANSLATION_MIN_WIDTH)
                            .round();
                        self.clipboard_detail_width = detail;
                        self.clipboard_translation_width = translate;
                    }
                }
            }
            (true, false) => {
                if self.clipboard_detail_width > available_right {
                    self.clipboard_detail_width =
                        available_right.max(CLIPBOARD_DETAIL_MIN_WIDTH).round();
                }
            }
            (false, true) => {
                if self.clipboard_translation_width > available_right {
                    self.clipboard_translation_width =
                        available_right.max(CLIPBOARD_TRANSLATION_MIN_WIDTH).round();
                }
            }
            (false, false) => {}
        }
    }

    pub fn resize_visible_width(&mut self, visible_width: f64) {
        if self.standalone_translation_visible {
            self.translation_width = visible_width.max(self.standalone_translation_min_width());
            self.right_extent = self.translation_width;
            return;
        }
        if !self.clipboard_visible {
            return;
        }
        if !self.clipboard_detail_visible && !self.clipboard_translation_visible {
            self.clipboard_width = visible_width.clamp(CLIPBOARD_MIN_WIDTH, CLIPBOARD_MAX_WIDTH);
            self.clipboard_extent = self.clipboard_width;
            return;
        }
        let right = (visible_width - self.clipboard_width).max(self.right_min_width());
        match (
            self.clipboard_detail_visible,
            self.clipboard_translation_visible,
        ) {
            (true, false) => {
                self.clipboard_detail_width = right.max(CLIPBOARD_DETAIL_MIN_WIDTH);
            }
            (false, true) => {
                self.clipboard_translation_width = right.max(CLIPBOARD_TRANSLATION_MIN_WIDTH);
            }
            (true, true) => {
                // Keep clipboard detail width stable unless total shrinks below its space;
                // rightmost attached translate pane absorbs outer resize first.
                let next_translate = right - self.clipboard_detail_width;
                if next_translate >= CLIPBOARD_TRANSLATION_MIN_WIDTH {
                    self.clipboard_translation_width = next_translate;
                } else {
                    self.clipboard_translation_width = CLIPBOARD_TRANSLATION_MIN_WIDTH;
                    self.clipboard_detail_width =
                        (right - CLIPBOARD_TRANSLATION_MIN_WIDTH).max(CLIPBOARD_DETAIL_MIN_WIDTH);
                }
            }
            (false, false) => {}
        }
        self.right_extent = self.right_target_width();
    }

    pub fn set_clipboard_detail_width(&mut self, width: f64) {
        if !width.is_finite() {
            return;
        }
        if self.clipboard_detail_visible && self.clipboard_translation_visible {
            let total = self.clipboard_detail_width + self.clipboard_translation_width;
            let max_detail =
                (total - CLIPBOARD_TRANSLATION_MIN_WIDTH).max(CLIPBOARD_DETAIL_MIN_WIDTH);
            let clamped = width.clamp(CLIPBOARD_DETAIL_MIN_WIDTH, max_detail).round();
            self.clipboard_detail_width = clamped;
            self.clipboard_translation_width = (total - clamped)
                .max(CLIPBOARD_TRANSLATION_MIN_WIDTH)
                .round();
        } else {
            self.clipboard_detail_width = width.max(CLIPBOARD_DETAIL_MIN_WIDTH).round();
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PanelFrame {
    pub left: f64,
    pub clipboard: f64,
    pub right: f64,
}

impl PanelFrame {
    pub fn spring(self, target: Self, velocity: Self, elapsed: f64) -> (Self, Self) {
        let sample = |start: f64, end: f64, speed: f64| {
            let delta = start - end;
            let decay = (-32.0 * elapsed).exp();
            let coefficient = speed + 32.0 * delta;
            (
                end + (delta + coefficient * elapsed) * decay,
                (speed - 32.0 * coefficient * elapsed) * decay,
            )
        };
        let (clipboard, clipboard_speed) =
            sample(self.clipboard, target.clipboard, velocity.clipboard);
        let duration = 0.28;
        let progress = (elapsed / duration).clamp(0.0, 1.0);
        let (right, right_speed) = if progress >= 1.0 {
            (target.right, 0.0)
        } else {
            let delta = target.right - self.right;
            let eased = progress * progress * (3.0 - 2.0 * progress);
            let carry = progress * (1.0 - progress).powi(2);
            (
                self.right + delta * eased + velocity.right * duration * carry,
                delta * 6.0 * progress * (1.0 - progress) / duration
                    + velocity.right * (1.0 - 4.0 * progress + 3.0 * progress * progress),
            )
        };
        let (anchor, anchor_speed) = sample(
            self.left + self.clipboard,
            target.left + target.clipboard,
            velocity.left + velocity.clipboard,
        );
        let frame = Self {
            left: anchor - clipboard,
            clipboard,
            right,
        };
        let speed = Self {
            left: anchor_speed - clipboard_speed,
            clipboard: clipboard_speed,
            right: right_speed,
        };
        if frame.settled(target, speed) {
            (target, Self::default())
        } else {
            (frame, speed)
        }
    }

    pub fn settled(self, target: Self, velocity: Self) -> bool {
        (self.left - target.left).abs() < 0.25
            && (self.clipboard - target.clipboard).abs() < 0.25
            && (self.right - target.right).abs() < 0.25
            && velocity.left.abs() < 8.0
            && velocity.clipboard.abs() < 8.0
            && velocity.right.abs() < 8.0
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
    use super::*;

    #[test]
    fn clipboard_details_and_attached_translation_toggle_independently() {
        let mut state = WindowState::default();
        state.show_clipboard();
        assert!(state.clipboard_visible);
        assert!(!state.clipboard_detail_visible);
        assert!(!state.clipboard_translation_visible);
        assert_eq!(state.right_target_width(), 0.0);

        // Open translation directly from list-only: [list | translate]
        state.open_clipboard_translation("rec-1".into());
        assert!(state.clipboard_visible);
        assert!(!state.clipboard_detail_visible);
        assert!(state.clipboard_translation_visible);
        assert_eq!(
            state.clipboard_translation_record_id.as_deref(),
            Some("rec-1")
        );
        assert_eq!(
            state.right_target_width(),
            CLIPBOARD_TRANSLATION_DEFAULT_WIDTH
        );

        // Open details while translate is open: [list | detail | translate]
        state.toggle_clipboard_details();
        assert!(state.clipboard_detail_visible && state.clipboard_translation_visible);
        assert_eq!(
            state.right_target_width(),
            CLIPBOARD_DETAIL_DEFAULT_WIDTH + CLIPBOARD_TRANSLATION_DEFAULT_WIDTH
        );

        // Collapse details while translate is open: [list | translate]
        state.toggle_clipboard_details();
        assert!(!state.clipboard_detail_visible && state.clipboard_translation_visible);
        assert_eq!(
            state.right_target_width(),
            CLIPBOARD_TRANSLATION_DEFAULT_WIDTH
        );

        // Reopen details then close translation: [list | detail]
        state.toggle_clipboard_details();
        state.close_clipboard_translation();
        assert!(state.clipboard_detail_visible && !state.clipboard_translation_visible);
        assert!(state.clipboard_translation_record_id.is_none());
        assert_eq!(state.right_target_width(), CLIPBOARD_DETAIL_DEFAULT_WIDTH);
    }

    #[test]
    fn standalone_translation_and_clipboard_have_independent_pins_and_blur() {
        let mut state = WindowState::default();
        state.show_clipboard();
        state.toggle_clipboard_details();
        state.open_clipboard_translation("rec-1".into());
        state.clipboard_pinned = true;
        state.translation_pinned = false;
        state.blur();
        assert!(
            state.clipboard_visible
                && state.clipboard_detail_visible
                && state.clipboard_translation_visible,
            "pinned clipboard keeps list, detail, and attached translation on blur"
        );

        state.clipboard_pinned = false;
        state.blur();
        assert!(!state.any_visible());
        assert!(state.clipboard_translation_record_id.is_none());

        state.summon_standalone_translation();
        state.translation_pinned = true;
        state.blur();
        assert!(state.standalone_translation_visible);

        state.translation_pinned = false;
        state.blur();
        assert!(!state.standalone_translation_visible);
    }

    #[test]
    fn keyboard_preview_toggles_the_same_item_and_selects_another_without_closing() {
        let mut state = WindowState::default();
        state.show_clipboard();
        state.preview_clipboard_record("first".into());
        assert!(state.clipboard_detail_visible && state.records_browsing);
        state.preview_clipboard_record("second".into());
        assert!(state.clipboard_detail_visible);
        assert_eq!(state.clipboard_detail_id.as_deref(), Some("second"));
        state.preview_clipboard_record("second".into());
        assert!(!state.clipboard_detail_visible);
    }

    #[test]
    fn clamp_right_panes_fits_three_panes_within_narrow_screen() {
        let mut state = WindowState::default();
        state.show_clipboard();
        state.toggle_clipboard_details();
        state.open_clipboard_translation("rec-1".into());
        state.clamp_right_panes_to_available(1200.0);
        assert!(
            state.clipboard_width + state.right_target_width() <= 1200.0 + 1.0,
            "total {} exceeds 1200",
            state.clipboard_width + state.right_target_width()
        );
        assert!(state.clipboard_detail_width >= CLIPBOARD_DETAIL_MIN_WIDTH);
        assert!(state.clipboard_translation_width >= CLIPBOARD_TRANSLATION_MIN_WIDTH);
    }

    #[test]
    fn session_ends_when_clipboard_closes_and_resets_attached_state() {
        let mut state = WindowState::default();
        state.show_clipboard();
        assert!(!state.sync_records_session());
        state.toggle_clipboard_details();
        state.clipboard_detail_id = Some("rec-1".into());
        state.open_clipboard_translation("rec-1".into());
        state.dismiss();
        assert!(state.sync_records_session());
        assert_eq!(state.records_session, 1);
        assert!(state.clipboard_detail_id.is_none());
        assert!(state.clipboard_translation_record_id.is_none());
        assert!(!state.clipboard_detail_visible);
        assert!(!state.clipboard_translation_visible);
    }

    #[test]
    fn details_open_and_close_have_mirrored_motion_and_finish_in_280ms() {
        for width in [440.0, 480.0, 920.0] {
            let closed = PanelFrame {
                left: 100.0,
                clipboard: 360.0,
                right: 0.0,
            };
            let open = PanelFrame {
                right: width,
                ..closed
            };
            for tick in 0..=28 {
                let elapsed = tick as f64 / 100.0;
                let opening = closed.spring(open, PanelFrame::default(), elapsed).0;
                let closing = open.spring(closed, PanelFrame::default(), 0.28 - elapsed).0;
                assert!(
                    (opening.right - closing.right).abs() < 1e-8,
                    "width={width}, elapsed={elapsed}: open={}, close={}",
                    opening.right,
                    closing.right
                );
            }
            assert_eq!(
                closed.spring(open, PanelFrame::default(), 0.28).0.right,
                width
            );
            assert_eq!(
                open.spring(closed, PanelFrame::default(), 0.28).0.right,
                0.0
            );
        }
    }

    #[test]
    fn window_position_center_mouse_cursor_and_last_position() {
        let screen = ScreenRect {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 900.0,
        };
        let anchor = PlacementAnchor {
            visible_frame: screen,
            mouse: Some(ScreenPoint { x: 500.0, y: 600.0 }),
            caret: Some(ScreenRect {
                x: 400.0,
                y: 700.0,
                width: 2.0,
                height: 20.0,
            }),
        };
        assert_eq!(
            compute_window_origin(WindowPositionMode::LastPosition, 400.0, 500.0, anchor),
            None
        );
        assert_eq!(
            compute_window_origin(WindowPositionMode::Center, 400.0, 500.0, anchor),
            Some(ScreenPoint { x: 520.0, y: 200.0 })
        );
        assert_eq!(
            compute_window_origin(WindowPositionMode::Mouse, 400.0, 500.0, anchor),
            Some(ScreenPoint { x: 476.0, y: 116.0 })
        );
        assert_eq!(
            compute_window_origin(WindowPositionMode::Cursor, 400.0, 500.0, anchor),
            Some(ScreenPoint { x: 400.0, y: 192.0 })
        );
    }
}
