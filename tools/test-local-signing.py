#!/usr/bin/env python3
"""Check identity continuity across different signed builds using synthetic apps."""
import importlib.util
from pathlib import Path
import plistlib
import shutil
import tempfile

spec = importlib.util.spec_from_file_location('local_app', Path(__file__).with_name('local-app.py'))
local = importlib.util.module_from_spec(spec)
spec.loader.exec_module(local)
fingerprint = local.identity()

with tempfile.TemporaryDirectory(prefix='nami-signing-test-') as temporary:
    apps = []
    for version in (1, 2):
        app = Path(temporary) / f'version-{version}.app'
        (app / 'Contents/MacOS').mkdir(parents=True)
        (app / 'Contents/Resources').mkdir()
        shutil.copyfile('/usr/bin/true', app / 'Contents/MacOS/nami')
        (app / 'Contents/MacOS/nami').chmod(0o755)
        (app / 'Contents/Resources/version').write_text(str(version))
        (app / 'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleIdentifier': local.BUNDLE_ID, 'CFBundleExecutable': 'nami', 'CFBundlePackageType': 'APPL', 'CFBundleVersion': str(version)}))
        local.run('codesign', '--force', '--sign', fingerprint, '--identifier', local.BUNDLE_ID, str(app))
        local.verify(app, fingerprint)
        apps.append(app)
    local.run('codesign', '--verify', '--strict', '-R', '=' + local.requirement(apps[0]), str(apps[1]))
    print('PASS different builds satisfy the same certificate-pinned identity')
    local.run('codesign', '--force', '--sign', '-', str(apps[1]))
    try:
        local.verify(apps[1], fingerprint)
    except (RuntimeError, StopIteration):
        print('PASS ad-hoc replacement rejected')
    else:
        raise RuntimeError('Ad-hoc replacement unexpectedly accepted')
    local.run('codesign', '--force', '--sign', fingerprint, '--requirements', '=designated => identifier "dev.local.nami"', str(apps[1]))
    try:
        local.verify(apps[1], fingerprint)
    except RuntimeError:
        print('PASS weakened identifier-only requirement rejected')
    else:
        raise RuntimeError('Unpinned replacement unexpectedly accepted')
