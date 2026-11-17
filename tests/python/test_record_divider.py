"""The pinned-record rule is the only record seam."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HORIZONTAL = 'linear-gradient(to right, transparent, hsl(var(--foreground) / .28) 25%, hsl(var(--foreground) / .28) 75%, transparent)'
VERTICAL = HORIZONTAL.replace('to right', 'to bottom')


def test_detail_metadata_and_pane_seams_match_the_pinned_divider():
    styles = (ROOT / 'src/styles.css').read_text()
    clipboard = (ROOT / 'src/clipboard/clipboard.css').read_text()
    native = (ROOT / 'src-tauri/src/windows.rs').read_text()
    assert HORIZONTAL in styles
    assert VERTICAL in styles
    assert 'top: 3%; width: 8px; height: 94%' in styles
    assert 'top: 3%; height: 94%; width: 1px' in styles
    assert 'hsl(var(--foreground) / .12) 50%' not in styles
    assert HORIZONTAL in clipboard
    assert 'left: 3%; width: 94%' in clipboard
    assert '[0.0_f64, 0.28, 0.28, 0.28, 0.0]' in native
    assert 'bounds.size.height * 0.03' in native
    assert 'bounds.size.height * 0.94' in native


def test_record_labels_share_the_theme_secondary_color():
    clipboard = (ROOT / 'src/clipboard/clipboard.css').read_text()
    heading = (ROOT / 'src/shared/RecordHeading.tsx').read_text()
    styles = (ROOT / 'src/styles.css').read_text()
    assert '.clipboard-panel .history-row .history-languages' not in clipboard
    assert 'brightLabel' not in heading
    assert '.history-row-heading' in styles and 'color: var(--history-secondary)' in styles
    assert "WebkitTextFillColor: '#fff'" not in heading


def test_translation_results_use_the_clipboard_detail_font():
    styles = (ROOT / 'src/styles.css').read_text()
    clipboard = (ROOT / 'src/clipboard/clipboard.css').read_text()
    family = "-apple-system, BlinkMacSystemFont, 'Helvetica Neue', sans-serif"
    result = next(block for block in styles.split('}') if '.result-column {' in block)
    detail = next(block for block in clipboard.split('}') if '.clipboard-detail-text {' in block)
    records = next(block for block in styles.split('}') if '.history-panel {' in block)
    assert 'font-size: 13px' in result and 'font-size: 13px' in detail
    assert 'line-height: 1.5' in result and 'line-height: 1.5' in detail
    assert f'font-family: {family}' in result
    assert f'font-family: {family}' in records


def test_detail_metadata_height_stays_three_rows():
    clipboard = (ROOT / 'src/clipboard/clipboard.css').read_text()
    assert 'height: calc(10px + 16px * 3 + 5px * 2)' in clipboard
    assert '.clipboard-detail-metadata { position: relative; flex: 0 0 auto;' in clipboard


def test_loading_glow_pulses_in_place_and_reduced_motion_stops_it():
    styles = (ROOT / 'src/styles.css').read_text()
    assert 'beam-breathe 1200ms ease-in-out infinite' in styles
    assert 'beam-travel' not in styles
    assert '--beam-distance' not in styles
    assert 'width: 100%' in next(block for block in styles.split('}') if '.activity-beam > span {' in block)
    breathe = styles.split('@keyframes beam-breathe')[1].split('@media')[0]
    assert 'opacity:' in breathe
    for property in ['transform:', 'left:', 'width:']:
        assert property not in breathe
    assert 'transition: opacity 160ms ease-out' in styles
    reduced = next(block for block in styles.split('@media (prefers-reduced-motion: reduce)') if '.activity-beam { transition: none; }' in block)
    assert '.activity-beam { transition: none; }' in reduced
    assert '.activity-beam > span { animation: none; opacity: .7; }' in reduced
