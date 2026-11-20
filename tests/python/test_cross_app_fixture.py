import os
import plistlib
import shutil
import subprocess
import sys
from pathlib import Path

import pytest


def build_cross_app_fixture(tmp_path):
    root = Path(__file__).resolve().parents[2]
    target = root / 'src-tauri/target/cross-app-fixture'
    native = root / 'tests/native'
    contents = tmp_path / 'Nami Cross-App Fixture.app' / 'Contents'
    (contents / 'MacOS').mkdir(parents=True)
    plist = (native / 'CrossAppFixture.Info.plist').read_bytes()
    metadata = plistlib.loads(plist)
    (contents / 'Info.plist').write_bytes(plist)
    executable = contents / 'MacOS' / metadata['CFBundleExecutable']
    subprocess.run([
        'cargo', 'build', '--locked', '--manifest-path', str(root / 'src-tauri/Cargo.toml'),
        '--target-dir', str(target), '--features', 'cross-app-fixture', '--bin', executable.name,
    ], cwd=root, env={**os.environ, 'CARGO_PROFILE_DEV_DEBUG': '0'}, check=True, timeout=300)
    shutil.copy2(target / 'debug' / executable.name, executable)
    return executable


@pytest.mark.tools
@pytest.mark.skipif(sys.platform != 'darwin', reason='AppKit fixture requires macOS')
def test_cross_app_fixture_without_showing_a_window(tmp_path):
    executable = build_cross_app_fixture(tmp_path)
    result = subprocess.run([str(executable), '--self-test'],
                            capture_output=True, text=True, timeout=15)
    assert result.returncode == 0, result.stderr
    assert result.stdout.startswith('PASS: isolated copy, replacement paste, empty copy/paste')
