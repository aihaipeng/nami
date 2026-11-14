from pathlib import Path
import struct
import subprocess
import sys

import pytest

pytestmark = [pytest.mark.tools, pytest.mark.skipif(sys.platform != 'darwin', reason='macOS AppKit required')]
ROOT = Path(__file__).resolve().parents[2]
CARGO = ['cargo', 'run', '--manifest-path', str(ROOT / 'src-tauri/Cargo.toml')]


def test_selection_fixture_payloads_without_clipboard_access():
    subprocess.run(CARGO + ['--features', 'selection-fixture', '--bin', 'selection-fixture', '--', '--self-check'],
                   cwd=ROOT, check=True, timeout=180)


@pytest.mark.parametrize('mode,count', [('reference', 12), ('symbols', 10)])
def test_icon_export_to_isolated_directory(tmp_path, mode, count):
    subprocess.run(CARGO + ['--features', 'ui-tests', '--bin', 'export-history-icons', '--',
                           '--' + mode, '--output', str(tmp_path)], cwd=ROOT, check=True, timeout=180)
    icons = list(tmp_path.glob('*.png'))
    assert len(icons) == count
    for icon in icons:
        header = icon.read_bytes()[:24]
        assert header[:8] == b'\x89PNG\r\n\x1a\n'
        assert struct.unpack('>II', header[16:24]) == (48, 48)
