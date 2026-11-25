use crate::{windows, AppState};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Listener, Manager};

const SCOPE: &str = "shared React clipboard workspace and real Tauri IPC in an isolated portable host; no OS mouse drag, clipboard access, translation network, or material pixel verification";

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
    for (id, title, lines) in [
        ("portable-note-one", "First note", 8),
        ("portable-note-two", "Second note", 12),
    ] {
        let text = format!("Synthetic original {id}\n").repeat(lines);
        let state = app.state::<AppState>();
        let mut database = state.database.lock().unwrap();
        let database = database.as_mut().ok_or("missing isolated test database")?;
        database.clipboard_insert(&nami::clipboard::ClipboardRecord {
            id: id.into(),
            kind: "text".into(),
            text: text.clone(),
            date: "2098-01-01T00:00:00Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
            image_path: None,
            fingerprint: nami::clipboard::fingerprint("text", text.as_bytes()),
        })?;
        database.clipboard_edit(id, title, Some(&text))?;
    }
    for index in 0..16 {
        let id = format!("portable-fill-{index:02}");
        let text = format!("Synthetic filler item {index} for scroll verification");
        let state = app.state::<AppState>();
        let mut database = state.database.lock().unwrap();
        let database = database.as_mut().ok_or("missing isolated test database")?;
        database.clipboard_insert(&nami::clipboard::ClipboardRecord {
            id,
            kind: "text".into(),
            text: text.clone(),
            date: format!("2097-01-{:02}T00:00:00Z", index + 1),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
            image_path: None,
            fingerprint: nami::clipboard::fingerprint("text", text.as_bytes()),
        })?;
    }
    {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        state.standalone_translation_visible = false;
        state.clipboard_visible = true;
        state.clipboard_detail_visible = true;
        state.clipboard_translation_visible = false;
        state.clipboard_pinned = true;
        state.translation_pinned = true;
        state.clipboard_width_initialized = true;
    }
    windows::show(app, "clipboard")?;
    {
        let state = app.state::<AppState>();
        let mut regions = state.windows.lock().unwrap();
        regions.clipboard_detail_visible = true;
    }
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
  if (!window.__TAURI_INTERNALS__ || !document.querySelector('.clipboard-panel') || window.__NAMI_PORTABLE_TEST_RUNNING__) return;
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
  const clipboardPanel = () => document.querySelector('.clipboard-panel');
  const row = (id) => clipboardPanel().querySelector(`[data-record-id="${id}"]`);
  const pass = (name) => { cases.push({name, passed:true}); };
  (async () => {
    try {
      currentCase = 'portable clipboard startup';
      await waitFor(() => row('portable-clipboard-sample'), 'isolated clipboard fixture did not render');
      const app = await invoke('app_snapshot');
      assert(app.capabilities.nativeHistory === false && app.capabilities.nativeChrome === false, 'portable host incorrectly reports native controls');
      for (const capability of ['nativeMaterial', 'selection', 'clipboardWrite', 'autoPaste', 'accessibilityPermission']) {
        assert(app.capabilities[capability] === false, 'isolated host exposes system capability: ' + capability);
      }
      assert(document.documentElement.dataset.nativeHistory === 'false', 'React history capability not applied');
      assert(!document.querySelector('[aria-label="Translations"]'), 'removed translation history switcher returned');
      pass(currentCase);

      currentCase = 'shared record presentation and menus';
      const clipboardNode = row('portable-clipboard-sample');
      for (const name of ['doc.on.doc','languages','note','star','pin','trash']) {
        const icon = clipboardNode.querySelector(`[data-symbol="${name}"]`);
        assert(icon, 'shared row icon missing: '+name);
        assert(getComputedStyle(icon).width === '14px' && getComputedStyle(icon).height === '14px', 'shared row icon must remain 14pt: '+name);
      }
      const searchStyle = getComputedStyle(clipboardPanel().querySelector('.history-search'));
      assert(searchStyle.borderTopWidth === '0px' && searchStyle.boxShadow === 'none', 'record search gained a border or focus ring');
      clipboardNode.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:40,clientY:100}));
      await waitFor(()=>document.querySelector('[role="menu"]'), 'shared context menu did not open');
      const menu = document.querySelector('[role="menu"]');
      assert(menu.querySelector('[role="menuitem"]').textContent === 'Paste', 'wrong context menu action');
      menu.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
      await waitFor(()=>!document.querySelector('[role="menu"]'), 'Escape did not close context menu');
      pass(currentCase);

      currentCase = 'shared loading animation in clipboard view';
      const internalFetch = window.fetch;
      let loadingStyle;
      function setSearch(value){
        const input = clipboardPanel().querySelector('.history-search input');
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,value);
        input.dispatchEvent(new Event('input',{bubbles:true}));
      }
      try {
        let release;
        window.fetch=(input,options)=>new URL(typeof input==='string'?input:input.url,location.href).pathname.endsWith('/clipboard_query')?new Promise((resolve,reject)=>{release=()=>internalFetch(input,options).then(resolve,reject);}):internalFetch(input,options);
        setSearch('Synthetic');
        await waitFor(()=>typeof release==='function' && clipboardPanel().querySelector('.activity-beam[data-visible=true] > span'), 'clipboard query did not expose shared loading animation');
        const track=clipboardPanel().querySelector('.activity-beam'),beam=track.firstElementChild,style=getComputedStyle(beam),parent=getComputedStyle(track);
        loadingStyle = {height:style.height,trackHeight:parent.height,animation:style.animationName,timing:style.animationTimingFunction,opacityOnly:beam.getAnimations().every(animation=>animation.effect.getKeyframes().every(frame=>!('left' in frame)&&!('width' in frame)&&!('transform' in frame)&&'opacity' in frame))};
        window.fetch=internalFetch;
        release();
        await waitFor(()=>!clipboardPanel().querySelector('.activity-beam[aria-hidden=false]'), 'clipboard query loading animation did not finish');
        setSearch('');
        await waitFor(()=>!clipboardPanel().querySelector('.activity-beam[aria-hidden=false]') && clipboardPanel().querySelector('.history-search input').value==='', 'clipboard search did not clear');
      } finally { window.fetch=internalFetch; }
      assert(loadingStyle.height==='1.5px' && loadingStyle.trackHeight==='8px', 'loading indicator is not the shared thin beam');
      if(!matchMedia('(prefers-reduced-motion: reduce)').matches) {
        assert(loadingStyle.animation==='beam-breathe' && loadingStyle.timing==='ease-in-out' && loadingStyle.opacityOnly, 'shared loading motion parameters changed');
      }
      pass(currentCase);

      currentCase = 'single record scroll owner and pinned transparency';
      await invoke('clipboard_record_update',{recordId:'portable-clipboard-sample',action:'pin'});
      await waitFor(()=>clipboardPanel().querySelector('.history-pinned [data-record-id="portable-clipboard-sample"]'),'clipboard pin fixture did not appear');
      const scroller=clipboardPanel().querySelector('.history-list'),pinned=clipboardPanel().querySelector('.history-pinned');
      assert(pinned.parentElement===scroller.parentElement && clipboardPanel().querySelector('.history-items').parentElement===scroller,'pinned and ordinary clip layout changed');
      assert(getComputedStyle(scroller).overflowY==='auto' && scroller.getBoundingClientRect().top>=pinned.getBoundingClientRect().bottom,'ordinary records are not clipped below the pinned divider');
      const pinnedStyle=getComputedStyle(pinned);
      assert(pinnedStyle.backgroundColor==='rgba(0, 0, 0, 0)','pinned background hides the shared glass');
      assert(Number(pinnedStyle.opacity)===1,'pinned row content is translucent');
      assert((pinnedStyle.backdropFilter||pinnedStyle.webkitBackdropFilter)==='none','pinned region adds a separate blur');
      await invoke('clipboard_record_update',{recordId:'portable-clipboard-sample',action:'pin'});
      pass(currentCase);

      currentCase = 'adjacent notes use measured reversible height transitions';
      const notes=['portable-note-one','portable-note-two'].map(id=>row(id));
      assert(notes.every(Boolean),'adjacent note fixtures missing');
      const noteBody=node=>node.querySelector('.clipboard-record-body');
      const hover=(node,enter)=>node.dispatchEvent(new MouseEvent(enter?'mouseover':'mouseout',{bubbles:true,relatedTarget:document.body}));
      for(const node of notes) {
        const title=node.querySelector('.clipboard-record-title'),content=node.querySelector('.clipboard-record-content');
        assert(getComputedStyle(title).position==='absolute'&&getComputedStyle(content).position==='absolute','hidden note layers still take part in layout');
        await waitFor(()=>Math.abs(noteBody(node).getBoundingClientRect().height-title.getBoundingClientRect().height)<1,'collapsed note height does not match its title');
        hover(node,true);
        await waitFor(()=>noteBody(node).dataset.expanded==='true','note original did not load');
        await waitFor(()=>Math.abs(noteBody(node).getBoundingClientRect().height-content.getBoundingClientRect().height)<1,'expanded note height does not match complete content');
        hover(node,false);
        await waitFor(()=>Math.abs(noteBody(node).getBoundingClientRect().height-title.getBoundingClientRect().height)<1,'note did not settle back to title height');
      }
      pass(currentCase);

      currentCase = 'portable clipboard layout preserves list across widths and details toggle';
      for (const width of [320, 440, 280, 520, 360]) {
        await invoke('clipboard_width_set', {width, initialize:false});
        await waitFor(() => document.querySelector('.history-divider')?.getAttribute('aria-valuenow') === String(width), 'clipboard width event did not reach React');
        await new Promise(requestAnimationFrame);
        await waitFor(() => {
          const left = document.querySelector('.history-clip').getBoundingClientRect();
          const right = document.querySelector('.translation-clip').getBoundingClientRect();
          return Math.abs(left.width-width)<1 && right.width>100 && Math.abs(left.right-right.left)<1;
        }, 'shared regions did not form a contiguous layout at width ' + width);
      }
      assert(!document.querySelector('[role=alert]'), 'application reported an error during portable regression');
      pass(currentCase);
      await invoke('plugin:event|emit', {event:'portable-test-result', payload:{passed:true,cases,error:null}});
    } catch (error) {
      await invoke('plugin:event|emit', {event:'portable-test-result', payload:{passed:false,cases,error:currentCase + ': ' + String(error)}});
    }
  })();
})();
"#;
