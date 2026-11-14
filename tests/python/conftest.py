import importlib.util
from pathlib import Path

import pytest


@pytest.fixture(scope='session')
def local_app():
    path = Path(__file__).resolve().parents[2] / 'tools/local-app.py'
    spec = importlib.util.spec_from_file_location('local_app', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
