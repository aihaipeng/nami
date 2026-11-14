"""Ensure named test projects neither omit nor duplicate frontend tests."""
from collections import Counter
from pathlib import Path, PurePosixPath, PureWindowsPath
import re

import pytest

ROOT = Path(__file__).resolve().parents[2]
pytestmark = pytest.mark.dfx_diagnostics


@pytest.mark.parametrize('path_type', [PurePosixPath, PureWindowsPath], ids=['posix', 'windows'])
def test_every_frontend_test_belongs_to_exactly_one_project(path_type):
    configuration = (ROOT / 'vite.config.ts').read_text()
    paths = re.findall(r"'([^']+\.test\.tsx?)'", configuration)
    counts = Counter(paths)
    duplicates = [path for path, count in counts.items() if count != 1]
    assert not duplicates, f'Tests appear in multiple projects: {duplicates}'
    actual = {path_type(*path.relative_to(ROOT).parts).as_posix() for base in ['src', 'tests/frontend']
              for path in (ROOT / base).rglob('*') if path.name.endswith(('.test.ts', '.test.tsx'))}
    assert set(paths) == actual, f'Missing from projects: {actual - set(paths)}; stale paths: {set(paths) - actual}'


@pytest.mark.parametrize('path_type', [PurePosixPath, PureWindowsPath], ids=['posix', 'windows'])
def test_windows_absolute_inventory_paths_use_portable_relative_components(path_type):
    root = PureWindowsPath('C:/repo')
    path = root / 'tests/frontend/clipboard-ui.test.tsx'
    assert path_type(*path.relative_to(root).parts).as_posix() == 'tests/frontend/clipboard-ui.test.tsx'


def test_public_suite_commands_match_the_python_runner():
    import importlib.util
    import json

    spec = importlib.util.spec_from_file_location('suite_runner', ROOT / 'tools/dev.py')
    runner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runner)
    scripts = json.loads((ROOT / 'package.json').read_text())['scripts']
    for suite in runner.SUITES:
        name = 'test:' + suite.replace('-', ':')
        assert scripts[name] == f'python tools/dev.py {suite}'
