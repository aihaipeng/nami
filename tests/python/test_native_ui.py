import copy
from contextlib import nullcontext
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import uuid

import pytest

from test_cross_app_fixture import build_cross_app_fixture

ROOT = Path(__file__).resolve().parents[2]
CASE_NAMES = {'clipboard-actions': 'clipboard action timing', 'clipboard-editor': 'clipboard editor layout', 'clipboard-media': 'clipboard media layout', 'clipboard-kind-text': 'clipboard kind filters and text actions', 'translation-actions': 'translation action timing', 'unified-records': 'unified records panel', 'records-geometry': 'records geometry', 'records-pagination': 'shared record child pagination IPC', 'records-open': 'records open and reopen readiness', 'selection-open': 'selection empty summon and clipboard protection'}
CASE_NAMES['record-display-shortcuts'] = 'record previews and macOS window shortcuts'
CASE_NAMES['clipboard-quick-paste'] = 'scoped numbered clipboard paste'
CASE_NAMES['clipboard-menu-paste'] = 'clipboard menu formats and file reveal'
CASE_NAMES['clipboard-translate-shortcut'] = 'clipboard Command-T focus routing'
CASE_NAMES['clipboard-shortcuts'] = 'clipboard shortcut smoke through macOS and WebView'
CASE_NAMES['records-pinned'] = 'shared pinned boundary coverage'
CASE_NAMES['translation-controls'] = 'real WK + native controls: anchors, long source, narrow menus and native action bridge'
CASE_NAMES['translation-partial'] = 'translation partial columns and retry through WKWebView IPC'
CASE_NAMES['clipboard-settings'] = 'clipboard settings: recording, shortcuts, storage and protected cleanup through real WebView IPC'
CASE_NAMES['translation-settings'] = 'translation settings: proxy validation and shortcut capture in WKWebView'
CASE_NAMES['clipboard-capture-burst'] = 'clipboard burst sampling and idle cost'
CASE_NAMES['clipboard-image-index'] = 'clipboard local image indexing and performance'
CASE_NAMES['clipboard-details'] = 'clipboard full details and hover navigation'
CASE_NAMES['cross-app'] = 'cross-process selection and clipboard safety'
SMOKE_CASES = (
    ('clipboard-shortcuts', True), ('clipboard-translate-shortcut', False), ('clipboard-menu-paste', False), ('selection-open', True),
    # This case measures layout and native chrome, not visible presentation frames.
    ('translation-controls', False), ('translation-partial', False), ('record-display-shortcuts', True),
    ('clipboard-settings', False), ('clipboard-kind-text', False), ('clipboard-image-index', False),
    ('unified-records', False),
)


def check_selected_report(result, run_id, selected):
    assert selected in CASE_NAMES, 'Unknown targeted native test'
    assert result['runId'] == run_id
    assert result['testCase'] == selected
    assert result['passed'] is True, result['domCases']
    assert result['cases'] == []
    assert [item['name'] for item in result['domCases']] == [CASE_NAMES[selected]]
    assert all(item['passed'] is True and item['samples'] for item in result['domCases'])


def check_report(result, run_id, visible):
    assert result['runId'] == run_id, 'Native UI report belongs to an older run'
    assert result['passed'] is True, 'Native regression failed'
    assert result['cases'], 'Native geometry cases missing'
    assert result['domCases'], 'Native/DOM cases missing'
    for item in result['cases'] + result['domCases']:
        assert item['passed'] is True, f"{item['name']}: {item.get('error')}"
    if visible:
        presentation = result['visiblePacedPresentation']
        assert presentation['enabled'] is True
        assert presentation['programmaticVisibleResizeObserved'] is True
        assert presentation['postPresentationSamples'] > 0
        assert presentation['observedLayerSamples'] > 0
        assert presentation['maxModelPresentationDelta'] <= 0.5
        assert presentation['visibleLiveResizeVerified'] is False


