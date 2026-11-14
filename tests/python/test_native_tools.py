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


@pytest.mark.parametrize('mode,count', [('reference', 14), ('symbols', 10), ('tray', 1)])
def test_icon_export_to_isolated_directory(tmp_path, mode, count):
    subprocess.run(CARGO + ['--features', 'ui-tests', '--bin', 'export-history-icons', '--',
                           '--' + mode, '--output', str(tmp_path)], cwd=ROOT, check=True, timeout=180)
    icons = list(tmp_path.glob('*.png'))
    assert len(icons) == count
    for icon in icons:
        if mode == 'tray':
            assert icon.read_bytes() == (ROOT / 'src-tauri/icons/tray-icon.png').read_bytes()
        header = icon.read_bytes()[:24]
        assert header[:8] == b'\x89PNG\r\n\x1a\n'
        assert struct.unpack('>II', header[16:24]) == (48, 48)


def test_nami_artwork_is_white_outlines_without_runtime_fonts():
    import xml.etree.ElementTree as ET
    sources = [ROOT / 'src-tauri/icons/app-icon.svg', ROOT / 'src/assets/history/reference/nami-flowing-outlined.svg', ROOT / 'src/assets/history/reference/nami-wordmark.svg']
    paths = []
    for source in sources:
        root = ET.fromstring(source.read_text())
        assert not root.findall('.//{http://www.w3.org/2000/svg}text')
        assert 'font-' not in source.read_text()
        group = root.find('{http://www.w3.org/2000/svg}g')
        assert group.attrib['fill'].lower() == '#ffffff'
        outlines = group.findall('{http://www.w3.org/2000/svg}path')
        assert len(outlines) == 5
        assert outlines[1].attrib['stroke'].lower() == '#ffffff'
        paths.append([outline.attrib['d'] for outline in outlines])
    assert paths[0] == paths[1] == paths[2]
