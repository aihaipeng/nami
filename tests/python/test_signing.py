import plistlib
import shutil
import sys

import pytest

pytestmark = [pytest.mark.signing, pytest.mark.skipif(sys.platform != 'darwin', reason='macOS codesign required')]


def test_certificate_pinned_identity_accepts_updates_and_rejects_replacements(local_app, tmp_path):
    fingerprint = local_app.identity()
    apps = []
    for version in (1, 2):
        app = tmp_path / f'version-{version}.app'
        (app / 'Contents/MacOS').mkdir(parents=True)
        (app / 'Contents/Resources').mkdir()
        shutil.copyfile('/usr/bin/true', app / 'Contents/MacOS/nami')
        (app / 'Contents/MacOS/nami').chmod(0o755)
        (app / 'Contents/Resources/version').write_text(str(version))
        (app / 'Contents/Info.plist').write_bytes(plistlib.dumps({
            'CFBundleIdentifier': local_app.BUNDLE_ID, 'CFBundleExecutable': 'nami',
            'CFBundlePackageType': 'APPL', 'CFBundleVersion': str(version),
        }))
        local_app.run('codesign', '--force', '--sign', fingerprint, '--identifier', local_app.BUNDLE_ID, str(app))
        local_app.verify(app, fingerprint)
        apps.append(app)
    local_app.run('codesign', '--verify', '--strict', '-R', '=' + local_app.requirement(apps[0]), str(apps[1]))
    local_app.run('codesign', '--force', '--sign', '-', str(apps[1]))
    with pytest.raises(RuntimeError):
        local_app.verify(apps[1], fingerprint)
    local_app.run('codesign', '--force', '--sign', fingerprint, '--requirements',
                  '=designated => identifier "dev.local.nami"', str(apps[1]))
    with pytest.raises(RuntimeError):
        local_app.verify(apps[1], fingerprint)