@pytest.mark.native
@pytest.mark.skipif(sys.platform != 'darwin', reason='macOS graphical session required')
def test_native_ui():
    target = ROOT / 'src-tauri/target/ui-tests'
    target.mkdir(parents=True, exist_ok=True)
    visible = os.environ.get('NAMI_UI_TEST_VISIBLE') == '1'
    report = target / ('native-ui-visible-report.json' if visible else 'native-ui-report.json')
    smoke = os.environ.get('NAMI_UI_TEST_SMOKE') == '1'
    selected_case = os.environ.get('NAMI_UI_TEST_CASE')
    cross_mode = os.environ.get('NAMI_UI_TEST_CROSS_APP')
    assert not (smoke and cross_mode in {'full', 'denied'}), 'Run full and denied as targeted checks on the same prebuilt host'
    targeted_external = selected_case == 'cross-app' and cross_mode in {'full', 'denied', 'denied-fresh'}
    if smoke or not targeted_external:
        subprocess.run([
            'npm', 'run', 'tauri', '--', 'build', '--features', 'ui-tests', '--config',
            str(ROOT / 'tests/native/tauri.ui-tests.conf.json'),
        ], cwd=ROOT, env={**os.environ, 'CARGO_TARGET_DIR': str(target)}, check=True)
    cases = SMOKE_CASES if smoke else ((selected_case, visible),)
    if smoke and os.environ.get('NAMI_UI_TEST_CROSS_APP'):
        cases = (*cases, ('cross-app', True))
    for selected, case_visible in cases:
        if smoke:
            report = target / f'native-ui-smoke-{selected}-report.json'
        elif selected in {'clipboard-capture-burst', 'clipboard-image-index'}:
            baseline = '-baseline' if os.environ.get('NAMI_UI_TEST_CAPTURE_BASELINE') == '1' else ''
            report = target / f'native-ui-{selected}{baseline}-report.json'
        elif selected == 'cross-app':
            mode = os.environ.get('NAMI_UI_TEST_CROSS_APP')
            assert mode in {'selection', 'full', 'denied', 'denied-fresh'}, 'Explicit cross-app mode required'
            report = target / f'native-ui-cross-app-{mode}-report.json'
        run_native_case(target, report, case_visible, selected)
    print('OS mouse live-resize and material/remote-content visual quality remain unverified.')


def run_native_case(target, report, visible, selected):
    run_id = uuid.uuid4().hex
    environment = {**os.environ, 'NAMI_UI_TEST_VISIBLE': '1' if visible else '0',
                   'NAMI_UI_TEST_REPORT': str(report), 'NAMI_UI_TEST_RUN_ID': run_id}
    if selected:
        environment['NAMI_UI_TEST_CASE'] = selected
        print(f'Running native case: {selected}', flush=True)
    executable = target / 'release/bundle/macos/Nami UI Tests.app/Contents/MacOS/nami'
    mode = environment.get('NAMI_UI_TEST_CROSS_APP') if selected == 'cross-app' else None
    pairing = check_session_host(executable, report, mode) if mode in {'full', 'denied'} else None
    fixture_dir = tempfile.TemporaryDirectory(prefix='nami-cross-app-', dir=target) if selected == 'cross-app' else nullcontext()
    with fixture_dir as directory:
        if directory:
            environment['NAMI_UI_TEST_FIXTURE'] = str(build_cross_app_fixture(Path(directory)))
        if mode == 'denied-fresh':
            run = run_untrusted_host(target, environment)
        elif pairing is not None:
            run = launch_services_host(executable.parents[2], environment)
        else:
            run = subprocess.run([str(executable)], cwd=ROOT, timeout=90, env=environment)
    assert report.is_file(), f'Native test process did not produce its report: {report}'
    result = json.loads(report.read_text())
    if selected:
        check_selected_report(result, run_id, selected)
    else:
        check_report(result, run_id, visible)
    assert run.returncode == 0, f'Native test process exited with {run.returncode}'
    if pairing is not None:
        check_cross_app_permission_evidence(result, mode)
        assert session_host_identity(executable) == pairing['host'], 'Test bundle or console account changed during the run'
        result['permissionPair'] = pairing
        report.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    for item in result['cases'] + result['domCases']:
        print(f"PASS {item['name']} ({len(item['samples'])} samples)")
    print(f"Native UI report: {report}\nScope: {result['scope']}")


def run_untrusted_host(target, environment):
    subprocess.run([
        'npm', 'run', 'tauri', '--', 'build', '--features', 'ui-tests', '--config',
        str(ROOT / 'tests/native/tauri.ui-tests-denial.conf.json'),
    ], cwd=ROOT, env={**os.environ, 'CARGO_TARGET_DIR': str(target)}, check=True)
    return launch_services_host(target / 'release/bundle/macos/Nami UI Denial Tests.app', environment)


