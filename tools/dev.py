#!/usr/bin/env python3
"""Run project Python checks with the platform's isolated virtual environment."""
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
MODES = {
    'unit': ['-m', 'not native and not signing and not tools'],
    'native': ['-s', '-m', 'native'],
    'native-visible': ['-s', '-m', 'native'],
    'signing': ['-s', '-m', 'signing'],
    'tools': ['-s', '-m', 'tools'],
}


def python_path(root=ROOT, windows=None):
    if windows is None:
        windows = os.name == 'nt'
    return root / '.venv' / ('Scripts/python.exe' if windows else 'bin/python')


def main(args=None):
    args = sys.argv[1:] if args is None else args
    if len(args) != 1 or args[0] not in ('setup', *MODES):
        raise SystemExit('Usage: python tools/dev.py setup|unit|native|native-visible|signing|tools')
    mode = args[0]
    python = python_path()
    if mode == 'setup':
        subprocess.run([sys.executable, '-m', 'venv', str(ROOT / '.venv')], check=True, cwd=ROOT)
        subprocess.run([str(python), '-m', 'pip', 'install', '-r', str(ROOT / 'requirements-dev.txt')], check=True, cwd=ROOT)
        return
    if mode != 'unit' and sys.platform != 'darwin':
        raise SystemExit(f'{mode} checks require macOS; use npm run verify:portable for shared checks. '
                         'Windows/Linux native interaction is not verified by skipped macOS tests.')
    if not python.is_file():
        raise SystemExit('Project Python environment missing. Run npm run setup:python first.')
    environment = {**os.environ, 'NAMI_UI_TEST_VISIBLE': '1' if mode == 'native-visible' else '0'}
    result = subprocess.run([str(python), '-m', 'pytest', '-q', *MODES[mode]], cwd=ROOT, env=environment)
    raise SystemExit(result.returncode)


if __name__ == '__main__':
    main()
