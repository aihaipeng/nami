import importlib.util
from pathlib import Path
from types import SimpleNamespace

import pytest

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