def launch_services_host(bundle, environment):
    command = ['/usr/bin/open', '-n', '-W']
    for key in ('NAMI_UI_TEST_CASE', 'NAMI_UI_TEST_CROSS_APP', 'NAMI_UI_TEST_FIXTURE',
                'NAMI_UI_TEST_REPORT', 'NAMI_UI_TEST_RUN_ID', 'NAMI_UI_TEST_VISIBLE'):
        command.extend(('--env', f'{key}={environment[key]}'))
    return subprocess.run([*command, str(bundle)], cwd=ROOT, timeout=90)


def session_host_identity(executable):
    console_uid = Path('/dev/console').stat().st_uid
    assert console_uid == os.getuid(), 'Run in the foreground macOS test login session'
    assert executable.is_file(), 'Build and sign the isolated Nami UI Tests bundle first'
    info = (executable.parents[1] / 'Info.plist').read_bytes()
    assert plistlib.loads(info)['CFBundleIdentifier'] == 'dev.local.nami.ui-tests'
    return {'consoleUid': console_uid, 'bundle': str(executable.parents[2].resolve()),
            'executableSha256': hashlib.sha256(executable.read_bytes()).hexdigest(),
            'infoSha256': hashlib.sha256(info).hexdigest()}


def check_cross_app_permission_evidence(result, mode):
    samples = result['domCases'][0]['samples']
    assert any(sample.get('axAuthorized') is (mode == 'full') for sample in samples)
    if mode == 'full':
        assert any(sample.get('actualExternalPasteReceived') is True for sample in samples)
    else:
        assert any(sample.get('pastePermissionDenied') is True and sample.get('noPasteReceived') is True
                   and sample.get('freshIdentity') is False for sample in samples)


def check_session_host(executable, report, mode):
    production = subprocess.run(['/usr/bin/pgrep', '-u', str(os.getuid()), '-fx', '/Applications/Nami.app/Contents/MacOS/nami'],
                                stdout=subprocess.DEVNULL, check=False)
    assert production.returncode == 1, 'Quit the production Nami in the dedicated test login session before running this check'
    host = session_host_identity(executable)
    subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', host['bundle']], check=True)
    if mode == 'full':
        return {'host': host, 'mode': mode}
    authorized = report.with_name('native-ui-cross-app-full-report.json')
    assert authorized.is_file(), 'Run full successfully before revoking the isolated host permission'
    previous = json.loads(authorized.read_text())
    assert previous.get('runId'), 'Missing authorized run identity'
    check_selected_report(previous, previous['runId'], 'cross-app')
    check_cross_app_permission_evidence(previous, 'full')
    assert previous.get('permissionPair') == {'host': host, 'mode': 'full'}, 'Use the same prebuilt bundle and test login as full; do not rebuild or re-sign'
    return {'host': host, 'mode': mode, 'authorizedRunId': previous['runId']}


