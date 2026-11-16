use crate::{windows, AppState};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Listener, Manager};

const SCOPE: &str = "shared React history and real Tauri IPC in an isolated portable host; no OS mouse drag, clipboard access, translation network, or material pixel verification";

fn finish(app: &AppHandle, finished: &AtomicBool, result: Value) {
    if finished.swap(true, Ordering::SeqCst) {
        return;
    }
    let passed = result["passed"] == true;
    let report = json!({
        "runId": std::env::var("NAMI_UI_TEST_RUN_ID").unwrap_or_default(),
        "passed": passed,
        "scope": SCOPE,
        "cases": result["cases"],
        "error": result["error"],
    });
    let saved = std::env::var_os("NAMI_UI_TEST_REPORT")
        .ok_or_else(|| "missing report path".to_string())
        .and_then(|path| {
            std::fs::write(path, report.to_string()).map_err(|error| error.to_string())
        });
    if let Err(error) = &saved {
        eprintln!("Portable UI report: {error}");
    }
    app.exit(if passed && saved.is_ok() { 0 } else { 1 });
}

pub fn start(app: &AppHandle) -> Result<(), String> {
    if app.config().identifier != "dev.local.nami.portable-tests" {
        return Err("portable tests require an isolated application identity".into());
    }
    if std::env::var("NAMI_UI_TEST_RUN_ID")
        .map(|value| value.is_empty())
        .unwrap_or(true)
        || std::env::var_os("NAMI_UI_TEST_REPORT").is_none()
    {
        return Err("portable tests require run ID and report path".into());
    }
    app.state::<AppState>()
        .database
        .lock()
        .unwrap()
        .as_mut()
        .ok_or("missing isolated test database")?
        .clipboard_insert(&nami::clipboard::ClipboardRecord {
            id: "portable-clipboard-sample".into(),
            kind: "text".into(),
            text: "Synthetic portable clipboard text".into(),
            date: "2099-01-01T00:00:00Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
            image_path: None,
            fingerprint: nami::clipboard::fingerprint("text", b"Synthetic portable clipboard text"),
        })?;
    {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        state.history_visible = true;
        state.translation_visible = true;
        state.history_pinned = true;
        state.translation_pinned = true;
        state.history_width_initialized = true;
    }
    windows::show(app, "translation")?;
    app.state::<AppState>()
        .windows
        .lock()
        .unwrap()
        .history_visible = true;
    windows::apply_translation(app)?;
    if app
        .get_window("translation")
        .ok_or("missing test window")?
        .is_focused()
        .map_err(|_| "window_focus")?
    {
        return Err("isolated portable host must not take keyboard focus".into());
    }
    let finished = Arc::new(AtomicBool::new(false));
    let owned = app.clone();
    let received = finished.clone();
    app.listen("portable-test-result", move |event| {
        let result = serde_json::from_str::<Value>(event.payload()).unwrap_or_else(|error| {
            json!({"passed":false,"cases":[],"error":format!("Invalid test result: {error}")})
        });
        finish(&owned, &received, result);
    });
    let owned = app.clone();
    std::thread::spawn(move || {
        for _ in 0..120 {
            if finished.load(Ordering::SeqCst) {
                return;
            }
            if let Some(view) = owned.get_webview("translation") {
                if let Err(error) = view.eval(SCRIPT) {
                    finish(
                        &owned,
                        &finished,
                        json!({"passed":false,"cases":[],"error":format!("WebView eval: {error}")}),
                    );
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        finish(
            &owned,
            &finished,
            json!({"passed":false,"cases":[],"error":"portable UI tests timed out"}),
        );
    });
    Ok(())
}

const SCRIPT: &str = r#"
(() => {
  if (!window.__TAURI_INTERNALS__ || !document.querySelector('.source-input') || window.__NAMI_PORTABLE_TEST_RUNNING__) return;
  window.__NAMI_PORTABLE_TEST_RUNNING__ = true;
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const cases = [];
  let currentCase = 'startup';
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const pause = (duration) => new Promise((resolve) => setTimeout(resolve, duration));
  async function waitFor(test, message) {
    const end = performance.now() + 6000;
    while (performance.now() < end) {
      if (await test()) return;
      await pause(20);
    }
    throw new Error(message);
  }
  const rows = () => [...document.querySelectorAll('#translation-history [data-record-id]')];
  const row = (id) => rows().find((element) => element.dataset.recordId === id);
  const source = () => document.querySelector('.source-input');
  const snapshot = () => invoke('history_snapshot');
  const record = async (id) => (await snapshot()).records.find((record) => record.id === id);
  const flag = async (id, key, value) => waitFor(async () => (await record(id))?.[key] === value, 'store flag did not update: ' + key);
  const clickAction = (id, index) => {
    const button = row(id)?.querySelectorAll('.history-row-actions button')[index];
    assert(button && !button.disabled, 'history action unavailable');
    button.click();
  };
  const pass = (name) => { cases.push({name, passed:true}); };
  (async () => {
    const events = [];
    const callback = window.__TAURI_INTERNALS__.transformCallback((event) => events.push(event.payload));
    const eventId = await invoke('plugin:event|listen', {event:'history_changed', target:{kind:'Any'}, handler:callback});
    try {
      currentCase = 'shared history startup';
      await waitFor(async () => rows().length === 20 && (await snapshot()).records.length === 20, 'synthetic history did not load into shared React view');
      await invoke('clipboard_query', {query:'',favoritesOnly:false,cursor:null});
      const app = await invoke('app_snapshot');
      assert(app.capabilities.nativeHistory === false && app.capabilities.nativeChrome === false, 'portable host incorrectly reports native controls');
      for (const capability of ['nativeMaterial', 'selection', 'clipboardWrite', 'autoPaste', 'accessibilityPermission']) {
        assert(app.capabilities[capability] === false, 'isolated host exposes system capability: ' + capability);
      }
      assert(document.documentElement.dataset.nativeHistory === 'false', 'React history capability not applied');
      assert(source().value === '', 'translation source must start empty');
      pass(currentCase);

      currentCase = 'shared record presentation and menus';
      const historyPanel = document.querySelector('#translation-history');
      const clipboardPanel = document.querySelector('.clipboard-panel');
      await waitFor(() => clipboardPanel.querySelector('[data-record-id="portable-clipboard-sample"]'), 'isolated clipboard fixture did not render');
      const historyNode = row('sample-03');
      const clipboardNode = clipboardPanel.querySelector('[data-record-id="portable-clipboard-sample"]');
      const iconStyle = (node) => {
        const style = getComputedStyle(node);
        return {width:style.width,height:style.height,mask:style.maskImage||style.webkitMaskImage,color:style.color,opacity:style.opacity};
      };
      for (const name of ['doc.on.doc','star','pin','trash']) {
        const left = historyNode.querySelector(`[data-symbol="${name}"]`), right = clipboardNode.querySelector(`[data-symbol="${name}"]`);
        assert(left && right, 'shared row icon missing: '+name);
        assert(JSON.stringify(iconStyle(left)) === JSON.stringify(iconStyle(right)), 'row icon appearance differs: '+name);
        assert(getComputedStyle(left).width === '14px' && getComputedStyle(left).height === '14px', 'shared row icon must remain 14pt: '+name);
        assert(getComputedStyle(left.parentElement).opacity === getComputedStyle(right.parentElement).opacity, 'disabled icon opacity differs: '+name);
      }
      assert(!historyNode.querySelector('.history-row-actions [data-symbol="languages"]'), 'inline retranslation icon returned');
      for (const selector of ['.history-search > .history-symbol','.history-search button .history-symbol','.history-shortcuts .history-symbol','.history-favorites .history-symbol']) {
        assert(JSON.stringify(iconStyle(historyPanel.querySelector(selector))) === JSON.stringify(iconStyle(clipboardPanel.querySelector(selector))), 'shared panel icon differs: '+selector);
      }
      for (const panel of [historyPanel,clipboardPanel]) {
        const style=getComputedStyle(panel.querySelector('.history-search'));
        assert(style.borderTopWidth === '0px' && style.boxShadow === 'none', 'record search gained a border or focus ring');
      }
      const menuAppearance = async (recordNode, expectedFirst) => {
        recordNode.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:40,clientY:100}));
        await waitFor(()=>document.querySelector('[role="menu"]'), 'shared context menu did not open');
        const menu=document.querySelector('[role="menu"]');
        const entries=[...menu.querySelectorAll('[role="menuitem"]')];
        assert(entries[0].textContent === expectedFirst, 'wrong context menu action for record type');
        const common={};
        for(const name of ['star','pin','trash']) {
          const icon=menu.querySelector(`[data-symbol="${name}"]`);
          assert(icon, 'menu icon missing: '+name);
          common[name]=iconStyle(icon);
        }
        const style=getComputedStyle(menu);
        common.surface={background:style.backgroundColor,border:style.borderColor,radius:style.borderRadius,shadow:style.boxShadow,blur:style.backdropFilter||style.webkitBackdropFilter};
        menu.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
        await waitFor(()=>!document.querySelector('[role="menu"]'), 'Escape did not close context menu');
        return common;
      };
      const historyMenu = await menuAppearance(historyNode, 'Retranslate');
      await invoke('records_view_set',{view:'clipboard'});
      await waitFor(()=>!clipboardPanel.closest('.record-view').hidden, 'clipboard presentation did not activate');
      const clipboardMenu = await menuAppearance(clipboardNode, 'Paste');
      assert(JSON.stringify(historyMenu) === JSON.stringify(clipboardMenu), 'shared menu colors, icons or material differ');
      await invoke('records_view_set',{view:'translation'});
      await waitFor(()=>!historyPanel.closest('.record-view').hidden, 'translation presentation did not restore');
      pass(currentCase);

      currentCase = 'shared loading animation in both record views';
      const internalFetch = window.fetch;
      const loadingStyles=[];
      function setSearch(panel,value){
        const input=panel.querySelector('.history-search input');
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,value);
        input.dispatchEvent(new Event('input',{bubbles:true}));
      }
      try {
        for(const [view,panel,command,query] of [['translation',historyPanel,'history_query','train'],['clipboard',clipboardPanel,'clipboard_query','Synthetic']]) {
          await invoke('records_view_set',{view});
          await waitFor(()=>!panel.closest('.record-view').hidden, 'loading view did not activate');
          let release;
          window.fetch=(input,options)=>new URL(typeof input==='string'?input:input.url,location.href).pathname.endsWith('/'+command)?new Promise((resolve,reject)=>{release=()=>internalFetch(input,options).then(resolve,reject);}):internalFetch(input,options);
          setSearch(panel,query);
          await waitFor(()=>typeof release==='function' && panel.querySelector('.activity-beam > span'), 'record query did not expose shared loading animation in '+view);
          const track=panel.querySelector('.activity-beam'),beam=track.firstElementChild,style=getComputedStyle(beam),parent=getComputedStyle(track);
          loadingStyles.push({height:style.height,maxWidth:style.maxWidth,radius:style.borderRadius,gradient:style.backgroundImage,shadow:style.boxShadow,animation:style.animationName,duration:style.animationDuration,timing:style.animationTimingFunction,trackHeight:parent.height,left:parent.left,right:parent.right});
          window.fetch=internalFetch;
          release();
          await waitFor(()=>!panel.querySelector('.activity-beam'), 'record query loading animation did not finish');
          setSearch(panel,'');
          await waitFor(()=>!panel.querySelector('.activity-beam') && panel.querySelector('.history-search input').value==='', 'record search did not clear');
        }
      } finally { window.fetch=internalFetch; }
      assert(JSON.stringify(loadingStyles[0])===JSON.stringify(loadingStyles[1]), 'record loading effects differ: '+JSON.stringify(loadingStyles));
      assert(loadingStyles[0].height==='2px' && loadingStyles[0].trackHeight==='8px', 'loading indicator is not the shared thin beam');
      if(!matchMedia('(prefers-reduced-motion: reduce)').matches) {
        assert(loadingStyles[0].animation==='beam-travel, beam-pulse' && loadingStyles[0].duration==='1.3s, 0.55s', 'shared loading motion parameters changed');
      }
      await invoke('records_view_set',{view:'translation'});
      await waitFor(async()=>rows().length===20 && (await snapshot()).records.length===20, 'loading test did not restore full history');
      pass(currentCase);

      currentCase = 'React actions update shared Rust history';
      const id = 'sample-03';
      const original = await record(id);
      assert(!original.isFavorite && !original.isPinned, 'fixture action record must be unprotected');
      clickAction(id, 1);
      await flag(id, 'isFavorite', true);
      await waitFor(() => row(id)?.querySelectorAll('.history-row-actions button')[1]?.getAttribute('aria-pressed') === 'true', 'favorite did not render from shared state');
      clickAction(id, 2);
      await flag(id, 'isPinned', true);
      await waitFor(() => row(id)?.classList.contains('pinned'), 'pinned record did not render in pinned section');
      await waitFor(() => events.some((event) => event.records.some((record) => record.id === id && record.isPinned && record.isFavorite)), 'history_changed event missing authoritative flags');
      pass(currentCase);

      currentCase = 'shared history protects favorite and pinned records';
      const before = await snapshot();
      let rejected = false;
      try { await invoke('history_record_update', {recordId:id, action:'delete'}); }
      catch (error) { rejected = String(error).includes('history_protected'); }
      assert(rejected, 'protected deletion was accepted');
      const after = await snapshot();
      assert(after.revision === before.revision && after.records.length === 20, 'protected deletion changed history');
      assert(row(id)?.querySelectorAll('.history-row-actions button').length === 3, 'protected record still exposes delete');
      clickAction(id, 2);
      await flag(id, 'isPinned', false);
      await waitFor(() => !row(id)?.classList.contains('pinned'), 'unpin did not update DOM');
      clickAction(id, 1);
      await flag(id, 'isFavorite', false);
      await waitFor(() => row(id)?.querySelectorAll('.history-row-actions button').length === 4, 'unprotected record did not restore delete');
      pass(currentCase);

      currentCase = 'React selection restores cached translation through IPC';
      row(id).querySelector('.history-row-select').click();
      await waitFor(async () => source().value === original.text && (await snapshot()).selectedId === id && row(id)?.classList.contains('selected'), 'history selection did not restore source and shared selection');
      assert(document.querySelector('.result-column').textContent === original.translations[0], 'cached translation was not restored');
      pass(currentCase);

      currentCase = 'React deletion removes shared record and preserves selection';
      clickAction('sample-05', 3);
      await waitFor(async () => !row('sample-05') && !(await record('sample-05')), 'deleted record remained in DOM or shared store');
      assert((await snapshot()).records.length === 19, 'shared deletion changed unexpected records');
      assert((await snapshot()).selectedId === id && source().value === original.text, 'deleting another record lost selection');
      pass(currentCase);

      currentCase = 'portable history layout preserves source across toggles and widths';
      const input = source();
      const selected = row(id);
      for (let index=0; index<12; index++) await invoke('history_toggle');
      await invoke('history_visibility_set', {visible:true});
      const expectedTranslationWidth = (await invoke('app_snapshot')).window.translationWidth;
      for (const width of [320, 440, 280, 520, 360]) {
        await invoke('history_width_set', {width, initialize:false});
        await waitFor(() => document.querySelector('.history-divider')?.getAttribute('aria-valuenow') === String(width), 'history width event did not reach React');
        await new Promise(requestAnimationFrame);
        const nextHistory = document.querySelector('.history-clip').getBoundingClientRect();
        const nextTranslation = document.querySelector('.translation-clip').getBoundingClientRect();
        assert(Math.abs(nextHistory.width-width)<1 && Math.abs(nextTranslation.width-expectedTranslationWidth)<1,
          'history resize changed translation width on next frame: ' + JSON.stringify({width,history:nextHistory.width,translation:nextTranslation.width,expectedTranslationWidth}));
        await waitFor(() => {
          const history = document.querySelector('.history-clip').getBoundingClientRect();
          const translation = document.querySelector('.translation-clip').getBoundingClientRect();
          return Math.abs(history.width-width)<1 && translation.width>100 && Math.abs(history.right-translation.left)<1;
        }, 'shared regions did not form a contiguous layout at width ' + width);
        assert(source() === input && row(id) === selected, 'portable layout recreated source or record elements');
        assert(source().value === original.text, 'portable layout lost input');
      }
      assert(!document.querySelector('[role=alert]'), 'application reported an error during portable regression: ' + JSON.stringify([...document.querySelectorAll('[role=alert]')].map(node=>({class:node.className,text:node.textContent}))));
      pass(currentCase);

      currentCase = 'unified records switch shares host and preserves cached translation';
      const hostInput = source();
      const sharedWidth = (await invoke('app_snapshot')).window.historyWidth;
      await invoke('records_view_set', {view:'clipboard'});
      await waitFor(async () => (await invoke('app_snapshot')).window.recordView === 'clipboard' && !document.querySelector('.clipboard-panel').closest('.record-view').hidden, 'clipboard view did not appear in shared host');
      assert(document.querySelector('.clipboard-panel [aria-label="Details unavailable"]').disabled, 'clipboard detail placeholder must be disabled');
      assert(document.querySelector('.clipboard-panel [aria-label="Clipboard"]').getAttribute('aria-pressed') === 'true', 'clipboard mode not highlighted');
      assert((await invoke('app_snapshot')).window.translationVisible === false, 'clipboard switch kept translation details open');
      assert((await invoke('app_snapshot')).window.historyWidth === sharedWidth, 'mode switch changed shared list width');
      await invoke('records_view_set', {view:'translation'});
      await waitFor(() => !document.querySelector('#translation-history').closest('.record-view').hidden, 'translation view did not restore');
      assert(source() === hostInput && source().value === original.text, 'mode switch recreated or cleared translation input');
      assert((await invoke('app_snapshot')).window.translationVisible === false, 'translation mode unexpectedly expanded details');
      await invoke('records_details_toggle');
      await waitFor(async () => (await invoke('app_snapshot')).window.translationVisible && (await snapshot()).selectedId === id, 'details did not restore associated record');
      assert(document.querySelector('.result-column').textContent === original.translations[0], 'details reopening lost cached result');
      row(id).querySelector('.history-row-select').click();
      await waitFor(async () => (await invoke('app_snapshot')).window.translationVisible, 'repeat selection closed details');
      assert(!document.querySelector('[role=alert]'), 'unified records produced an error');
      pass(currentCase);
      await invoke('plugin:event|emit', {event:'portable-test-result', payload:{passed:true,cases,error:null}});
    } finally {
      await invoke('plugin:event|unlisten', {event:'history_changed', eventId});
    }
  })().catch(async (error) => {
    await invoke('plugin:event|emit', {event:'portable-test-result', payload:{passed:false,cases,error:currentCase + ': ' + String(error)}});
  });
})();
"#;
