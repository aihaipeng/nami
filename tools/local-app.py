#!/usr/bin/env python3
"""Build/install the local app with a certificate-pinned identity."""
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
PRODUCT = ROOT / 'src-tauri/target/release/bundle/macos/Nami.app'
INSTALLED = Path('/Applications/Nami.app')
BUNDLE_ID = 'dev.local.nami'


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def identity():
    fingerprint = (ROOT / '.nami-signing-identity').read_text().strip()
    if not re.fullmatch(r'[0-9A-Fa-f]{40}', fingerprint):
        raise RuntimeError('Invalid local signing identity')
    identities = run('security', 'find-identity', '-p', 'codesigning', capture_output=True, text=True).stdout
    if fingerprint.upper() not in identities.upper():
        raise RuntimeError('Restore the original Nami certificate and private key; identity is unavailable')
    return fingerprint


def setup():
    config = ROOT / '.nami-signing-identity'
    if config.exists():
        identity()
        print('Nami fixed local signing identity is available.')
        return
    keychain = run('security', 'default-keychain', '-d', 'user', capture_output=True, text=True).stdout.strip().strip('"')
    with tempfile.TemporaryDirectory(prefix='nami-certificate-') as directory:
        scratch = Path(directory)
        certificate = scratch / 'certificate.pem'
        found = subprocess.run(['security', 'find-certificate', '-a', '-c', 'Nami Local Development', '-p', keychain],
                               capture_output=True, check=False)
        certificate.write_bytes(found.stdout)
        if not found.stdout:
            settings = scratch / 'signing.cnf'
            settings.write_text('[req]\ndistinguished_name = name\n[name]\n[signing]\n'
                                'basicConstraints = critical,CA:false\nkeyUsage = critical,digitalSignature\n'
                                'extendedKeyUsage = critical,codeSigning\n')
            key = scratch / 'Nami Local Development.key'
            run('/usr/bin/openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '3650',
                '-subj', '/CN=Nami Local Development/O=Nami Local Development',
                '-keyout', str(key), '-out', str(certificate), '-extensions', 'signing', '-config', str(settings),
                capture_output=True)
            key.chmod(0o600)
            run('security', 'import', str(key), '-k', keychain, '-T', '/usr/bin/codesign')
            run('security', 'import', str(certificate), '-k', keychain)
        if certificate.read_bytes().count(b'BEGIN CERTIFICATE') != 1:
            raise RuntimeError('Multiple Nami certificates found; automatic selection stopped')
        output = run('/usr/bin/openssl', 'x509', '-in', str(certificate), '-noout', '-fingerprint', '-sha1',
                     capture_output=True, text=True).stdout
        fingerprint = output.split('=', 1)[1].strip().replace(':', '')
        if not re.fullmatch(r'[0-9A-Fa-f]{40}', fingerprint):
            raise RuntimeError('Invalid certificate fingerprint')
        identities = run('security', 'find-identity', '-p', 'codesigning', capture_output=True, text=True).stdout
        if fingerprint.upper() not in identities.upper():
            raise RuntimeError('Restore the original certificate and private key; no replacement will be generated')
        with open(config, 'x', opener=lambda path, flags: os.open(path, flags, 0o600)) as output:
            output.write(fingerprint + '\n')
    print('Nami local signing identity pinned. Keep the original certificate and private key in Keychain.')


def requirement(app):
    result = run('codesign', '-d', '-r-', str(app), capture_output=True, text=True)
    for line in (result.stdout + result.stderr).splitlines():
        if line.removeprefix('# ').startswith('designated => '):
            return line.removeprefix('# ').removeprefix('designated => ')
    raise RuntimeError('Unable to inspect the application signing requirement')


def verify(app, fingerprint):
    with (app / 'Contents/Info.plist').open('rb') as source:
        if plistlib.load(source)['CFBundleIdentifier'] != BUNDLE_ID:
            raise RuntimeError('Unexpected application bundle identity')
    run('codesign', '--verify', '--deep', '--strict', str(app))
    expected = f'identifier "{BUNDLE_ID}" and certificate root = H"{fingerprint.lower()}"'
    if requirement(app) != expected:
        raise RuntimeError('Signing requirement must bind the Nami identifier and original certificate')
    run('codesign', '--verify', '--strict', '-R', '=' + expected, str(app))


def build(fingerprint):
    with tempfile.TemporaryDirectory(prefix='nami-build-config-') as directory:
        config = Path(directory) / 'signing.json'
        config.write_text(json.dumps({'bundle': {'macOS': {'signingIdentity': fingerprint}}}))
        run('npm', 'run', 'tauri', '--', 'build', '--config', str(config), cwd=ROOT)
    verify(PRODUCT, fingerprint)


def running():
    result = run('ps', '-axo', 'pid=,comm=', capture_output=True, text=True)
    executable = str(INSTALLED / 'Contents/MacOS/nami')
    return [int(parts[0]) for line in result.stdout.splitlines() if len(parts := line.strip().split(None, 1)) == 2 and parts[1] == executable]


def install(fingerprint):
    verify(PRODUCT, fingerprint)
    if INSTALLED.exists():
        with (INSTALLED / 'Contents/Info.plist').open('rb') as source:
            if plistlib.load(source)['CFBundleIdentifier'] != BUNDLE_ID:
                raise RuntimeError('The installed path belongs to a different app; replacement stopped')
        old = requirement(INSTALLED)
        if 'cdhash' not in old:
            run('codesign', '--verify', '--strict', '-R', '=' + old, str(PRODUCT))
    if running():
        run('osascript', '-e', 'tell application id "dev.local.nami" to quit')
        for _ in range(30):
            if not running():
                break
            time.sleep(0.1)
        for pid in running():
            os.kill(pid, signal.SIGTERM)
        for _ in range(30):
            if not running():
                break
            time.sleep(0.1)
        if running():
            raise RuntimeError('Nami did not exit; installation stopped')
    run('ditto', str(PRODUCT), str(INSTALLED))
    verify(INSTALLED, fingerprint)
    for app in (PRODUCT, INSTALLED):
        executable = app / 'Contents/MacOS/nami'
        print(hashlib.sha256(executable.read_bytes()).hexdigest(), executable)
    registry = '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister'
    # A build-only bundle may already be absent from LaunchServices.
    subprocess.run([registry, '-u', str(PRODUCT)], check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    run(registry, '-f', str(INSTALLED))
    run('open', str(INSTALLED))


def main(args=None):
    args = sys.argv[1:] if args is None else args
    if len(args) != 1 or args[0] not in ('setup', 'build', 'install'):
        raise SystemExit('Usage: python tools/local-app.py setup|build|install')
    if sys.platform != 'darwin':
        raise SystemExit('Local certificate signing and /Applications installation require macOS. '
                         'Use npm run tauri -- build for a native Windows/Linux build.')
    if args[0] == 'setup':
        setup()
        return
    fingerprint = identity()
    if args[0] == 'build':
        build(fingerprint)
    else:
        install(fingerprint)


if __name__ == '__main__':
    main()
