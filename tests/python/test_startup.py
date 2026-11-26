"""Guard resident startup configuration; explicit menu/shortcut actions stay available."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def test_resident_startup_does_not_open_translation():
    config = json.loads((ROOT / 'src-tauri/tauri.conf.json').read_text())
    assert config['app']['windows'] == [], 'Startup must not create declarative windows'
    source = (ROOT / 'src-tauri/src/main.rs').read_text()
    setup_tail = source.split('clipboard_runtime::start(app.handle());', 1)[1].split('.build(tauri::generate_context!())', 1)[0]
    assert 'windows::show' not in setup_tail, 'Resident startup must not show a product window'
    assert 'RunEvent::Reopen' not in source, 'macOS launch/reopen must not auto-open translation'
    assert 'tray.build(app)?;' in source
    assert 'app.global_shortcut().register(*shortcut)' in source


def test_clipboard_image_policy_allows_only_local_preview_sources():
    config = json.loads((ROOT / 'src-tauri/tauri.conf.json').read_text())
    policy = dict((parts[0], set(parts[1:])) for directive in config['app']['security']['csp'].split(';') if (parts := directive.split()))
    media = {'blob:', 'nami-media:', 'http://nami-media.localhost'}
    assert policy['img-src'] == {"'self'", 'data:', *media}
    for directive in ('default-src', 'script-src', 'connect-src'):
        assert policy[directive].isdisjoint(media)