@pytest.mark.dfx_diagnostics
@pytest.mark.parametrize('include_cross_app', [False, True])
def test_smoke_builds_once_and_runs_each_case_in_a_separate_report(tmp_path, monkeypatch, include_cross_app):
    monkeypatch.setitem(globals(), 'ROOT', tmp_path)
    monkeypatch.setenv('NAMI_UI_TEST_VISIBLE', '1')
    monkeypatch.setenv('NAMI_UI_TEST_SMOKE', '1')
    monkeypatch.setenv('NAMI_UI_TEST_CASE', 'clipboard-editor')
    if include_cross_app:
        monkeypatch.setenv('NAMI_UI_TEST_CROSS_APP', 'selection')
    else:
        monkeypatch.delenv('NAMI_UI_TEST_CROSS_APP', raising=False)
    calls = []
    monkeypatch.setattr(subprocess, 'run', lambda args, **kwargs: calls.append(args))
    monkeypatch.setitem(globals(), 'run_native_case',
                        lambda target, report, visible, selected: calls.append((report.name, visible, selected)))

    test_native_ui()

    assert calls[0][:4] == ['npm', 'run', 'tauri', '--']
    expected = [
        ('native-ui-smoke-clipboard-shortcuts-report.json', True, 'clipboard-shortcuts'),
        ('native-ui-smoke-clipboard-translate-shortcut-report.json', False, 'clipboard-translate-shortcut'),
        ('native-ui-smoke-clipboard-menu-paste-report.json', False, 'clipboard-menu-paste'),
        ('native-ui-smoke-selection-open-report.json', True, 'selection-open'),
        ('native-ui-smoke-translation-controls-report.json', False, 'translation-controls'),
        ('native-ui-smoke-translation-partial-report.json', False, 'translation-partial'),
        ('native-ui-smoke-record-display-shortcuts-report.json', True, 'record-display-shortcuts'),
        ('native-ui-smoke-clipboard-settings-report.json', False, 'clipboard-settings'),
        ('native-ui-smoke-clipboard-kind-text-report.json', False, 'clipboard-kind-text'),
        ('native-ui-smoke-clipboard-image-index-report.json', False, 'clipboard-image-index'),
        ('native-ui-smoke-unified-records-report.json', False, 'unified-records'),
    ]
    if include_cross_app:
        expected.append(('native-ui-smoke-cross-app-report.json', True, 'cross-app'))
    assert calls[1:] == expected


@pytest.mark.dfx_diagnostics
def test_fresh_denial_is_targeted_and_keeps_its_own_report(tmp_path, monkeypatch):
    monkeypatch.setitem(globals(), 'ROOT', tmp_path)
    monkeypatch.delenv('NAMI_UI_TEST_SMOKE', raising=False)
    monkeypatch.setenv('NAMI_UI_TEST_CASE', 'cross-app')
    monkeypatch.setenv('NAMI_UI_TEST_CROSS_APP', 'denied-fresh')
    calls = []
    monkeypatch.setattr(subprocess, 'run', lambda *args, **kwargs: pytest.fail('Normal bundle must not build'))
    monkeypatch.setitem(globals(), 'run_native_case',
                        lambda target, report, visible, selected: calls.append((report.name, selected)))

    test_native_ui()

    assert calls == [('native-ui-cross-app-denied-fresh-report.json', 'cross-app')]


@pytest.mark.dfx_diagnostics
@pytest.mark.parametrize('mode', ['full', 'denied'])
def test_session_checks_do_not_rebuild_the_test_bundle(tmp_path, monkeypatch, mode):
    monkeypatch.setitem(globals(), 'ROOT', tmp_path)
    monkeypatch.delenv('NAMI_UI_TEST_SMOKE', raising=False)
    monkeypatch.setenv('NAMI_UI_TEST_CASE', 'cross-app')
    monkeypatch.setenv('NAMI_UI_TEST_CROSS_APP', mode)
    monkeypatch.setenv('NAMI_UI_TEST_VISIBLE', '1')
    calls = []
    monkeypatch.setattr(subprocess, 'run', lambda *args, **kwargs: pytest.fail('Paired host must not rebuild'))
    monkeypatch.setitem(globals(), 'run_native_case',
                        lambda target, report, visible, selected: calls.append((report.name, visible, selected)))

    test_native_ui()

    assert calls == [(f'native-ui-cross-app-{mode}-report.json', True, 'cross-app')]


@pytest.mark.dfx_diagnostics
def test_revocation_requires_positive_paste_and_matching_host(tmp_path, monkeypatch):
    host = {'bundle': 'synthetic-path', 'consoleUid': 501, 'executableSha256': 'same', 'infoSha256': 'same'}
    monkeypatch.setitem(globals(), 'session_host_identity', lambda executable: host.copy())

    def run(args, **kwargs):
        return subprocess.CompletedProcess(args, 1 if args[0].endswith('pgrep') else 0)

    monkeypatch.setattr(subprocess, 'run', run)
    report = tmp_path / 'native-ui-cross-app-denied-report.json'
    authorized = tmp_path / 'native-ui-cross-app-full-report.json'
    with pytest.raises(AssertionError, match='Run full successfully'):
        check_session_host(tmp_path / 'nami', report, 'denied')
    verdict = {
        'runId': 'authorized', 'testCase': 'cross-app', 'passed': True, 'cases': [],
        'domCases': [{'name': CASE_NAMES['cross-app'], 'passed': True, 'samples': [{'axAuthorized': True}]}],
        'permissionPair': {'host': host.copy(), 'mode': 'full'},
    }
    authorized.write_text(json.dumps(verdict))
    with pytest.raises(AssertionError):
        check_session_host(tmp_path / 'nami', report, 'denied')
    verdict['domCases'][0]['samples'].append({'actualExternalPasteReceived': True})
    authorized.write_text(json.dumps(verdict))
    assert check_session_host(tmp_path / 'nami', report, 'denied')['authorizedRunId'] == 'authorized'
    for key in host:
        original = host[key]
        host[key] = 'changed'
        with pytest.raises(AssertionError, match='same prebuilt bundle and test login'):
            check_session_host(tmp_path / 'nami', report, 'denied')
        host[key] = original


