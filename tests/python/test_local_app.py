from types import SimpleNamespace
import json
import os

import pytest

pytestmark = [pytest.mark.dfx_security, pytest.mark.dfx_compatibility]


def test_setup_keeps_existing_identity(local_app, tmp_path, monkeypatch):
    monkeypatch.setattr(local_app, 'ROOT', tmp_path)
    config = tmp_path / '.nami-signing-identity'
    fingerprint = 'A' * 40
    config.write_text(fingerprint)
    calls = []

    def run(*args, **kwargs):
        calls.append(args)
        return SimpleNamespace(stdout=fingerprint)

    monkeypatch.setattr(local_app, 'run', run)
    local_app.setup()
    assert config.read_text() == fingerprint
    assert calls == [('security', 'find-identity', '-p', 'codesigning')]


@pytest.mark.parametrize('fingerprint', ['invalid', 'A' * 40])
def test_setup_never_replaces_unavailable_configured_identity(local_app, tmp_path, monkeypatch, fingerprint):
    monkeypatch.setattr(local_app, 'ROOT', tmp_path)
    config = tmp_path / '.nami-signing-identity'
    config.write_text(fingerprint)
    monkeypatch.setattr(local_app, 'run', lambda *args, **kwargs: SimpleNamespace(stdout=''))
    with pytest.raises(RuntimeError):
        local_app.setup()
    assert config.read_text() == fingerprint


@pytest.mark.parametrize('certificate_count,private_key_available', [(1, True), (1, False), (2, True)])
def test_setup_reuses_existing_certificate_or_refuses_ambiguity(local_app, tmp_path, monkeypatch,
                                                               certificate_count, private_key_available):
    monkeypatch.setattr(local_app, 'ROOT', tmp_path)
    fingerprint = 'A' * 40
    calls = []

    def run(*args, **kwargs):
        calls.append(args)
        if args[:2] == ('security', 'default-keychain'):
            return SimpleNamespace(stdout='  "/tmp/synthetic.keychain"\n')
        if args[:2] == ('/usr/bin/openssl', 'x509'):
            return SimpleNamespace(stdout='sha1 Fingerprint=' + ':'.join(['AA'] * 20))
        assert args == ('security', 'find-identity', '-p', 'codesigning')
        return SimpleNamespace(stdout=fingerprint if private_key_available else '')

    monkeypatch.setattr(local_app, 'run', run)
    monkeypatch.setattr(local_app.subprocess, 'run', lambda *args, **kwargs:
                        SimpleNamespace(stdout=b'-----BEGIN CERTIFICATE-----\n' * certificate_count))
    config = tmp_path / '.nami-signing-identity'
    if certificate_count == 1 and private_key_available:
        local_app.setup()
        assert config.read_text().strip() == fingerprint
        if os.name != 'nt':
            assert config.stat().st_mode & 0o777 == 0o600
    else:
        with pytest.raises(RuntimeError):
            local_app.setup()
        assert not config.exists()
    assert all('import' not in call and 'req' not in call for call in calls)


@pytest.mark.parametrize('platform', ['win32', 'linux'])
@pytest.mark.parametrize('mode', ['setup', 'build', 'install'])
def test_local_install_cli_refuses_non_macos(local_app, monkeypatch, platform, mode):
    monkeypatch.setattr(local_app.sys, 'platform', platform)
    monkeypatch.setattr(local_app, 'identity', lambda: pytest.fail('Must not access local signing on another platform'))
    with pytest.raises(SystemExit, match='require macOS'):
        local_app.main([mode])


def test_repository_name_is_lowercase_and_app_name_is_preserved(local_app):
    root = local_app.ROOT
    package = json.loads((root / 'package.json').read_text())
    tauri = json.loads((root / 'src-tauri/tauri.conf.json').read_text())
    assert package['name'] == 'nami'
    assert package['repository'] == {'type': 'git', 'url': 'https://github.com/aihaipeng/nami.git'}
    cargo = (root / 'src-tauri/Cargo.toml').read_text()
    assert 'name = "nami"' in cargo
    assert 'repository = "https://github.com/aihaipeng/nami.git"' in cargo
    assert tauri['productName'] == 'Nami'
    assert tauri['identifier'] == local_app.BUNDLE_ID == 'dev.local.nami'
    assert local_app.PRODUCT.name == local_app.INSTALLED.name == 'Nami.app'
