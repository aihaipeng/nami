#!/usr/bin/env python3
"""Run isolated checks by UI, functional behavior, or a specific DFX concern."""
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
SUITES = (
    'ui', 'functional', 'dfx-performance', 'dfx-reliability',
    'dfx-security', 'dfx-diagnostics', 'dfx-compatibility', 'all',
)
MACOS_SUITES = {'ui', 'dfx-security', 'all'}


def python_path(root=ROOT, windows=None):
    if windows is None:
        windows = os.name == 'nt'
    return root / '.venv' / ('Scripts/python.exe' if windows else 'bin/python')


def suite_commands(suite, python):
    """Return a serial command/environment list; never execute during collection."""
    npm = 'npm.cmd' if os.name == 'nt' else 'npm'
    pytest = [str(python), '-m', 'pytest', '-q']
    cargo = ['cargo', 'test', '--manifest-path', 'src-tauri/Cargo.toml']

    def frontend(project=None):
        return ([npm, 'exec', '--', 'vitest', 'run'] + (['--project', project] if project else []), {})

    def rust(filter=None, *options):
        return (cargo + list(options) + ([filter] if filter else []), {})

    def py(*args, visible=False):
        return (pytest + list(args), {'NAMI_UI_TEST_VISIBLE': '1' if visible else '0'})

    hidden = py('-s', '-m', 'native')
    # The shared portable-host case already ran with the hidden native suite.
    visible = py('-s', 'tests/python/test_native_ui.py::test_native_ui', visible=True)
    signing = py('-s', '-m', 'signing')
    tools = py('-s', '-m', 'tools')
    units = py('-m', 'not native and not signing and not tools')
    performance = (cargo + ['--release', '--lib', 'database::tests::benchmark_large_history',
                            '--', '--ignored', '--exact', '--nocapture'], {})
    clipboard_performance = (cargo + ['--release', '--lib', 'database::clipboard::tests::benchmark_large_clipboard',
                                      '--', '--ignored', '--exact', '--nocapture'], {})
    static = [
        ([npm, 'run', 'build'], {}),
        (['cargo', 'fmt', '--manifest-path', 'src-tauri/Cargo.toml', '--check'], {}),
        (['cargo', 'clippy', '--manifest-path', 'src-tauri/Cargo.toml', '--all-targets', '--', '-D', 'warnings'], {}),
        (['cargo', 'clippy', '--manifest-path', 'src-tauri/Cargo.toml', '--all-targets', '--all-features', '--', '-D', 'warnings'], {}),
    ]
    commands = {
        'ui': [frontend('ui'), hidden, visible],
        'functional': [frontend('functional'), rust('history::tests', '--lib'),
                       rust('model::tests', '--lib'), rust('translation::tests', '--lib')],
        'dfx-performance': [performance, clipboard_performance],
        'dfx-reliability': [frontend('dfx-reliability'), rust('database::tests', '--lib')],
        'dfx-security': [frontend('dfx-security'), py('-m', 'dfx_security and not signing'),
                         rust('selection::tests', '--bin', 'nami'),
                         rust('platform::macos::tests', '--bin', 'nami'), signing],
        'dfx-diagnostics': [py('-m', 'dfx_diagnostics and not native')],
        'dfx-compatibility': [frontend('dfx-compatibility'),
                              py('-m', 'dfx_compatibility and not native'), *static],
        'all': [*static, frontend(), units, rust(), rust(None, '--all-features'),
                tools, hidden, signing, performance, clipboard_performance, visible],
    }
    if suite not in commands:
        raise ValueError(f'Unknown test suite: {suite}')
    return commands[suite]


def run_suite(suite, python):
    if suite in MACOS_SUITES and sys.platform != 'darwin':
        raise SystemExit(f'{suite} checks require macOS for native or signing cases; '
                         'use dfx-compatibility for shared checks. Skips do not verify platform behavior.')
    for command, overrides in suite_commands(suite, python):
        print('+ ' + ' '.join(command), flush=True)
        environment = {**os.environ, 'NAMI_UI_TEST_VISIBLE': '0', **overrides}
        result = subprocess.run(command, cwd=ROOT, env=environment)
        if result.returncode:
            return result.returncode
    return 0


def main(args=None):
    args = sys.argv[1:] if args is None else args
    choices = ('setup', *MODES, *SUITES)
    if len(args) != 1 or args[0] not in choices:
        raise SystemExit('Usage: python tools/dev.py ' + '|'.join(choices))
    mode = args[0]
    python = python_path()
    if mode == 'setup':
        subprocess.run([sys.executable, '-m', 'venv', str(ROOT / '.venv')], check=True, cwd=ROOT)
        subprocess.run([str(python), '-m', 'pip', 'install', '-r', str(ROOT / 'requirements-dev.txt')], check=True, cwd=ROOT)
        return
    if mode in MODES and mode != 'unit' and sys.platform != 'darwin':
        raise SystemExit(f'{mode} checks require macOS; use npm run verify:portable for shared checks. '
                         'Windows/Linux native interaction is not verified by skipped macOS tests.')
    if not python.is_file():
        raise SystemExit('Project Python environment missing. Run npm run setup:python first.')
    if mode in SUITES:
        raise SystemExit(run_suite(mode, python))
    environment = {**os.environ, 'NAMI_UI_TEST_VISIBLE': '1' if mode == 'native-visible' else '0'}
    result = subprocess.run([str(python), '-m', 'pytest', '-q', *MODES[mode]], cwd=ROOT, env=environment)
    raise SystemExit(result.returncode)


if __name__ == '__main__':
    main()
