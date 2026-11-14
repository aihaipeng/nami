import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid

import pytest

ROOT = Path(__file__).resolve().parents[2]


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
    subprocess.run([
        'npm', 'run', 'tauri', '--', 'build', '--features', 'ui-tests', '--config',
        str(ROOT / 'tests/native/tauri.ui-tests.conf.json'),
    ], cwd=ROOT, env={**os.environ, 'CARGO_TARGET_DIR': str(target)}, check=True)
    run_id = uuid.uuid4().hex
    executable = target / 'release/bundle/macos/Nami UI Tests.app/Contents/MacOS/nami'
    run = subprocess.run([str(executable)], cwd=ROOT, timeout=90, env={
        **os.environ, 'NAMI_UI_TEST_REPORT': str(report), 'NAMI_UI_TEST_RUN_ID': run_id,
    })
    assert report.is_file(), 'Native test process did not produce its report'
    result = json.loads(report.read_text())
    selected = os.environ.get('NAMI_UI_TEST_CASE')
    if selected:
        names = {'clipboard-actions': 'clipboard action timing', 'clipboard-editor': 'clipboard editor layout', 'clipboard-media': 'clipboard media layout'}
        assert selected in names, 'Unknown targeted native test'
        assert result['runId'] == run_id
        assert result['testCase'] == selected
        assert result['passed'] is True, result['domCases']
        assert result['cases'] == []
        assert [item['name'] for item in result['domCases']] == [names[selected]]
        assert all(item['passed'] for item in result['domCases'])
    else:
        check_report(result, run_id, visible)
    assert run.returncode == 0, f'Native test process exited with {run.returncode}'
    for item in result['cases'] + result['domCases']:
        print(f"PASS {item['name']} ({len(item['samples'])} samples)")
    print(f"Native UI report: {report}\nScope: {result['scope']}")
    print('OS mouse live-resize and material/remote-content visual quality remain unverified.')


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
