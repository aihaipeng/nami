import importlib.util
from pathlib import Path
from types import SimpleNamespace

import pytest

pytestmark = pytest.mark.dfx_compatibility

spec = importlib.util.spec_from_file_location('dev', Path(__file__).resolve().parents[2] / 'tools/dev.py')
dev = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dev)


def test_virtual_environment_paths(tmp_path):
    assert dev.python_path(tmp_path, windows=True) == tmp_path / '.venv/Scripts/python.exe'
    assert dev.python_path(tmp_path, windows=False) == tmp_path / '.venv/bin/python'


@pytest.mark.parametrize('mode,visible', [('native', '0'), ('native-visible', '1')])
def test_native_mode_sets_environment_and_preserves_failure(tmp_path, monkeypatch, mode, visible):
    executable = tmp_path / 'python'
    executable.touch()
    monkeypatch.setattr(dev, 'python_path', lambda: executable)
    monkeypatch.setattr(dev.sys, 'platform', 'darwin')
    calls = []

    def run(args, **kwargs):
        calls.append((args, kwargs))
        return SimpleNamespace(returncode=7)

    monkeypatch.setattr(dev.subprocess, 'run', run)
    with pytest.raises(SystemExit) as exit:
        dev.main([mode])
    assert exit.value.code == 7
    assert calls[0][0] == [str(executable), '-m', 'pytest', '-q', '-s', '-m', 'native']
    assert calls[0][1]['env']['NAMI_UI_TEST_VISIBLE'] == visible


@pytest.mark.parametrize('failed_phase', ['translation_smoke_', 'ocr_search_', 'rich_text_smoke_', 'translation-ui', 'native'])
def test_smoke_runs_core_cases_and_propagates_failures(tmp_path, monkeypatch, failed_phase):
    executable = tmp_path / 'python'
    executable.touch()
    monkeypatch.setattr(dev, 'python_path', lambda: executable)
    monkeypatch.setattr(dev.sys, 'platform', 'darwin')
    monkeypatch.setenv('NAMI_UI_TEST_CASE', 'clipboard-editor')
    calls = []

    def run(args, **kwargs):
        phase = args[-1] if args[0] == 'cargo' else 'translation-ui' if args[0] == 'npm' else 'native'
        if phase == 'translation-ui':
            assert args[-1] == 'tests/frontend/translation-reliability.test.tsx'
        calls.append(phase)
        if phase == 'native':
            assert args[-2:] == ['tests/python/test_cross_app_fixture.py', 'tests/python/test_native_ui.py::test_native_ui']
            assert 'NAMI_UI_TEST_CASE' not in kwargs['env']
            assert kwargs['env']['NAMI_UI_TEST_SMOKE'] == '1'
            assert kwargs['env']['NAMI_UI_TEST_VISIBLE'] == '1'
        return SimpleNamespace(returncode=7 if phase == failed_phase else 0)

    monkeypatch.setattr(dev.subprocess, 'run', run)
    with pytest.raises(SystemExit) as exit:
        dev.main(['smoke'])
    assert exit.value.code == 7
    phases = ['translation_smoke_', 'ocr_search_', 'rich_text_smoke_', 'translation-ui', 'native']
    assert calls == phases[:phases.index(failed_phase) + 1]


@pytest.mark.parametrize('platform', ['win32', 'linux'])
@pytest.mark.parametrize('mode', ['native', 'native-visible', 'signing', 'tools'])
def test_macos_checks_do_not_succeed_by_skipping_on_other_platforms(monkeypatch, platform, mode):
    monkeypatch.setattr(dev.sys, 'platform', platform)
    monkeypatch.setattr(dev.subprocess, 'run', lambda *args, **kwargs: pytest.fail('Must refuse before running skipped tests'))
    with pytest.raises(SystemExit, match='require macOS'):
        dev.main([mode])


def test_missing_environment_explains_setup(tmp_path, monkeypatch):
    monkeypatch.setattr(dev, 'python_path', lambda: tmp_path / 'missing-python')
    with pytest.raises(SystemExit, match='setup:python'):
        dev.main(['unit'])


def test_named_suites_have_explicit_commands_and_no_implicit_network_tests(tmp_path):
    expected = {'ui', 'functional', 'dfx-performance', 'dfx-reliability', 'dfx-security',
                'dfx-diagnostics', 'dfx-compatibility', 'all'}
    assert set(dev.SUITES) == expected
    for suite in dev.SUITES:
        commands = dev.suite_commands(suite, tmp_path / 'python')
        assert commands
        assert all(isinstance(command, list) and command for command, _ in commands)
        assert all('live_services_translate_synthetic_text' not in command for command, _ in commands)
        ignored = [command for command, _ in commands if '--ignored' in command]
        assert bool(ignored) == (suite in {'all', 'dfx-performance'})
        assert all(any(name in command for name in ['database::tests::benchmark_large_history', 'database::clipboard::tests::benchmark_large_clipboard']) and '--exact' in command for command in ignored)


