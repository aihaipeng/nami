import json
import os
from pathlib import Path
import subprocess
import sys
import uuid

import pytest

ROOT = Path(__file__).resolve().parents[2]
EXPECTED_CASES = {
    'shared history startup',
    'React actions update shared Rust history',
    'shared history protects favorite and pinned records',
    'React selection restores cached translation through IPC',
    'React deletion removes shared record and preserves selection',
    'portable history layout preserves source across toggles and widths',
}


def check_portable_report(result, run_id):
    assert result['runId'] == run_id, 'Portable report belongs to an older run'
    assert result['passed'] is True, result.get('error')
    assert {case['name'] for case in result['cases']} == EXPECTED_CASES
    assert all(case['passed'] is True for case in result['cases'])
    assert 'real Tauri IPC' in result['scope']


@pytest.mark.native
@pytest.mark.skipif(sys.platform != 'darwin', reason='macOS graphical session required for this portable-host check')
def test_portable_shared_history_in_real_webview():
    target = ROOT / 'src-tauri/target/portable-ui-tests'
    target.mkdir(parents=True, exist_ok=True)
    report = target / 'portable-ui-report.json'
    subprocess.run([
        'npm', 'run', 'tauri', '--', 'build', '--features', 'ui-tests,portable-ui', '--config',
        str(ROOT / 'tests/native/tauri.portable-tests.conf.json'),
    ], cwd=ROOT, env={**os.environ, 'CARGO_TARGET_DIR': str(target)}, check=True)
    executable = target / 'release/bundle/macos/Nami Portable Tests.app/Contents/MacOS/nami'
    run_id = uuid.uuid4().hex
    run = subprocess.run([str(executable)], cwd=ROOT, timeout=90, env={
        **os.environ, 'NAMI_UI_TEST_REPORT': str(report), 'NAMI_UI_TEST_RUN_ID': run_id,
    })
    assert report.is_file(), 'Portable host did not produce a report'
    result = json.loads(report.read_text())
    check_portable_report(result, run_id)
    assert run.returncode == 0, f'Portable host exited with {run.returncode}'
    for case in result['cases']:
        print(f"PASS {case['name']}")
    print(f"Portable UI report: {report}\nScope: {result['scope']}")
    print('Windows/Linux OS interactions, real mouse drags and material pixel quality remain unverified.')


@pytest.mark.parametrize('change', ['old-run', 'failed', 'missing-case'])
def test_portable_report_rejects_stale_failed_or_missing_coverage(change):
    result = {
        'runId': 'current', 'passed': True, 'scope': 'real Tauri IPC',
        'cases': [{'name': name, 'passed': True} for name in EXPECTED_CASES],
    }
    if change == 'old-run':
        result['runId'] = 'old'
    elif change == 'failed':
        result['passed'] = False
    else:
        result['cases'].pop()
    with pytest.raises(AssertionError):
        check_portable_report(result, 'current')