@pytest.mark.dfx_diagnostics
def test_session_checks_reject_running_production_before_launch(tmp_path, monkeypatch):
    monkeypatch.setattr(subprocess, 'run', lambda args, **kwargs: subprocess.CompletedProcess(args, 0))
    monkeypatch.setitem(globals(), 'session_host_identity', lambda executable: pytest.fail('Must abort preflight'))
    with pytest.raises(AssertionError, match='Quit the production Nami'):
        check_session_host(tmp_path / 'nami', tmp_path / 'report.json', 'full')


def valid_report():
    return {
        'runId': 'current', 'passed': True,
        'cases': [{'name': 'geometry', 'passed': True}],
        'domCases': [{'name': 'interaction', 'passed': True}],
        'visiblePacedPresentation': {
            'enabled': True, 'programmaticVisibleResizeObserved': True,
            'postPresentationSamples': 120, 'observedLayerSamples': 240,
            'maxModelPresentationDelta': 0, 'visibleLiveResizeVerified': False,
        },
    }


@pytest.mark.dfx_diagnostics
@pytest.mark.parametrize('selected', ['clipboard-shortcuts', 'clipboard-translate-shortcut', 'clipboard-menu-paste', 'record-display-shortcuts', 'clipboard-kind-text', 'clipboard-details', 'clipboard-settings', 'translation-settings', 'records-open', 'selection-open', 'records-pinned'])
def test_selected_report_accepts_only_its_current_complete_case(selected):
    name = CASE_NAMES[selected]
    result = {'runId': 'current', 'testCase': selected, 'passed': True,
              'cases': [], 'domCases': [{'name': name, 'passed': True, 'samples': [{}]}]}
    check_selected_report(result, 'current', selected)
    for field, value in [('runId', 'old'), ('testCase', 'clipboard-media'), ('passed', False),
                         ('cases', [{}]), ('domCases', []),
                         ('domCases', [{'name': 'clipboard media layout', 'passed': True}]),
                         ('domCases', [{'name': name, 'passed': True, 'samples': []}]),
                         ('domCases', [{'name': name, 'passed': False, 'samples': [{}]}])]:
        altered = {**result, field: value}
        with pytest.raises(AssertionError):
            check_selected_report(altered, 'current', selected)


@pytest.mark.dfx_diagnostics
def test_report_accepts_current_success():
    check_report(valid_report(), 'current', True)


@pytest.mark.parametrize('field,value', [
    ('runId', 'old'), ('passed', False), ('cases', []), ('domCases', []),
    ('domCases', [{'name': 'failed interaction', 'passed': False}]),
    ('visiblePacedPresentation', {}),
])
@pytest.mark.dfx_diagnostics
def test_report_rejects_stale_failed_or_incomplete_results(field, value):
    result = valid_report()
    result[field] = value
    with pytest.raises((AssertionError, KeyError)):
        check_report(result, 'current', True)


@pytest.mark.parametrize('field,value', [
    ('enabled', False), ('programmaticVisibleResizeObserved', False),
    ('postPresentationSamples', 0), ('observedLayerSamples', 0),
    ('maxModelPresentationDelta', 2), ('visibleLiveResizeVerified', True),
])
@pytest.mark.dfx_diagnostics
def test_report_rejects_invalid_presentation(field, value):
    result = copy.deepcopy(valid_report())
    result['visiblePacedPresentation'][field] = value
    with pytest.raises(AssertionError):
        check_report(result, 'current', True)