@pytest.mark.parametrize('suite,project', [
    ('ui', 'ui'), ('functional', 'functional'), ('dfx-reliability', 'dfx-reliability'),
    ('dfx-security', 'dfx-security'), ('dfx-compatibility', 'dfx-compatibility'),
])
def test_frontend_project_mapping(tmp_path, suite, project):
    command, _ = dev.suite_commands(suite, tmp_path / 'python')[0]
    assert command[1:] == ['exec', '--', 'vitest', 'run', '--project', project]


def test_performance_is_release_exact_and_does_not_launch_native_windows(tmp_path):
    commands = dev.suite_commands('dfx-performance', tmp_path / 'python')
    assert len(commands) == 2
    for (command, _), name in zip(commands, ['database::tests::benchmark_large_history', 'database::clipboard::tests::benchmark_large_clipboard']):
        assert command == ['cargo', 'test', '--manifest-path', 'src-tauri/Cargo.toml', '--release', '--lib',
                           name, '--', '--ignored', '--exact', '--nocapture']


def test_security_suite_covers_selection_orchestration_and_macos_capture(tmp_path):
    commands = dev.suite_commands('dfx-security', tmp_path / 'python')
    rust_checks = [command for command, _ in commands if command[0] == 'cargo']
    assert [command[-1] for command in rust_checks] == ['selection::tests', 'platform::macos::']
    assert all(command[-3:-1] == ['--bin', 'nami'] for command in rust_checks)


def test_all_runs_native_serially_once_and_performance_before_visible(tmp_path):
    commands = dev.suite_commands('all', tmp_path / 'python')
    hidden = [i for i, (command, env) in enumerate(commands) if command[-2:] == ['-m', 'native']]
    visible = [i for i, (command, env) in enumerate(commands) if env.get('NAMI_UI_TEST_VISIBLE') == '1']
    performance = [i for i, (command, env) in enumerate(commands) if '--release' in command]
    assert len(hidden) == len(visible) == 1
    assert len(performance) == 2
    assert hidden[0] < performance[0] < performance[1] < visible[0]
    assert commands[visible[0]][0][-1] == 'tests/python/test_native_ui.py::test_native_ui'
    assert all('pytest' not in command or '-n' not in command for command, _ in commands)


@pytest.mark.parametrize('platform', ['linux', 'win32'])
@pytest.mark.parametrize('suite', ['ui', 'dfx-security', 'all'])
def test_native_suites_refuse_other_platforms_before_subprocess(monkeypatch, tmp_path, platform, suite):
    monkeypatch.setattr(dev.sys, 'platform', platform)
    monkeypatch.setattr(dev.subprocess, 'run', lambda *args, **kwargs: pytest.fail('Must refuse before launch'))
    with pytest.raises(SystemExit, match='require macOS'):
        dev.run_suite(suite, tmp_path / 'python')


@pytest.mark.parametrize('suite', ['functional', 'dfx-performance', 'dfx-reliability', 'dfx-diagnostics', 'dfx-compatibility'])
def test_shared_suites_can_run_on_non_macos(monkeypatch, tmp_path, suite):
    monkeypatch.setattr(dev.sys, 'platform', 'linux')
    calls = []
    monkeypatch.setattr(dev.subprocess, 'run', lambda command, **kwargs:
                        calls.append((command, kwargs)) or SimpleNamespace(returncode=0))
    assert dev.run_suite(suite, tmp_path / 'python') == 0
    assert len(calls) == len(dev.suite_commands(suite, tmp_path / 'python'))
    assert all(call[1]['cwd'] == dev.ROOT for call in calls)
    assert all(call[1]['env']['NAMI_UI_TEST_VISIBLE'] == '0' for call in calls)


def test_suite_stops_immediately_on_first_failure(monkeypatch, tmp_path):
    monkeypatch.setattr(dev.sys, 'platform', 'darwin')
    calls = []

    def run(command, **kwargs):
        calls.append(command)
        return SimpleNamespace(returncode=23 if len(calls) == 2 else 0)

    monkeypatch.setattr(dev.subprocess, 'run', run)
    assert dev.run_suite('all', tmp_path / 'python') == 23
    assert len(calls) == 2


def test_suite_cli_preserves_exit_status(monkeypatch, tmp_path):
    executable = tmp_path / 'python'
    executable.touch()
    monkeypatch.setattr(dev, 'python_path', lambda: executable)
    monkeypatch.setattr(dev, 'run_suite', lambda suite, python: 12)
    with pytest.raises(SystemExit) as exit:
        dev.main(['functional'])
    assert exit.value.code == 12


def test_unknown_suite_is_rejected_without_running(monkeypatch, tmp_path):
    with pytest.raises(ValueError, match='Unknown test suite'):
        dev.suite_commands('unknown', tmp_path / 'python')
    with pytest.raises(SystemExit, match='dfx-performance'):
        dev.main(['unknown'])
