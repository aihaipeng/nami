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
