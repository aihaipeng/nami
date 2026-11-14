use super::*;
use crate::model::WindowState;
use serde::Serialize;
use std::{sync::mpsc, time::Duration};
use tauri::Listener;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct LayerFrame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl From<objc2_foundation::NSRect> for LayerFrame {
    fn from(frame: objc2_foundation::NSRect) -> Self {
        Self {
            x: frame.origin.x,
            y: frame.origin.y,
            width: frame.size.width,
            height: frame.size.height,
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct LayerSample {
    model: LayerFrame,
    presentation: Option<LayerFrame>,
    animation_count: usize,
}

impl LayerSample {
    fn presentation_delta(self) -> Option<f64> {
        self.presentation.map(|presented| {
            [
                (presented.x - self.model.x).abs(),
                (presented.y - self.model.y).abs(),
                (presented.width - self.model.width).abs(),
                (presented.height - self.model.height).abs(),
                ((presented.x + presented.width) - (self.model.x + self.model.width)).abs(),
            ]
            .into_iter()
            .fold(0.0_f64, f64::max)
        })
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct Geometry {
    stage: &'static str,
    window_left: f64,
    window_right: f64,
    window_width: f64,
    history_x: f64,
    history_width: f64,
    translation_x: f64,
    translation_width: f64,
    history_state_width: f64,
    translation_state_width: f64,
    history_layer: Option<LayerSample>,
    translation_layer: Option<LayerSample>,
}

impl Geometry {
    fn translation_left(self) -> f64 {
        self.window_left + self.translation_x
    }

    fn translation_right(self) -> f64 {
        self.translation_left() + self.translation_width
    }

    fn adjacent(self) -> Result<(), String> {
        equal(
            self.history_x + self.history_width,
            self.translation_x,
            "sibling view seam",
        )
    }
}

#[derive(Serialize)]
struct Case {
    name: &'static str,
    passed: bool,
    error: Option<String>,
    samples: Vec<Geometry>,
}

fn equal(actual: f64, expected: f64, field: &str) -> Result<(), String> {
    if (actual - expected).abs() > 0.01 {
        Err(format!("{field}: expected {expected:.4}, got {actual:.4}"))
    } else {
        Ok(())
    }
}

fn on_main<T: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce(&AppHandle) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    if std::env::var("NAMI_UI_TEST_VISIBLE").as_deref() == Ok("1") {
        std::thread::sleep(Duration::from_secs_f64(1.0 / 60.0));
    }
    let (send, receive) = mpsc::sync_channel(1);
    let owned = app.clone();
    app.run_on_main_thread(move || {
        let _ = send.send(f(&owned));
    })
    .map_err(|error| error.to_string())?;
    receive
        .recv_timeout(Duration::from_secs(10))
        .map_err(|error| error.to_string())?
}

fn view_frame(app: &AppHandle, label: &str) -> Result<objc2_foundation::NSRect, String> {
    if label == "history" {
        let window = app.get_window("translation").ok_or("missing_test_window")?;
        let stage = translation_stage(&window)?;
        let pointer = stage.ivars().history_view.get() as *mut AnyObject;
        if pointer.is_null() {
            return Err("missing_native_history".into());
        }
        return unsafe { Ok(objc2::msg_send![pointer, frame]) };
    }
    let view = app.get_webview(label).ok_or("missing_test_webview")?;
    let (send, receive) = mpsc::sync_channel(1);
    view.with_webview(move |view| unsafe {
        let frame: objc2_foundation::NSRect =
            objc2::msg_send![view.inner().cast::<objc2::runtime::AnyObject>(), frame];
        let _ = send.send(frame);
    })
    .map_err(|error| error.to_string())?;
    // This function runs on the main thread; dispatch must have executed inline.
    receive
        .try_recv()
        .map_err(|_| "native frame read was unexpectedly deferred".into())
}

fn view_layer(app: &AppHandle, label: &str) -> Result<Option<LayerSample>, String> {
    if label == "history" {
        let window = app.get_window("translation").ok_or("missing_test_window")?;
        let stage = translation_stage(&window)?;
        let pointer = stage.ivars().history_view.get() as *mut AnyObject;
        if pointer.is_null() {
            return Err("missing_native_history".into());
        }
        return unsafe {
            let layer: *mut AnyObject = objc2::msg_send![pointer, layer];
            if layer.is_null() {
                return Ok(None);
            }
            let model: objc2_foundation::NSRect = objc2::msg_send![layer, frame];
            let presentation: *mut AnyObject = objc2::msg_send![layer, presentationLayer];
            let presented = if presentation.is_null() {
                None
            } else {
                let frame: objc2_foundation::NSRect = objc2::msg_send![presentation, frame];
                Some(frame.into())
            };
            let keys: *mut AnyObject = objc2::msg_send![layer, animationKeys];
            let animation_count: usize = if keys.is_null() {
                0
            } else {
                objc2::msg_send![keys, count]
            };
            Ok(Some(LayerSample {
                model: model.into(),
                presentation: presented,
                animation_count,
            }))
        };
    }
    let view = app.get_webview(label).ok_or("missing_test_webview")?;
    let (send, receive) = mpsc::sync_channel(1);
    view.with_webview(move |view| unsafe {
        let layer: *mut objc2::runtime::AnyObject =
            objc2::msg_send![view.inner().cast::<objc2::runtime::AnyObject>(), layer];
        let sample = if layer.is_null() {
            None
        } else {
            let model: objc2_foundation::NSRect = objc2::msg_send![layer, frame];
            let presentation: *mut objc2::runtime::AnyObject =
                objc2::msg_send![layer, presentationLayer];
            let presented = if presentation.is_null() {
                None
            } else {
                let frame: objc2_foundation::NSRect = objc2::msg_send![presentation, frame];
                Some(frame.into())
            };
            let keys: *mut objc2::runtime::AnyObject = objc2::msg_send![layer, animationKeys];
            let animation_count: usize = if keys.is_null() {
                0
            } else {
                objc2::msg_send![keys, count]
            };
            Some(LayerSample {
                model: model.into(),
                presentation: presented,
                animation_count,
            })
        };
        let _ = send.send(sample);
    })
    .map_err(|error| error.to_string())?;
    receive
        .try_recv()
        .map_err(|_| "native layer read was unexpectedly deferred".into())
}

fn geometry(app: &AppHandle) -> Result<Geometry, String> {
    let window = app.get_window("translation").ok_or("missing_test_window")?;
    let frame = native_translation_frame(&window)?;
    let h = view_frame(app, "history")?;
    let t = view_frame(app, "translation")?;
    let state = app.state::<AppState>().windows.lock().unwrap().clone();
    Ok(Geometry {
        stage: "after_command",
        window_left: frame.origin.x,
        window_right: frame.origin.x + frame.size.width,
        window_width: frame.size.width,
        history_x: h.origin.x,
        history_width: h.size.width,
        translation_x: t.origin.x,
        translation_width: t.size.width,
        history_state_width: state.history_width,
        translation_state_width: state.translation_width,
        history_layer: view_layer(app, "history")?,
        translation_layer: view_layer(app, "translation")?,
    })
}

fn set_native_width(app: &AppHandle, width: f64, fixed_right: bool) -> Result<(), String> {
    let window = app.get_window("translation").ok_or("missing_test_window")?;
    let mut frame = native_translation_frame(&window)?;
    if fixed_right {
        frame.origin.x += frame.size.width - width;
    }
    frame.size.width = width;
    unsafe {
        let native = window
            .ns_window()
            .map_err(|_| "window_geometry")?
            .cast::<objc2::runtime::AnyObject>();
        let _: () = objc2::msg_send![native, setFrame: frame, display: false];
    }
    translation_stage(&window)?.finish_layout();
    Ok(())
}

fn reset(app: &AppHandle, history: f64, translation: f64) -> Result<(), String> {
    let window = app.get_window("translation").ok_or("missing_test_window")?;
    let scale = window.scale_factor().map_err(|_| "window_geometry")?;
    let left = window
        .current_monitor()
        .map_err(|_| "window_geometry")?
        .map(|monitor| monitor.work_area().position.x as f64 / scale + 20.0)
        .unwrap_or(20.0);
    let top = window
        .outer_position()
        .map_err(|_| "window_geometry")?
        .to_logical::<f64>(scale)
        .y;
    let state = WindowState {
        history_visible: true,
        translation_visible: true,
        history_pinned: true,
        translation_pinned: true,
        history_width: history,
        history_extent: history,
        translation_width: translation,
        translation_extent: translation,
        history_width_initialized: true,
        motion_revision: 1,
        motion_anchor: Some(left + history),
        ..WindowState::default()
    };
    *app.state::<AppState>().windows.lock().unwrap() = state;
    window
        .set_min_size(Some(LogicalSize::new(1.0, 440.0)))
        .map_err(|_| "window_geometry")?;
    window
        .set_max_size::<LogicalSize<f64>>(None)
        .map_err(|_| "window_geometry")?;
    set_panel_frame(
        app,
        &window,
        1,
        (
            PanelFrame {
                left,
                history,
                translation,
            },
            PanelFrame::default(),
        ),
        top,
        620.0,
        true,
    )?;
    Ok(())
}

fn owner(app: &AppHandle, _history: bool) -> Result<(), String> {
    let geometry = {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        state.native_resizing = true;
        StageGeometry {
            mode: StageMode::NativeResize,
            history_extent: state.history_extent,
            history_width: state.history_width,
            translation_width: state.translation_width,
            history_visible: state.history_visible,
            translation_visible: state.translation_visible,
        }
    };
    let window = app.get_window("translation").ok_or("missing_test_window")?;
    update_translation_stage(&window, geometry)?;
    update_size_limits(app, &window)
}

fn record_case(
    cases: &mut Vec<Case>,
    name: &'static str,
    run: impl FnOnce(&mut Vec<Geometry>) -> Result<(), String>,
) {
    let mut samples = Vec::new();
    let result = run(&mut samples);
    cases.push(Case {
        name,
        passed: result.is_ok(),
        error: result.err(),
        samples,
    });
}

fn run(app: &AppHandle) -> Vec<Case> {
    let mut cases = Vec::new();
    record_case(
        &mut cases,
        "FinderX left outer edge: fixed history width, translation absorbs width",
        |samples| {
            let baseline = on_main(app, |app| {
                reset(app, 360.0, 700.0)?;
                owner(app, true)?;
                geometry(app)
            })?;
            for index in 0..80 {
                let t = [700.0, 800.0, 920.0, 1050.0, 740.0][index % 5];
                let pair = on_main(app, move |app| {
                    set_native_width(app, 360.0 + t, true)?;
                    let mut before = geometry(app)?;
                    before.stage = "before_callback";
                    let window = app.get_window("translation").unwrap();
                    track_translation_resize(app, &window)?;
                    let mut after = geometry(app)?;
                    after.stage = "after_callback";
                    Ok((before, after))
                })?;
                for current in [pair.0, pair.1] {
                    samples.push(current);
                    equal(current.window_right, baseline.window_right, "window right")?;
                    equal(
                        current.translation_right(),
                        baseline.translation_right(),
                        "translation screen right",
                    )?;
                    equal(current.translation_width, t, "translation width")?;
                    equal(current.history_x, 0.0, "history left")?;
                    equal(current.history_width, 360.0, "history width")?;
                    current.adjacent()?;
                }
            }
            Ok(())
        },
    );
    record_case(
        &mut cases,
        "right divider: native command locks left and preserves translation width",
        |samples| {
            let baseline = on_main(app, |app| {
                reset(app, 360.0, 700.0)?;
                geometry(app)
            })?;
            for index in 0..80 {
                let h = [280.0, 441.0, 360.0, 500.0, 320.0][index % 5];
                let current = on_main(app, move |app| {
                    resize_history(app, h, false)?;
                    geometry(app)
                })?;
                samples.push(current);
                equal(current.window_left, baseline.window_left, "window left")?;
                equal(current.translation_width, 700.0, "translation width")?;
                equal(
                    current.history_width,
                    current.history_state_width,
                    "history viewport/state width",
                )?;
                equal(
                    current.translation_x,
                    current.history_width,
                    "translation origin",
                )?;
                current.adjacent()?;
            }
            Ok(())
        },
    );
    record_case(
        &mut cases,
        "alternate left edge and divider without width accumulation",
        |samples| {
            on_main(app, |app| reset(app, 360.0, 700.0))?;
            for index in 0..40 {
                let t = [740.0, 800.0, 920.0, 700.0][index % 4];
                let (baseline, current) = on_main(app, move |app| {
                    owner(app, true)?;
                    let baseline = geometry(app)?;
                    set_native_width(app, baseline.history_width + t, true)?;
                    let window = app.get_window("translation").unwrap();
                    track_translation_resize(app, &window)?;
                    Ok((baseline, geometry(app)?))
                })?;
                samples.push(current);
                equal(
                    current.window_right,
                    baseline.window_right,
                    "left resize window right anchor",
                )?;
                current.adjacent()?;
                let (baseline, current) = on_main(app, |app| {
                    app.state::<AppState>()
                        .windows
                        .lock()
                        .unwrap()
                        .native_resizing = false;
                    let baseline = geometry(app)?;
                    resize_history(app, 360.0, false)?;
                    Ok((baseline, geometry(app)?))
                })?;
                samples.push(current);
                equal(
                    current.window_left,
                    baseline.window_left,
                    "divider left anchor",
                )?;
                equal(
                    current.translation_width,
                    baseline.translation_width,
                    "translation width after alternating",
                )?;
                current.adjacent()?;
            }
            Ok(())
        },
    );
    record_case(
        &mut cases,
        "translation right edge changes only translation width",
        |samples| {
            let baseline = on_main(app, |app| {
                reset(app, 360.0, 700.0)?;
                owner(app, false)?;
                geometry(app)
            })?;
            for t in [800.0, 1050.0, 700.0, 920.0, 740.0] {
                let pair = on_main(app, move |app| {
                    set_native_width(app, 360.0 + t, false)?;
                    let mut before = geometry(app)?;
                    before.stage = "before_callback";
                    let window = app.get_window("translation").unwrap();
                    track_translation_resize(app, &window)?;
                    let mut after = geometry(app)?;
                    after.stage = "after_callback";
                    Ok((before, after))
                })?;
                for current in [pair.0, pair.1] {
                    samples.push(current);
                    equal(current.window_left, baseline.window_left, "window left")?;
                    equal(current.history_width, 360.0, "history width")?;
                    equal(
                        current.translation_left(),
                        baseline.translation_left(),
                        "translation left",
                    )?;
                    equal(current.translation_width, t, "translation width")?;
                    current.adjacent()?;
                }
            }
            Ok(())
        },
    );
    record_case(
        &mut cases,
        "single and dual column native minimum widths",
        |samples| {
            on_main(app, |app| {
                reset(app, 300.0, 700.0)?;
                app.state::<AppState>()
                    .windows
                    .lock()
                    .unwrap()
                    .secondary_enabled = false;
                owner(app, false)
            })?;
            let single = on_main(app, |app| {
                set_native_width(app, 300.0 + 480.0, false)?;
                track_translation_resize(app, &app.get_window("translation").unwrap())?;
                geometry(app)
            })?;
            samples.push(single);
            equal(single.translation_width, 480.0, "single column minimum")?;
            let dual = on_main(app, |app| {
                app.state::<AppState>()
                    .windows
                    .lock()
                    .unwrap()
                    .secondary_enabled = true;
                let window = app.get_window("translation").unwrap();
                update_size_limits(app, &window)?;
                track_translation_resize(app, &window)?;
                geometry(app)
            })?;
            samples.push(dual);
            equal(dual.translation_width, 700.0, "dual column minimum")?;
            Ok(())
        },
    );
    record_case(
        &mut cases,
        "history toggle 40 times keeps view boundaries and stored widths",
        |samples| {
            on_main(app, |app| reset(app, 360.0, 700.0))?;
            for index in 0..40 {
                let current = on_main(app, move |app| {
                    app.state::<AppState>()
                        .windows
                        .lock()
                        .unwrap()
                        .toggle_history();
                    layout_translation(app, false)?;
                    geometry(app)
                })?;
                samples.push(current);
                equal(current.history_width, 360.0, "stored history view width")?;
                equal(
                    current.translation_width,
                    700.0,
                    "stored translation view width",
                )?;
                equal(
                    current.window_width,
                    if index % 2 == 0 { 700.0 } else { 1060.0 },
                    "visible window width",
                )?;
                current.adjacent()?;
            }
            Ok(())
        },
    );
    record_case(
        &mut cases,
        "history animation intermediate native frames preserve translation anchor",
        |samples| {
            on_main(app, |app| reset(app, 360.0, 700.0))?;
            let baseline = on_main(app, geometry)?;
            for open in [false, true, false, true] {
                for step in 0..=30 {
                    let extent = if open {
                        step as f64 * 12.0
                    } else {
                        360.0 - step as f64 * 12.0
                    };
                    let current = on_main(app, move |app| {
                        let state = app.state::<AppState>();
                        let mut state = state.windows.lock().unwrap();
                        state.history_visible = open;
                        state.motion_revision += 1;
                        state.motion_anchor = Some(baseline.translation_left());
                        let revision = state.motion_revision;
                        drop(state);
                        let window = app.get_window("translation").unwrap();
                        window
                            .set_min_size(Some(LogicalSize::new(1.0, 440.0)))
                            .map_err(|_| "window_geometry")?;
                        let scale = window.scale_factor().map_err(|_| "window_geometry")?;
                        let top = window
                            .outer_position()
                            .map_err(|_| "window_geometry")?
                            .to_logical::<f64>(scale)
                            .y;
                        set_panel_frame(
                            app,
                            &window,
                            revision,
                            (
                                PanelFrame {
                                    left: baseline.translation_left() - extent,
                                    history: extent,
                                    translation: 700.0,
                                },
                                PanelFrame::default(),
                            ),
                            top,
                            620.0,
                            false,
                        )?;
                        let mut current = geometry(app)?;
                        current.stage = "animation_intermediate";
                        Ok(current)
                    })?;
                    samples.push(current);
                    equal(
                        current.translation_left(),
                        baseline.translation_left(),
                        "animation translation screen left",
                    )?;
                    equal(
                        current.translation_right(),
                        baseline.translation_right(),
                        "animation translation screen right",
                    )?;
                    equal(
                        current.history_width,
                        360.0,
                        "animation history viewport width",
                    )?;
                    equal(
                        current.translation_width,
                        700.0,
                        "animation translation viewport width",
                    )?;
                    current.adjacent()?;
                }
            }
            Ok(())
        },
    );
    record_case(
        &mut cases,
        "shared native background covers sibling views without viewport focus outlines",
        |samples| {
            on_main(app, |app| reset(app, 360.0, 700.0))?;
            for h in [280.0, 620.0, 360.0] {
                let current = on_main(app, move |app| {
                    resize_history(app, h, false)?;
                    let window = app.get_window("translation").unwrap();
                    unsafe {
                        let native = window
                            .ns_window()
                            .map_err(|_| "window_geometry")?
                            .cast::<objc2::runtime::AnyObject>();
                        let content: *mut objc2::runtime::AnyObject =
                            objc2::msg_send![native, contentView];
                        let bounds: objc2_foundation::NSRect = objc2::msg_send![content, bounds];
                        let subviews: *mut objc2::runtime::AnyObject =
                            objc2::msg_send![content, subviews];
                        let count: usize = objc2::msg_send![subviews, count];
                        let mut found = false;
                        for index in 0..count {
                            let view: *mut objc2::runtime::AnyObject =
                                objc2::msg_send![subviews, objectAtIndex: index];
                            let is_surface: bool = objc2::msg_send![view, isKindOfClass: <TranslationSurface as objc2::ClassType>::class()];
                            if is_surface {
                                found = true;
                                let frame: objc2_foundation::NSRect = objc2::msg_send![view, frame];
                                equal(frame.origin.x, bounds.origin.x, "surface origin")?;
                                equal(frame.size.width, bounds.size.width, "surface width")?;
                                equal(frame.size.height, bounds.size.height, "surface height")?;
                                let hit: *mut objc2::runtime::AnyObject = objc2::msg_send![view, hitTest: objc2_foundation::NSPoint::new(20.0, 20.0)];
                                if !hit.is_null() {
                                    return Err("background intercepts pointer events".into());
                                }
                            }
                        }
                        if !found {
                            return Err("shared native surface missing".into());
                        }
                    }
                    if app.get_webview("history").is_some() {
                        return Err("history still uses a WebView".into());
                    }
                    {
                        let label = "translation";
                        let view = app.get_webview(label).unwrap();
                        let (send, receive) = mpsc::sync_channel(1);
                        view.with_webview(move |view| unsafe {
                            let focus: usize = objc2::msg_send![
                                view.inner().cast::<objc2::runtime::AnyObject>(),
                                focusRingType
                            ];
                            let _ = send.send(focus);
                        })
                        .map_err(|_| "window_geometry")?;
                        if receive.try_recv().map_err(|_| "focus ring read deferred")? != 1 {
                            return Err(format!("{label} viewport focus ring enabled"));
                        }
                    }
                    geometry(app)
                })?;
                samples.push(current);
                current.adjacent()?;
            }
            Ok(())
        },
    );
    record_case(
        &mut cases,
        "history-only resize and restore translation",
        |samples| {
            on_main(app, |app| {
                reset(app, 360.0, 700.0)?;
                app.state::<AppState>()
                    .windows
                    .lock()
                    .unwrap()
                    .translation_visible = false;
                layout_translation(app, false)?;
                owner(app, true)
            })?;
            let h_only = on_main(app, |app| {
                set_native_width(app, 500.0, true)?;
                track_translation_resize(app, &app.get_window("translation").unwrap())?;
                geometry(app)
            })?;
            samples.push(h_only);
            equal(h_only.window_width, 500.0, "history-only window width")?;
            equal(h_only.history_width, 500.0, "history-only view width")?;
            let restored = on_main(app, |app| {
                let state = app.state::<AppState>();
                let mut state = state.windows.lock().unwrap();
                state.translation_visible = true;
                state.native_resizing = false;
                drop(state);
                layout_translation(app, false)?;
                geometry(app)
            })?;
            samples.push(restored);
            equal(restored.history_width, 500.0, "restored history width")?;
            equal(
                restored.translation_width,
                700.0,
                "restored translation width",
            )?;
            equal(restored.window_width, 1200.0, "restored window width")?;
            restored.adjacent()
        },
    );
    for translation_visible in [true, false] {
        record_case(
            &mut cases,
            if translation_visible {
                "FinderX outer edge sweep with translation preserves history width"
            } else {
                "FinderX history-only outer edge sweep clamps history width"
            },
            |samples| {
                let baseline = on_main(app, move |app| {
                    reset(app, 360.0, 700.0)?;
                    if !translation_visible {
                        app.state::<AppState>()
                            .windows
                            .lock()
                            .unwrap()
                            .translation_visible = false;
                        layout_translation(app, false)?;
                    }
                    owner(app, true)?;
                    geometry(app)
                })?;
                for index in 0..80 {
                    let width = if translation_visible {
                        [1060.0, 1160.0, 1280.0, 1410.0][index % 4]
                    } else {
                        [280.0, 360.0, 500.0, 720.0][index % 4]
                    };
                    let current = on_main(app, move |app| {
                        set_native_width(app, width, true)?;
                        track_translation_resize(app, &app.get_window("translation").unwrap())?;
                        geometry(app)
                    })?;
                    samples.push(current);
                    equal(
                        current.window_right,
                        baseline.window_right,
                        "outer sweep fixed right",
                    )?;
                    equal(
                        current.history_width,
                        if translation_visible { 360.0 } else { width },
                        "outer sweep history width",
                    )?;
                    if translation_visible {
                        equal(
                            current.translation_width,
                            width - 360.0,
                            "outer sweep translation width",
                        )?;
                    }
                    current.adjacent()?;
                }
                Ok(())
            },
        );
    }
    cases
}

fn run_visible_paced(app: &AppHandle) -> Vec<Case> {
    let mut cases = Vec::new();
    for translation_visible in [true, false] {
        record_case(
            &mut cases,
            if translation_visible {
                "visible paced FinderX native outer resize with translation"
            } else {
                "visible paced FinderX history-only native outer resize"
            },
            |samples| {
                let baseline = on_main(app, move |app| {
                    reset(app, 360.0, 700.0)?;
                    if !translation_visible {
                        app.state::<AppState>()
                            .windows
                            .lock()
                            .unwrap()
                            .translation_visible = false;
                        layout_translation(app, false)?;
                    }
                    owner(app, true)?;
                    geometry(app)
                })?;
                let mut first_error = None;
                for index in 0..60 {
                    let phase = index as f64 * std::f64::consts::TAU / 59.0;
                    let width = if translation_visible {
                        (1240.0 + 160.0 * phase.sin()).round()
                    } else {
                        (440.0 + 160.0 * phase.sin()).round()
                    };
                    let pair = on_main(app, move |app| {
                        let before = geometry(app)?;
                        set_native_width(app, width, true)?;
                        track_translation_resize(app, &app.get_window("translation").unwrap())?;
                        Ok((before, geometry(app)?))
                    })?;
                    samples.extend([pair.0, pair.1]);
                    let presented = on_main(app, |app| {
                        let mut current = geometry(app)?;
                        current.stage = "post_presentation";
                        Ok(current)
                    })?;
                    samples.push(presented);
                    let check = (|| {
                        equal(
                            presented.window_right,
                            baseline.window_right,
                            "paced fixed window right",
                        )?;
                        presented.adjacent()?;
                        let expected_history = if translation_visible { 360.0 } else { width };
                        equal(
                            presented.history_width,
                            expected_history,
                            "paced history model width",
                        )?;
                        let history = presented
                            .history_layer
                            .and_then(|layer| layer.presentation)
                            .ok_or("history presentation unavailable")?;
                        equal(
                            history.width,
                            expected_history,
                            "paced history presentation width",
                        )?;
                        if translation_visible {
                            let translation = presented
                                .translation_layer
                                .and_then(|layer| layer.presentation)
                                .ok_or("translation presentation unavailable")?;
                            equal(
                                translation.width,
                                width - 360.0,
                                "paced translation presentation width",
                            )?;
                            equal(
                                presented.window_left + translation.x + translation.width,
                                baseline.window_right,
                                "paced translation presentation right",
                            )?;
                        }
                        Ok::<(), String>(())
                    })();
                    if first_error.is_none() {
                        first_error = check.err();
                    }
                }
                first_error.map_or(Ok(()), Err)
            },
        );
    }
    cases
}

#[derive(Serialize)]
struct DomCase {
    name: &'static str,
    passed: bool,
    error: Option<String>,
    samples: Vec<serde_json::Value>,
}

fn evaluate_dom(app: &AppHandle, script: &str) -> Result<serde_json::Value, String> {
    let (send, receive) = mpsc::sync_channel(1);
    let script = format!("JSON.stringify((() => {{ {script} }})())");
    on_main(app, move |app| {
        let view = app
            .get_webview("translation")
            .ok_or("missing_test_webview")?;
        view.with_webview(move |view| unsafe {
            let completion = block2::RcBlock::new(move |value: *mut AnyObject, error: *mut AnyObject| {
                let result = if !error.is_null() {
                    let description: Retained<objc2_foundation::NSString> = objc2::msg_send![error, description];
                    Err(format!("WKWebView evaluateJavaScript error: {description}"))
                } else if value.is_null() {
                    Err("WKWebView evaluateJavaScript returned no value".to_string())
                } else {
                    let text = &*value.cast::<objc2_foundation::NSString>();
                    serde_json::from_str(&text.to_string()).map_err(|error| error.to_string())
                };
                let _ = send.send(result);
            });
            let script = objc2_foundation::NSString::from_str(&script);
            let _: () = objc2::msg_send![view.inner().cast::<AnyObject>(), evaluateJavaScript: &*script, completionHandler: &*completion];
        })
        .map_err(|error| error.to_string())?;
        Ok(())
    })?;
    receive
        .recv_timeout(Duration::from_secs(10))
        .map_err(|error| error.to_string())?
}

fn wait_dom(app: &AppHandle, script: &str) -> Result<serde_json::Value, String> {
    let mut last = serde_json::Value::Null;
    for _ in 0..40 {
        let value = evaluate_dom(app, script)?;
        if value["ready"] == true {
            return Ok(value);
        }
        last = value;
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(format!(
        "WKWebView DOM did not reach expected state for {script}; last={last}"
    ))
}

fn native_chrome_sample(app: &AppHandle) -> Result<serde_json::Value, String> {
    let window = app.get_window("translation").ok_or("missing_test_window")?;
    let stage = translation_stage(&window)?;
    let window_frame = native_translation_frame(&window)?;
    let bounds: objc2_foundation::NSRect = unsafe { objc2::msg_send![&*stage, bounds] };
    let mut buttons = Vec::new();
    for tag in [101isize, 102, 103] {
        unsafe {
            let button: *mut AnyObject = objc2::msg_send![&*stage, viewWithTag: tag];
            if button.is_null() {
                return Err(format!("native control {tag} is missing"));
            }
            let frame: objc2_foundation::NSRect = objc2::msg_send![button, frame];
            let hidden: bool = objc2::msg_send![button, isHidden];
            let enabled: bool = objc2::msg_send![button, isEnabled];
            let title: Retained<objc2_foundation::NSString> = objc2::msg_send![button, title];
            let layer: *mut AnyObject = objc2::msg_send![button, layer];
            let presentation: Option<LayerFrame> = if layer.is_null() {
                None
            } else {
                let presented: *mut AnyObject = objc2::msg_send![layer, presentationLayer];
                if presented.is_null() {
                    None
                } else {
                    let frame: objc2_foundation::NSRect = objc2::msg_send![presented, frame];
                    Some(frame.into())
                }
            };
            buttons.push(serde_json::json!({
                "tag": tag, "x": frame.origin.x, "y": frame.origin.y,
                "width": frame.size.width, "height": frame.size.height,
                "screenRight": window_frame.origin.x + frame.origin.x + frame.size.width,
                "hidden": hidden, "enabled": enabled, "title": title.to_string(),
                "presentation": presentation,
            }));
        }
    }
    Ok(
        serde_json::json!({ "windowRight": window_frame.origin.x + window_frame.size.width,
        "height": bounds.size.height, "buttons": buttons }),
    )
}

fn click_native_chrome(app: &AppHandle, tag: isize) -> Result<(), String> {
    settle_chrome_dom(app)?;
    let (send, receive) = mpsc::sync_channel(1);
    let listener = app.listen_any("translation_chrome_action", move |event| {
        let _ = send.send(event.payload().to_owned());
    });
    let result = on_main(app, move |app| {
        let stage =
            translation_stage(&app.get_window("translation").ok_or("missing_test_window")?)?;
        unsafe {
            let button: *mut AnyObject = objc2::msg_send![&*stage, viewWithTag: tag];
            if button.is_null() {
                return Err(format!("native control {tag} is missing"));
            }
            let enabled: bool = objc2::msg_send![button, isEnabled];
            let hidden: bool = objc2::msg_send![button, isHidden];
            if hidden || !enabled {
                return Err(format!("native control {tag} is not actionable"));
            }
            // performClick runs a nested AppKit loop while Tao holds its event-handler lock.
            let enabled: bool = objc2::msg_send![button, isEnabled];
            if enabled {
                let action: objc2::runtime::Sel = objc2::msg_send![button, action];
                let target: *mut AnyObject = objc2::msg_send![button, target];
                let sent: bool = objc2::msg_send![button, sendAction: action, to: target];
                if !sent {
                    return Err("native button action was not sent".into());
                }
            }
        }
        Ok(())
    });
    app.unlisten(listener);
    result?;
    let expected = match tag {
        101 => "source",
        102 => "translate",
        103 => "actions",
        _ => return Err("invalid chrome test tag".into()),
    };
    let payload = receive
        .try_recv()
        .map_err(|_| format!("native button {tag} did not emit its action"))?;
    if payload != format!("\"{expected}\"") {
        return Err(format!("native button {tag} emitted {payload}"));
    }
    Ok(())
}

fn settle_chrome_dom(app: &AppHandle) -> Result<(), String> {
    // Native geometry notifications are asynchronous; interact after they stop,
    // rather than racing the application's intentional resize-dismiss behavior.
    evaluate_dom(app, "window.__namiTestLayoutQuietSince = performance.now(); if (!window.__namiTestLayoutObserved) { window.__namiTestLayoutObserved = true; for (const event of ['resize', 'nami:window-moved']) window.addEventListener(event, () => { window.__namiTestLayoutQuietSince = performance.now(); }); } return true;")?;
    let script = if std::env::var("NAMI_UI_TEST_VISIBLE").as_deref() == Ok("1") {
        "window.__namiTestSettled = false; requestAnimationFrame(() => requestAnimationFrame(() => { window.__namiTestSettled = true; })); return true;"
    } else {
        "window.__namiTestSettled = false; setTimeout(() => setTimeout(() => { window.__namiTestSettled = true; }, 0), 0); return true;"
    };
    evaluate_dom(app, script)?;
    wait_dom(app, "const inert = document.querySelector('.translation-clip').inert, historyWidth = getComputedStyle(document.querySelector('.translation-shell')).getPropertyValue('--history-width').trim(); return {ready: window.__namiTestSettled === true && performance.now() - window.__namiTestLayoutQuietSince >= 80 && !inert && historyWidth === '360px', settled: window.__namiTestSettled, inert, historyWidth};")?;
    Ok(())
}

fn assert_chrome_anchors(value: &serde_json::Value) -> Result<(), String> {
    let number = |value: &serde_json::Value, key: &str| {
        value[key]
            .as_f64()
            .ok_or_else(|| format!("missing chrome field {key}"))
    };
    let buttons = value["buttons"]
        .as_array()
        .ok_or("missing chrome buttons")?;
    let right = number(value, "windowRight")?;
    let source = &buttons[0];
    let translate = &buttons[1];
    let actions = &buttons[2];
    equal(
        right - number(source, "screenRight")?,
        24.0,
        "native source right inset",
    )?;
    equal(
        right - number(actions, "screenRight")?,
        24.0,
        "native actions right inset",
    )?;
    equal(
        number(value, "height")? - number(source, "y")?,
        40.0,
        "native source top inset",
    )?;
    equal(number(actions, "y")?, 0.0, "native actions bottom inset")?;
    equal(
        number(translate, "y")?,
        0.0,
        "native translate bottom inset",
    )?;
    equal(
        number(actions, "x")? - number(translate, "x")? - number(translate, "width")?,
        8.0,
        "native footer button gap",
    )?;
    for button in buttons {
        equal(number(button, "height")?, 32.0, "native control height")?;
        if std::env::var("NAMI_UI_TEST_VISIBLE").as_deref() == Ok("1") && button["hidden"] == false
        {
            let frame = button["presentation"]
                .as_object()
                .ok_or("visible native control has no presentation layer")?;
            for key in ["x", "y", "width", "height"] {
                equal(
                    frame[key].as_f64().ok_or("invalid native presentation")?,
                    number(button, key)?,
                    "native chrome presentation",
                )?;
            }
        }
    }
    Ok(())
}

fn run_native_chrome_geometry(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |app| reset(app, 360.0, 920.0))?;
        let baseline = on_main(app, native_chrome_sample)?;
        let right = baseline["windowRight"]
            .as_f64()
            .ok_or("missing baseline right")?;
        on_main(app, |app| owner(app, false))?;
        for index in 0..60 {
            let width = (360.0 + 920.0 + 180.0 * (index as f64 * 0.2).sin()).round();
            on_main(app, move |app| set_native_width(app, width, true))?;
            let sample = on_main(app, native_chrome_sample)?;
            equal(
                sample["windowRight"]
                    .as_f64()
                    .ok_or("missing window right")?,
                right,
                "chrome sweep fixed window right",
            )?;
            assert_chrome_anchors(&sample)?;
            samples.push(sample);
        }
        on_main(app, |app| {
            let window = app.get_window("translation").ok_or("missing_test_window")?;
            end_translation_resize(app, &window)
        })?;
        for width in [280.0, 720.0] {
            let sample = on_main(app, move |app| {
                let window = app.get_window("translation").ok_or("missing_test_window")?;
                {
                    let app_state = app.state::<AppState>();
                    let mut state = app_state.windows.lock().unwrap();
                    state.translation_visible = false;
                    state.history_visible = true;
                    state.history_width = width;
                    state.history_extent = width;
                }
                owner(app, true)?;
                set_native_width(app, width, true)?;
                track_translation_resize(app, &window)?;
                native_chrome_sample(app)
            })?;
            if sample["buttons"]
                .as_array()
                .ok_or("missing controls")?
                .iter()
                .any(|button| button["hidden"] != true)
            {
                return Err(
                    "native translation controls remained visible in history-only mode".into(),
                );
            }
            samples.push(sample);
        }
        on_main(app, |app| reset(app, 360.0, 920.0))?;
        Ok::<(), String>(())
    })();
    DomCase {
        name: "native controls: left-resize right anchors, presentation and history-only hiding",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_right_control_dom(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |app| reset(app, 360.0, 920.0))?;
        wait_dom(app, "return { ready: document.fonts.status === 'loaded' && document.querySelector('[data-native-chrome=true]') !== null, fonts:document.fonts.status, chrome:document.querySelector('.translation-panel')?.dataset.nativeChrome, error:document.querySelector('.global-error')?.textContent };")?;
        click_native_chrome(app, 101)?;
        wait_dom(
            app,
            "return { ready: !!document.querySelector('.language-options [role=option]') };",
        )?;
        evaluate_dom(app, "const option = [...document.querySelectorAll('.language-options [role=option]')].find(e => e.textContent === 'Chinese (Traditional)'); if (!option) throw Error('missing long source language'); option.click(); return true;")?;
        wait_dom(app, "return { ready: document.querySelector('.source-bar > .language-picker').textContent === 'Chinese (Traditional)' && !document.querySelector('.language-popover') };")?;
        for width in [700.0, 920.0, 1200.0] {
            on_main(app, move |app| reset(app, 360.0, width))?;
            samples.push(check_right_control_dom(app, width)?);
        }
        settle_chrome_dom(app)?;
        evaluate_dom(app, "document.querySelector('.translation-footer button[aria-label^=\"Secondary language:\"]').click(); return true;")?;
        wait_dom(
            app,
            "return { ready: !!document.querySelector('.language-popover[aria-label=\"Secondary language\"] .language-options [role=option]') };",
        )?;
        evaluate_dom(app, "const option = [...document.querySelectorAll('.language-options [role=option]')].find(e => e.textContent === 'None'); if (!option) throw Error('missing None language'); option.click(); return true;")?;
        wait_dom(app, "return { ready: !!document.querySelector('.translation-footer.single-column') && !document.querySelector('.language-popover') };")?;
        on_main(app, |app| {
            set_native_width(app, 360.0 + 480.0, true)?;
            track_translation_resize(app, &app.get_window("translation").unwrap())
        })?;
        samples.push(check_right_control_dom(app, 480.0)?);

        click_native_chrome(app, 101)?;
        samples.push(wait_dom(app, "const panel = document.querySelector('.language-popover'); const rect = panel?.getBoundingClientRect(); return { ready: !!panel, sourceMenuInsideViewport: !!rect && rect.left >= 7 && rect.right <= innerWidth - 7 }; ")?);
        if samples.last().unwrap()["sourceMenuInsideViewport"] != true {
            return Err("source menu extends outside the narrow viewport".into());
        }
        evaluate_dom(app, "document.querySelector('.language-popover').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); return true;")?;
        wait_dom(
            app,
            "return { ready: !document.querySelector('.language-popover') };",
        )?;
        click_native_chrome(app, 103)?;
        samples.push(wait_dom(app, "const panel = document.querySelector('.action-menu'); const rect = panel?.getBoundingClientRect(); return { ready: !!panel, actionMenuInsideViewport: !!rect && rect.left >= 7 && rect.right <= innerWidth - 7 }; ")?);
        if samples.last().unwrap()["actionMenuInsideViewport"] != true {
            return Err("actions menu extends outside the narrow viewport".into());
        }
        evaluate_dom(app, "document.querySelector('.action-menu').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); return true;")?;
        wait_dom(
            app,
            "return { ready: !document.querySelector('.action-menu') };",
        )?;
        let blank = on_main(app, native_chrome_sample)?;
        if blank["buttons"][1]["enabled"] != false {
            return Err("native Translate enabled for empty source".into());
        }
        evaluate_dom(app, "const input = document.querySelector('.source-input'); Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(input, 'x'.repeat(5001)); input.dispatchEvent(new Event('input', {bubbles:true})); return true;")?;
        let mut enabled = false;
        for _ in 0..40 {
            if on_main(app, native_chrome_sample)?["buttons"][1]["enabled"] == true {
                enabled = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if !enabled {
            return Err("native Translate did not enable after source input".into());
        }
        click_native_chrome(app, 102)?;
        samples.push(wait_dom(app, "const status = document.querySelector('.translation-status'); return { ready: !!status, translatedNotice: status?.textContent }; ")?);
        if !samples.last().unwrap()["translatedNotice"]
            .as_str()
            .unwrap_or_default()
            .contains("Input limit:")
        {
            return Err("native Translate did not invoke validated submit behavior".into());
        }
        samples.push(on_main(app, native_chrome_sample)?);
        Ok::<(), String>(())
    })();
    DomCase {
        name:
            "real WK + native controls: anchors, long source, narrow menus and native action bridge",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn check_right_control_dom(app: &AppHandle, width: f64) -> Result<serde_json::Value, String> {
    let script = format!(
        r#"
        const source = document.querySelector('.source-bar > .language-picker');
        const actions = document.querySelector('.translation-actions');
        const input = document.querySelector('.source-input');
        const s = source.getBoundingClientRect(), a = actions.getBoundingClientRect(), i = input.getBoundingClientRect();
        return {{ ready: Math.abs(innerWidth - {width}) < .01 && i.right <= s.left - 1,
            width: innerWidth, native: document.documentElement.dataset.native,
            sourcePosition: getComputedStyle(source).position, actionsPosition: getComputedStyle(actions).position,
            sourceVisibility: getComputedStyle(source).visibility, actionsVisibility: getComputedStyle(actions).visibility,
            sourceAriaHidden: source.getAttribute('aria-hidden'), actionsAriaHidden: actions.getAttribute('aria-hidden'),
            nativeChrome: document.querySelector('.translation-panel').dataset.nativeChrome,
            sourceRightInset: innerWidth - s.right, sourceTopInset: s.top,
            actionsRightInset: innerWidth - a.right, actionsBottomInset: innerHeight - a.bottom,
            inputPickerGap: s.left - i.right, sourceLabel: source.textContent }};
    "#
    );
    let value = wait_dom(app, &script)?;
    for key in ["sourcePosition", "actionsPosition"] {
        if value[key] != "fixed" {
            return Err(format!("{key} is not fixed: {}", value[key]));
        }
    }
    if value["nativeChrome"] != "true"
        || value["sourceVisibility"] != "hidden"
        || value["actionsVisibility"] != "hidden"
        || value["sourceAriaHidden"] != "true"
        || value["actionsAriaHidden"] != "true"
    {
        return Err(
            "web controls remained visible or accessible after native chrome activation".into(),
        );
    }
    let native = on_main(app, native_chrome_sample)?;
    assert_chrome_anchors(&native)?;
    if native["buttons"][0]["title"] != value["sourceLabel"] {
        return Err("native source label did not follow React selection".into());
    }
    if native["buttons"][1]["title"] != "Translate" || native["buttons"][2]["title"] != "Actions" {
        return Err("native action labels did not follow interface language".into());
    }
    if value["native"] != "true" {
        return Err("DOM test did not load the native stylesheet".into());
    }
    for (key, expected) in [
        ("sourceRightInset", 24.0),
        ("sourceTopInset", 8.0),
        ("actionsRightInset", 20.0),
        ("actionsBottomInset", 0.0),
    ] {
        equal(
            value[key].as_f64().ok_or("missing DOM inset")?,
            expected,
            key,
        )?;
    }
    Ok(value)
}

fn history_snapshot(app: &AppHandle) -> Result<serde_json::Value, String> {
    let window = app.get_window("translation").ok_or("missing_test_window")?;
    native_history::snapshot(&window)
}

fn history_object(app: &AppHandle, tag: isize) -> Result<*mut AnyObject, String> {
    let window = app.get_window("translation").ok_or("missing_test_window")?;
    let stage = translation_stage(&window)?;
    let pointer: *mut AnyObject = unsafe { objc2::msg_send![&*stage, viewWithTag: tag] };
    if pointer.is_null() {
        Err(format!("missing native history control {tag}"))
    } else {
        Ok(pointer)
    }
}

fn history_search(app: &AppHandle, query: &str) -> Result<(), String> {
    unsafe {
        let search = history_object(app, 205)?;
        let value = objc2_foundation::NSString::from_str(query);
        let _: () = objc2::msg_send![search, setStringValue: &*value];
        let root = history_object(app, 201)?;
        let _: () = objc2::msg_send![root, controlTextDidChange: std::ptr::null::<AnyObject>()];
    }
    Ok(())
}

fn history_click(app: &AppHandle, tag: isize) -> Result<(), String> {
    unsafe {
        if tag >= 1000 {
            let index = (tag - 1000) / 10;
            let row = history_object(app, 3000 + index)?;
            let _: () = objc2::msg_send![row, mouseEntered: std::ptr::null::<AnyObject>()];
        }
        let button = history_object(app, tag)?;
        // performClick runs a nested AppKit loop while Tao holds its event-handler lock.
        let enabled: bool = objc2::msg_send![button, isEnabled];
        if enabled {
            let action: objc2::runtime::Sel = objc2::msg_send![button, action];
            let target: *mut AnyObject = objc2::msg_send![button, target];
            let sent: bool = objc2::msg_send![button, sendAction: action, to: target];
            if !sent {
                return Err("native button action was not sent".into());
            }
        }
    }
    Ok(())
}

fn history_select(app: &AppHandle, index: isize) -> Result<(), String> {
    unsafe {
        let row = history_object(app, 3000 + index)?;
        let pressed: bool = objc2::msg_send![row, accessibilityPerformPress];
        if !pressed {
            return Err("native history row rejected accessibility press".into());
        }
    }
    Ok(())
}

fn wait_motion(app: &AppHandle) -> Result<(), String> {
    for _ in 0..100 {
        if on_main(app, |app| {
            Ok(app
                .state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .motion_anchor
                .is_none())
        })? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err("real animation driver did not settle".into())
}

fn assert_same_history_nodes(a: &serde_json::Value, b: &serde_json::Value) -> Result<(), String> {
    if a["rootPointer"] != b["rootPointer"] {
        return Err("history root was replaced".into());
    }
    let a = a["rows"].as_array().ok_or("missing history rows")?;
    let b = b["rows"].as_array().ok_or("missing history rows")?;
    if a.len() != b.len() || a.iter().zip(b).any(|(a, b)| a["pointer"] != b["pointer"]) {
        return Err("history row instances were rebuilt".into());
    }
    Ok(())
}

fn run_history_native_resize(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        let mut layouts = Vec::new();
        for fixed_right in [true, false] {
            let baseline = on_main(app, |app| {
                reset(app, 440.0, 920.0)?;
                app.state::<AppState>()
                    .windows
                    .lock()
                    .unwrap()
                    .translation_visible = false;
                layout_translation(app, false)?;
                owner(app, true)?;
                Ok((geometry(app)?, history_snapshot(app)?))
            })?;
            let mut edge_layouts = Vec::new();
            for index in 0..80 {
                let width = 280.0 + (index % 41) as f64 * 11.0;
                let value = on_main(app, move |app| {
                    set_native_width(app, width, fixed_right)?;
                    track_translation_resize(app, &app.get_window("translation").unwrap())?;
                    Ok((geometry(app)?, history_snapshot(app)?))
                })?;
                let (current, snapshot) = value;
                equal(
                    current.history_width,
                    width,
                    "standalone history fills width",
                )?;
                equal(current.history_x, 0.0, "standalone history origin")?;
                equal(
                    if fixed_right {
                        current.window_right
                    } else {
                        current.window_left
                    },
                    if fixed_right {
                        baseline.0.window_right
                    } else {
                        baseline.0.window_left
                    },
                    "standalone opposite edge anchor",
                )?;
                assert_same_history_nodes(&baseline.1, &snapshot)?;
                let rows = snapshot["rows"].as_array().ok_or("missing native rows")?;
                let layout = rows.iter().filter(|row| row["hidden"] == false).map(|row| {
                    let actions = row["actions"].as_array().unwrap().iter().map(|action| serde_json::json!({"x":action["x"],"y":action["y"],"width":action["width"],"height":action["height"],"drawnWidth":action["drawnWidth"],"drawnHeight":action["drawnHeight"]})).collect::<Vec<_>>();
                    serde_json::json!({"x":row["x"],"y":row["y"],"width":row["width"],"height":row["height"],"actions":actions})
                }).collect::<Vec<_>>();
                edge_layouts.push(layout);
                samples.push(
                    serde_json::json!({"fixedRight":fixed_right,"width":width,"history":snapshot}),
                );
            }
            layouts.push(edge_layouts);
            on_main(app, |app| {
                end_translation_resize(app, &app.get_window("translation").unwrap())
            })?;
        }
        if layouts[0] != layouts[1] {
            return Err("left/right standalone resizing used different internal layouts".into());
        }
        on_main(app, |app| reset(app, 360.0, 920.0))?;
        Ok::<(), String>(())
    })();
    DomCase {
        name: "native standalone history: identical left/right layouts and stable row instances",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_history_driver(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        let baseline = on_main(app, |app| {
            reset(app, 360.0, 920.0)?;
            history_search(app, "quiet")?;
            history_snapshot(app)
        })?;
        let initial_layout = baseline["layoutCount"].clone();
        for (delay, open) in [
            (0, false),
            (30, true),
            (35, false),
            (45, true),
            (30, false),
            (65, true),
        ] {
            if delay > 0 {
                std::thread::sleep(Duration::from_millis(delay));
            }
            let sample = on_main(app, move |app| {
                let before = geometry(app)?;
                app.state::<AppState>()
                    .windows
                    .lock()
                    .unwrap()
                    .history_visible = open;
                layout_translation(app, true)?;
                let after = geometry(app)?;
                equal(
                    before.history_x,
                    after.history_x,
                    "retarget keeps current history frame",
                )?;
                equal(
                    before.window_left,
                    after.window_left,
                    "retarget keeps current window origin",
                )?;
                after.adjacent()?;
                equal(
                    after.history_width,
                    360.0,
                    "animation keeps history internal width",
                )?;
                equal(
                    after.translation_width,
                    920.0,
                    "animation keeps translation internal width",
                )?;
                history_snapshot(app)
            })?;
            assert_same_history_nodes(&baseline, &sample)?;
            if sample["query"] != baseline["query"] || sample["selected"] != baseline["selected"] {
                return Err("animation discarded history query or selection".into());
            }
            if sample["layoutCount"] != initial_layout {
                return Err("origin-only animation relaid out history rows".into());
            }
            samples.push(sample);
        }
        wait_motion(app)?;
        let final_sample = on_main(app, |app| {
            let g = geometry(app)?;
            equal(g.history_x, 0.0, "latest revision ends expanded")?;
            g.adjacent()?;
            history_snapshot(app)
        })?;
        assert_same_history_nodes(&baseline, &final_sample)?;
        samples.push(final_sample);
        on_main(app, |app| {
            app.state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .translation_visible = false;
            layout_translation(app, true)
        })?;
        std::thread::sleep(Duration::from_millis(35));
        let closing = on_main(app, |app| {
            let state = app.state::<AppState>().windows.lock().unwrap().clone();
            if state.translation_extent <= 0.5 {
                return Err("translation animation had no intermediate sample".into());
            }
            let chrome = native_chrome_sample(app)?;
            if chrome["buttons"][0]["hidden"] != false {
                return Err(
                    "source native chrome disappeared before translation finished closing".into(),
                );
            }
            Ok(chrome)
        })?;
        samples.push(closing);
        wait_motion(app)?;
        let closed = on_main(app, native_chrome_sample)?;
        if closed["buttons"]
            .as_array()
            .ok_or("missing native chrome")?
            .iter()
            .any(|button| button["hidden"] != true)
        {
            return Err("translation native chrome remained after closing".into());
        }
        samples.push(closed);
        on_main(app, |app| {
            history_search(app, "")?;
            reset(app, 360.0, 920.0)
        })?;
        Ok::<(), String>(())
    })();
    DomCase {
        name: "real spring driver: rapid reversals preserve instances and closing chrome",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_history_native_actions(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |app| {
            reset(app, 360.0, 920.0)?;
            history_search(app, "")?;
            history_click(app, 202)
        })?;
        let initial = on_main(app, history_snapshot)?;
        if initial["pinnedCount"] != 2 || initial["ordinaryCount"] != 18 {
            return Err("native history did not load 20 samples with pinned records first".into());
        }
        if initial["countText"] != "20 records" || initial["shortcutsHidden"] != true {
            return Err(
                "history footer did not show the total and hide shortcuts initially".into(),
            );
        }
        for (hidden, description) in [(false, "open"), (true, "close")] {
            let footer = on_main(app, |app| {
                history_click(app, 207)?;
                history_snapshot(app)
            })?;
            if footer["shortcutsHidden"] != hidden {
                return Err(format!(
                    "history keyboard button did not {description} shortcuts"
                ));
            }
        }
        let scrollbar_top = initial["scrollbarY"]
            .as_f64()
            .ok_or("missing scrollbar position")?;
        let pinned_top = initial["pinnedY"]
            .as_f64()
            .ok_or("missing pinned position")?;
        if (scrollbar_top - pinned_top).abs() > 8.0 {
            return Err("history scrollbar does not start at the top of the pinned section".into());
        }
        for row in initial["rows"]
            .as_array()
            .ok_or("missing native history rows")?
        {
            for field in ["headingCenterY", "timeCenterY"] {
                equal(
                    row[field].as_f64().ok_or("missing heading center")?,
                    16.0,
                    "FinderX source icon and text center",
                )?;
            }
            for (actual, required) in [
                ("timeWidth", "timeRequiredWidth"),
                ("timeHeight", "timeRequiredHeight"),
            ] {
                if row[actual].as_f64().unwrap_or(0.0)
                    < row[required].as_f64().unwrap_or(f64::INFINITY)
                {
                    return Err(format!("history timestamp clipped: {}", row["id"]));
                }
            }
            for action in row["actions"]
                .as_array()
                .ok_or("missing history actions")?
                .iter()
            {
                for dimension in ["imageWidth", "imageHeight", "drawnWidth", "drawnHeight"] {
                    let size = action[dimension].as_f64().unwrap_or(0.0);
                    let expected = 14.0;
                    if size != expected {
                        return Err(format!("history symbol size {size}, expected {expected}"));
                    }
                }
            }
        }
        for button in initial["toolbar"]
            .as_array()
            .ok_or("missing history toolbar")?
        {
            for dimension in ["drawnWidth", "drawnHeight"] {
                equal(
                    button[dimension]
                        .as_f64()
                        .ok_or("missing toolbar image rectangle")?,
                    14.0,
                    "toolbar image rectangle",
                )?;
            }
        }
        samples.push(initial.clone());
        let scrolled = on_main(app, |app| unsafe {
            let row = history_object(app, 3002)?;
            let scroll: *mut AnyObject = objc2::msg_send![row, enclosingScrollView];
            let clip: *mut AnyObject = objc2::msg_send![scroll, contentView];
            let _: () =
                objc2::msg_send![clip, scrollToPoint: objc2_foundation::NSPoint::new(0.0, 64.0)];
            let _: () = objc2::msg_send![scroll, reflectScrolledClipView: clip];
            history_snapshot(app)
        })?;
        if scrolled["rows"][2]["parentPointer"] == scrolled["rows"][0]["parentPointer"] {
            return Err("pinned and ordinary records share a scroll document".into());
        }
        for (index, expected) in [(0usize, 0.0), (2, 64.0)] {
            equal(
                initial["rows"][index]["viewportY"]
                    .as_f64()
                    .ok_or("missing row viewport")?
                    - scrolled["rows"][index]["viewportY"]
                        .as_f64()
                        .ok_or("missing scrolled viewport")?,
                expected,
                "pinned record remains fixed while ordinary history scrolls",
            )?;
        }
        samples.push(scrolled);
        on_main(app, |app| history_search(app, ""))?;
        for showing in [true, false, true, false] {
            let value = on_main(app, move |app| unsafe {
                let row = history_object(app, 3002)?;
                if showing {
                    let _: () = objc2::msg_send![row, mouseEntered: std::ptr::null::<AnyObject>()];
                } else {
                    let _: () = objc2::msg_send![row, mouseExited: std::ptr::null::<AnyObject>()];
                }
                history_snapshot(app)
            })?;
            for action in value["rows"][2]["actions"]
                .as_array()
                .ok_or("missing actions")?
            {
                equal(
                    action["opacity"].as_f64().ok_or("missing opacity")?,
                    if showing { 1.0 } else { 0.0 },
                    "hover target opacity",
                )?;
                equal(
                    action["offset"].as_f64().ok_or("missing offset")?,
                    if showing { 0.0 } else { 8.0 },
                    "hover target slide",
                )?;
                if action["enabled"] != showing || action["hidden"] != false {
                    return Err("hover actions must animate without hidden toggles and disable when retracted".into());
                }
                let duration = action["animationDuration"]
                    .as_f64()
                    .ok_or("missing animation duration")?;
                if duration != 0.0 {
                    equal(
                        duration,
                        if showing { 0.14 } else { 0.1 },
                        "FinderX hover duration",
                    )?;
                }
            }
            samples.push(value);
        }
        for (query, expected) in [
            ("安静", 1),
            ("100%_literal", 1),
            ("not in synthetic examples", 0),
        ] {
            let value = on_main(app, move |app| {
                history_search(app, query)?;
                history_snapshot(app)
            })?;
            let count = value["pinnedCount"].as_u64().unwrap_or(0)
                + value["ordinaryCount"].as_u64().unwrap_or(0);
            if count != expected {
                return Err(format!(
                    "native history search {query} expected {expected}, got {count}"
                ));
            }
            samples.push(value);
        }
        let favorites = on_main(app, |app| {
            history_search(app, "")?;
            history_click(app, 203)?;
            history_snapshot(app)
        })?;
        if favorites["pinnedCount"].as_u64().unwrap_or(0)
            + favorites["ordinaryCount"].as_u64().unwrap_or(0)
            != 5
        {
            return Err("native favorites filter count".into());
        }
        samples.push(favorites);
        on_main(app, |app| history_click(app, 202))?;
        for (tag, field, expected) in [
            (1022, "favorite", true),
            (1023, "pinned", true),
            (1023, "pinned", false),
            (1022, "favorite", false),
        ] {
            let value = on_main(app, move |app| {
                history_click(app, tag)?;
                history_snapshot(app)
            })?;
            if value["rows"][2][field] != expected {
                return Err(format!("native flag {field} did not toggle"));
            }
            assert_same_history_nodes(&initial, &value)?;
            samples.push(value);
        }
        on_main(app, |app| {
            app.state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .translation_pinned = false;
            update_translation_level(app)?;
            history_select(app, 2)
        })?;
        wait_dom(app,"return {ready:document.querySelector('.source-input').value === 'The next train leaves in ten minutes.'};")?;
        wait_motion(app)?;
        let selection = on_main(app, history_snapshot)?;
        if selection["selected"] != "sample-03" {
            return Err("native selection did not synchronize".into());
        }
        samples.push(selection);
        on_main(app, |app| history_select(app, 2))?;
        for _ in 0..80 {
            if on_main(app, |app| {
                Ok(!app
                    .state::<AppState>()
                    .windows
                    .lock()
                    .unwrap()
                    .translation_visible)
            })? {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        wait_motion(app)?;
        if on_main(app, |app| {
            Ok(app
                .state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .translation_visible)
        })? {
            return Err("repeat native selection did not collapse translation".into());
        }
        on_main(app, |app| history_select(app, 2))?;
        for _ in 0..80 {
            if on_main(app, |app| {
                Ok(app
                    .state::<AppState>()
                    .windows
                    .lock()
                    .unwrap()
                    .translation_visible)
            })? {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        wait_motion(app)?;
        if !on_main(app, |app| {
            Ok(app
                .state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .translation_visible)
        })? {
            return Err("native selection did not restore translation".into());
        }
        on_main(app, |app| {
            crate::commands::select_history_sample(app, None, false)
        })?;
        if !on_main(app, history_snapshot)?["selected"].is_null() {
            return Err("native selection did not clear".into());
        }
        let exposed = on_main(app, |app| {
            unsafe {
                let row = history_object(app, 3004)?;
                let frame: objc2_foundation::NSRect = objc2::msg_send![row, frame];
                let scroll: *mut AnyObject = objc2::msg_send![row, enclosingScrollView];
                let clip: *mut AnyObject = objc2::msg_send![scroll, contentView];
                let _: () = objc2::msg_send![clip, scrollToPoint: objc2_foundation::NSPoint::new(0.0, frame.origin.y + 15.0)];
                let _: () = objc2::msg_send![scroll, reflectScrolledClipView: clip];
                history_select(app, 4)?;
                let bounds: objc2_foundation::NSRect = objc2::msg_send![clip, bounds];
                equal(
                    bounds.origin.y,
                    frame.origin.y,
                    "partially covered row scrolls minimally into view",
                )?;
            }
            history_snapshot(app)
        })?;
        samples.push(exposed);
        wait_motion(app)?;
        let protected = on_main(app, |app| {
            history_click(app, 1004)?;
            history_snapshot(app)
        })?;
        if protected["rows"][0]["id"] != "sample-01"
            || protected["rows"][0]["actions"][4]["enabled"] != false
        {
            return Err("pinned sample deletion was not protected".into());
        }
        samples.push(protected);
        let deleted = on_main(app, |app| {
            history_click(app, 1024)?;
            history_snapshot(app)
        })?;
        if deleted["ordinaryCount"] != 17 {
            return Err("unprotected sample was not deleted".into());
        }
        if deleted["countText"] != "19 records" {
            return Err("history footer did not update after deletion".into());
        }
        samples.push(deleted);
        on_main(app, |app| reset(app, 360.0, 920.0))?;
        Ok::<(), String>(())
    })();
    DomCase {
        name: "native history actions: search, flags, selection, persistence and delete protection",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_resize_hit_targets(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |app| reset(app, 360.0, 920.0))?;
        let sample = on_main(app, |app| {
            let window = app.get_window("translation").ok_or("window_unavailable")?;
            let stage = translation_stage(&window)?;
            unsafe {
                let bounds: objc2_foundation::NSRect = objc2::msg_send![&*stage, bounds];
                let divider: *mut AnyObject = objc2::msg_send![&*stage, viewWithTag: 206isize];
                for y in [12.0, bounds.size.height / 2.0, bounds.size.height - 12.0] {
                    for x in [355.0, 360.0, 365.0] {
                        let hit: *mut AnyObject =
                            objc2::msg_send![&*stage, hitTest: objc2_foundation::NSPoint::new(x,y)];
                        if hit != divider {
                            return Err(format!("seam missed at {x},{y}"));
                        }
                    }
                }
                for (x, y) in [
                    (6.0, 100.0),
                    (bounds.size.width - 6.0, 100.0),
                    (100.0, 6.0),
                    (100.0, bounds.size.height - 6.0),
                ] {
                    let hit: *mut AnyObject =
                        objc2::msg_send![&*stage, hitTest: objc2_foundation::NSPoint::new(x,y)];
                    let edge: bool = !hit.is_null()
                        && objc2::msg_send![hit, respondsToSelector: objc2::sel!(resizeDirectionAtPoint:)];
                    if !edge {
                        return Err(format!("outer edge missed at {x},{y}"));
                    }
                }
                for (x, y) in [
                    (12.0, 12.0),
                    (12.0, bounds.size.height - 12.0),
                    (bounds.size.width - 12.0, 12.0),
                    (bounds.size.width - 12.0, bounds.size.height - 12.0),
                    (360.0, 4.0),
                    (360.0, bounds.size.height - 4.0),
                ] {
                    let hit: *mut AnyObject =
                        objc2::msg_send![&*stage,hitTest:objc2_foundation::NSPoint::new(x,y)];
                    let edge: bool = !hit.is_null()
                        && objc2::msg_send![hit,respondsToSelector:objc2::sel!(resizeDirectionAtPoint:)];
                    if !edge {
                        return Err(format!("corner or outer-edge priority missed at {x},{y}"));
                    }
                }
                let root = stage.ivars().history_view.get() as *mut AnyObject;
                let wordmark: *mut AnyObject = objc2::msg_send![root,viewWithTag:208isize];
                let bounds: objc2_foundation::NSRect = objc2::msg_send![wordmark, bounds];
                let center: objc2_foundation::NSPoint = objc2::msg_send![wordmark,convertPoint:objc2_foundation::NSPoint::new(bounds.size.width/2.0,bounds.size.height/2.0),toView:&*stage];
                let hit: *mut AnyObject = objc2::msg_send![root,hitTest:center];
                if hit != root {
                    return Err("history wordmark does not route to native window drag".into());
                }
                Ok(
                    serde_json::json!({"seamSamples":9,"outerEdgeSamples":4,"cornerAndPrioritySamples":6,"wordmarkDraggable":true}),
                )
            }
        })?;
        samples.push(sample);
        for mode in 0..3 {
            samples.push(on_main(app, move |app| {
                reset(app, 360.0, 920.0)?;
                if mode != 0 {
                    let state = app.state::<AppState>();
                    if mode == 1 { state.windows.lock().unwrap().translation_visible = false; }
                    else { state.windows.lock().unwrap().history_visible = false; }
                    layout_translation(app, false)?;
                }
                let before = geometry(app)?;
                let window = app.get_window("translation").ok_or("window_unavailable")?;
                native_resize::test_drag(&window, objc2_foundation::NSPoint::new(6.0,100.0), objc2_foundation::NSPoint::new(48.0,0.0))?;
                let after = geometry(app)?;
                equal(after.window_right,before.window_right,"grab band preserves opposite edge")?;
                equal(after.window_width,before.window_width-48.0,"grab band drag changes width")?;
                if mode == 0 { equal(after.history_width,before.history_width,"grab band preserves history width")?; after.adjacent()?; }
                if mode == 1 { equal(after.history_width,after.window_width,"history-only grab band uses full width")?; }
                let stage=translation_stage(&window)?;
                unsafe {
                    let bounds: objc2_foundation::NSRect=objc2::msg_send![&*stage,bounds];
                    let hit:*mut AnyObject=objc2::msg_send![&*stage,hitTest:objc2_foundation::NSPoint::new(bounds.size.width-6.0,100.0)];
                    let edge:bool=!hit.is_null() && objc2::msg_send![hit,respondsToSelector:objc2::sel!(resizeDirectionAtPoint:)];
                    if !edge { return Err("resize hit target did not follow the new window bounds".into()); }
                    let _:()=objc2::msg_send![&*stage,resetCursorRects];
                    let _:()=objc2::msg_send![hit,resetCursorRects];
                    if mode != 0 {
                        let seam:*mut AnyObject=objc2::msg_send![&*stage,viewWithTag:206isize];
                        let hit:*mut AnyObject=objc2::msg_send![&*stage,hitTest:objc2_foundation::NSPoint::new(300.0,100.0)];
                        if hit == seam { return Err("hidden divider still captures input".into()); }
                    }
                }
                Ok(serde_json::json!({"mode":mode,"beforeWidth":before.window_width,"afterWidth":after.window_width}))
            })?);
        }
        Ok::<(), String>(())
    })();
    DomCase {
        name: "native resize hit targets: both sides of seam and all outer edges",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_source_editing(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |app| reset(app, 360.0, 920.0))?;
        evaluate_dom(app, "const input=document.querySelector('.source-input'); input.focus(); input.select(); document.execCommand('insertText', false, ''); return true;")?;
        for (text, expected) in [("a", "a"), ("b", "ab"), ("c", "abc")] {
            evaluate_dom(app, &format!("document.execCommand('insertText', false, {text:?}); window.__sourceEditReady=false; requestAnimationFrame(()=>requestAnimationFrame(()=>window.__sourceEditReady=true)); return true;"))?;
            let sample = wait_dom(app, "const input=document.querySelector('.source-input'); return {ready:window.__sourceEditReady, value:input.value, start:input.selectionStart, end:input.selectionEnd};")?;
            if sample["value"] != expected || sample["start"] != expected.len() {
                return Err(format!(
                    "sequential typing reversed or moved the caret: {sample}"
                ));
            }
            samples.push(sample);
        }
        evaluate_dom(app, "const input=document.querySelector('.source-input'); input.setSelectionRange(1,2); document.execCommand('insertText', false, 'X'); input.setSelectionRange(0,0); window.__sourceEditReady=false; requestAnimationFrame(()=>requestAnimationFrame(()=>window.__sourceEditReady=true)); return true;")?;
        let sample = wait_dom(app, "const input=document.querySelector('.source-input'); return {ready:window.__sourceEditReady, value:input.value, start:input.selectionStart, end:input.selectionEnd};")?;
        if sample["value"] != "aXc" || sample["start"] != 0 || sample["end"] != 0 {
            return Err(format!(
                "editing overwrote the newer caret position: {sample}"
            ));
        }
        samples.push(sample);
        Ok::<(), String>(())
    })();
    DomCase {
        name: "source editing: sequential text, replacement and caret ownership in WKWebView",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_runtime_history(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |app| {
            reset(app, 360.0, 920.0)?;
            history_search(app, "")?;
            history_click(app, 202)?;
            let record = crate::model::HistoryRecord {
                id: "runtime-native-test".into(),
                text: "Synthetic runtime selection".into(),
                source_language: "en".into(),
                target_languages: vec!["zh-CN".into()],
                translations: vec!["合成原生译文".into()],
                date: "2099-01-01T12:34:00+08:00".into(),
                is_favorite: false,
                is_pinned: false,
                source_app: Some(
                    serde_json::json!({"bundleId":"dev.local.nami.ui-tests", "name":"Synthetic source", "iconDataUrl":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a3ioAAAAASUVORK5CYII="}),
                ),
            };
            history_add(app, record.clone())?;
            let restored = history_record(app, &record.id).ok_or("runtime history missing")?;
            if restored.source_app != record.source_app
                || restored.translations != record.translations
            {
                return Err("runtime history content changed".into());
            }
            crate::commands::select_history_sample(app, Some(record.id), false)?;
            Ok(())
        })?;
        let dom = wait_dom(app, "const source=document.querySelector('.source-input');const result=document.querySelector('.result-column');return {ready:source?.value==='Synthetic runtime selection' && result?.textContent==='合成原生译文',source:source?.value,result:result?.textContent};")?;
        samples.push(dom);
        samples.push(on_main(app, history_snapshot)?);
        Ok::<(), String>(())
    })();
    DomCase {
        name: "runtime history: native append, source icon and WebView restoration",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

pub fn start(app: &AppHandle) -> Result<(), String> {
    if app.config().identifier != "dev.local.nami.ui-tests" {
        return Err(
            "native UI tests require the isolated dev.local.nami.ui-tests application identity"
                .into(),
        );
    }
    let path = std::env::var_os("NAMI_UI_TEST_REPORT").ok_or("test_report_path_missing")?;
    let run_id = std::env::var("NAMI_UI_TEST_RUN_ID").unwrap_or_default();
    app.state::<AppState>()
        .windows
        .lock()
        .unwrap()
        .history_width_initialized = true;
    let window = create(app, "translation")?;
    let visible = std::env::var("NAMI_UI_TEST_VISIBLE").as_deref() == Ok("1");
    if visible {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        state.history_pinned = true;
        state.translation_pinned = true;
        drop(state);
        window.show().map_err(|_| "window_show")?;
        window.set_always_on_top(true).map_err(|_| "window_pin")?;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // Allow the two local WebViews to finish their initial subscription/width setup.
        std::thread::sleep(Duration::from_millis(800));
        let mut cases = run(&app);
        if visible {
            cases.extend(run_visible_paced(&app));
        }
        let mut dom_cases = vec![
            run_native_chrome_geometry(&app),
            run_right_control_dom(&app),
            run_history_native_resize(&app),
            run_history_native_actions(&app),
        ];
        if visible {
            dom_cases.push(run_history_driver(&app));
        }
        dom_cases.push(run_runtime_history(&app));
        dom_cases.push(run_source_editing(&app));
        dom_cases.push(run_resize_hit_targets(&app));
        let clipboard = on_main(&app, |_| crate::platform::test_selection_clipboard());
        dom_cases.push(DomCase {
            name: "selection clipboard: text-only classification, all-format restore, empty restore and newer-copy protection",
            passed: clipboard.is_ok(),
            error: clipboard.err(),
            samples: Vec::new(),
        });
        let passed =
            cases.iter().all(|case| case.passed) && dom_cases.iter().all(|case| case.passed);
        let mut observed = 0usize;
        let mut missing = 0usize;
        let mut max_delta = 0.0_f64;
        let mut animated = 0usize;
        for geometry in cases.iter().flat_map(|case| &case.samples) {
            for layer in [geometry.history_layer, geometry.translation_layer] {
                match layer.and_then(LayerSample::presentation_delta) {
                    Some(delta) => {
                        observed += 1;
                        max_delta = max_delta.max(delta);
                    }
                    None => missing += 1,
                }
                if layer.is_some_and(|layer| layer.animation_count > 0) {
                    animated += 1;
                }
            }
        }
        let presentation_status = if missing > 0 {
            "unverified_missing_presentation_layers"
        } else if max_delta > 0.01 {
            "observed_model_presentation_difference"
        } else {
            "observed_no_model_presentation_difference"
        };
        eprintln!("Presentation-layer coverage: {presentation_status}; observed={observed}, missing={missing}, maxDelta={max_delta:.4}, animated={animated}. Immediate samples do not establish visible live-resize stability; next-frame observations are reported separately.");
        let paced_samples: Vec<_> = cases
            .iter()
            .flat_map(|case| &case.samples)
            .filter(|sample| sample.stage == "post_presentation")
            .collect();
        let paced_deltas: Vec<_> = paced_samples
            .iter()
            .flat_map(|sample| [sample.history_layer, sample.translation_layer])
            .flatten()
            .filter_map(LayerSample::presentation_delta)
            .collect();
        let paced_max_delta = paced_deltas.iter().copied().fold(0.0_f64, f64::max);
        let report = serde_json::json!({
            "runId": run_id,
            "passed": passed,
            "cases": cases,
            "domCases": dom_cases,
            "scope": if visible { "actual visible paced NSWindow/WKView programmatic geometry and next-frame presentation; no OS mouse drag" } else { "actual hidden NSWindow/WKView programmatic model geometry; no OS mouse drag or visual assertions" },
            "visiblePacedPresentation": {
                "enabled": visible,
                "programmaticVisibleResizeObserved": visible && !paced_samples.is_empty(),
                "postPresentationSamples": paced_samples.len(),
                "observedLayerSamples": paced_deltas.len(),
                "maxModelPresentationDelta": paced_max_delta,
                "visibleLiveResizeVerified": false,
            },
            "presentationCoverage": {
                "status": presentation_status,
                "observedLayerSamples": observed,
                "missingLayerSamples": missing,
                "maxModelPresentationDelta": max_delta,
                "animatedLayerSamples": animated,
                "visibleLiveResizeVerified": false,
            },
            "coverageGaps": [
                "OS mouse live-resize notifications and actual pointer event sequence are not simulated",
                "Hidden windows may have nil presentation layers; nil is unverified, never a presentation pass",
                "Visible WebKit remote content, material, shadow and frame pacing need visible capture tests",
            ],
        });
        let written = std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap());
        if let Err(error) = &written {
            eprintln!("Native UI report write failed: {error}");
        }
        app.exit(if passed && written.is_ok() { 0 } else { 1 });
    });
    Ok(())
}
