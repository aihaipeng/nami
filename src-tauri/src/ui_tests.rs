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
    let label = if label == "history" {
        "clipboard"
    } else {
        label
    };
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
    let label = if label == "history" {
        "clipboard"
    } else {
        label
    };
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
        record_view: crate::model::RecordView::Translation,
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
                if std::env::var("NAMI_UI_TEST_VISIBLE").as_deref() == Ok("1") {
                    let window = app.get_window("translation").ok_or("window")?;
                    let scale = window.scale_factor().map_err(|_| "scale")?;
                    if let Some(monitor) = window.current_monitor().map_err(|_| "monitor")? {
                        let area = monitor.work_area();
                        let mut frame = native_translation_frame(&window)?;
                        // Keep the sweep on screen, outside AppKit's off-screen placement constraints.
                        frame.origin.x = (area.position.x as f64 + area.size.width as f64) / scale
                            - 20.0
                            - frame.size.width;
                        unsafe {
                            let native = window
                                .ns_window()
                                .map_err(|_| "native window")?
                                .cast::<AnyObject>();
                            let _: () = objc2::msg_send![native,setFrame:frame,display:false];
                        }
                    }
                }
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
    evaluate_view_dom(app, "translation", script)
}

fn evaluate_view_dom(
    app: &AppHandle,
    label: &'static str,
    script: &str,
) -> Result<serde_json::Value, String> {
    let (send, receive) = mpsc::sync_channel(1);
    let script = format!("JSON.stringify((() => {{ {script} }})())");
    on_main(app, move |app| {
        let view = app.get_webview(label).ok_or("missing_test_webview")?;
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
        evaluate_dom(app,"const input=document.querySelector('.source-input');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(input,'');input.dispatchEvent(new Event('input',{bubbles:true}));return true;")?;
        wait_dom(
            app,
            "return {ready:document.querySelector('.source-input').value===''};",
        )?;
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
        let mut blank = serde_json::Value::Null;
        for _ in 0..40 {
            blank = on_main(app, native_chrome_sample)?;
            if blank["buttons"][1]["enabled"] == false {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
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

fn clipboard_host(app: &AppHandle) -> Result<Window, String> {
    create(app, "translation")?;
    {
        let state = app.state::<AppState>();
        state
            .windows
            .lock()
            .unwrap()
            .set_record_view(crate::model::RecordView::Clipboard);
    }
    apply_translation(app)?;
    Ok(app
        .get_webview("clipboard")
        .ok_or("missing clipboard view")?
        .window()
        .clone())
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

fn run_clipboard_capture_setting(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |app| {
            create(app, "settings")?;
            Ok(())
        })?;
        let mut ready = false;
        for _ in 0..80 {
            let value = evaluate_view_dom(app, "settings", "const button=[...document.querySelectorAll('.settings-sidebar nav button')].find(button=>button.textContent==='Clipboard');if(!button)return {ready:false};button.click();return {ready:true};")?;
            if value["ready"] == true {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if !ready {
            return Err("settings clipboard navigation did not connect".into());
        }
        let mut previous_background = None;
        for enabled in [true, false] {
            let script = format!("const input=document.querySelector('#clipboard-capture');if(!input)return {{ready:false}};if(input.disabled)throw Error('capture switch is disabled');if(input.checked!=={enabled})input.click();return {{ready:true}};");
            let mut sent = false;
            for _ in 0..80 {
                let value = evaluate_view_dom(app, "settings", &script)?;
                if value["ready"] == true {
                    sent = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            if !sent {
                return Err("missing capture switch".into());
            }
            let mut value = serde_json::Value::Null;
            for _ in 0..80 {
                value = evaluate_view_dom(app, "settings", &format!("const input=document.querySelector('#clipboard-capture');const style=getComputedStyle(input);const thumb=getComputedStyle(input,'::after');return {{ready:input.checked==={enabled},checked:input.checked,disabled:input.disabled,background:style.backgroundColor,opacity:style.opacity,transform:thumb.transform}};"))?;
                if value["ready"] == true {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            if value["ready"] != true || value["disabled"] != false || value["opacity"] != "1" {
                return Err(format!("capture switch failed to change: {value}"));
            }
            let state = app.state::<AppState>();
            if state
                .clipboard_enabled
                .load(std::sync::atomic::Ordering::SeqCst)
                != enabled
                || state
                    .database
                    .lock()
                    .unwrap()
                    .as_ref()
                    .ok_or("missing_test_database")?
                    .clipboard_capture_enabled()?
                    != enabled
            {
                return Err("capture switch did not persist through IPC".into());
            }
            if enabled && value["transform"] == "none" {
                return Err("capture switch thumb did not move".into());
            }
            if previous_background.as_ref() == Some(&value["background"]) {
                return Err("capture switch states look identical".into());
            }
            previous_background = Some(value["background"].clone());
            samples.push(value);
        }
        Ok::<(), String>(())
    })();
    DomCase {
        name: "clipboard capture setting: clickable WebView switch, visible state and durable IPC",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_clipboard_action_timing(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |app| {
            let window = clipboard_host(app)?;
            window.show().map_err(|_| "window_show")?;
            window.set_focus().map_err(|_| "window_focus")?;
            activate_application();
            app.get_webview("clipboard")
                .ok_or("missing clipboard view")?
                .set_focus()
                .map_err(|_| "window_focus")?;
            Ok(())
        })?;
        for _ in 0..100 {
            let sample = evaluate_view_dom(
                app,
                "clipboard",
                r#"
const panel=document.querySelector('.clipboard-panel');if(!panel)return {ready:false};
const row=document.createElement('article');row.className='history-row';
row.innerHTML='<button class="history-row-select">Synthetic timing fixture</button><time>12:00</time><div class="history-row-actions"><button>Copy</button></div>';
panel.append(row);const actions=row.querySelector('.history-row-actions'),time=row.querySelector('time');
const read=()=>({duration:getComputedStyle(actions).transitionDuration,delay:getComputedStyle(actions).transitionDelay,easing:getComputedStyle(actions).transitionTimingFunction,timeDuration:getComputedStyle(time).transitionDuration});
const idle=read();row.querySelector('button').focus();const focused=row.matches(':focus-within'),active=read();row.remove();return {ready:true,idle,active,focused};
"#,
            )?;
            if sample["ready"] == true {
                if sample["idle"]["duration"] != "0.15s, 0.15s, 0.15s"
                    || sample["active"]["duration"] != "0.15s"
                    || sample["focused"] != true
                {
                    return Err(format!("clipboard action timing: {sample}"));
                }
                for state in ["idle", "active"] {
                    if sample[state]["delay"] != "0s, 0s, 0s"
                        || sample[state]["easing"] != "ease-out, ease-out, ease"
                        || sample[state]["timeDuration"] != "0.15s, 0.15s"
                    {
                        return Err(format!("clipboard action transition: {sample}"));
                    }
                }
                samples.push(sample);
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Err("clipboard action test did not load".into())
    })();
    DomCase {
        name: "clipboard action timing",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_clipboard_media_layout(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let root = std::env::temp_dir().join(format!("nami-media-ui-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        std::fs::create_dir_all(root.join("Folder")).map_err(|_| "fixture")?;
        let image = root.join("Preview.JPG");
        std::fs::write(
            &image,
            include_bytes!("../../tests/fixtures/clipboard-preview.jpg"),
        )
        .map_err(|_| "fixture")?;
        let wide = root.join("Wide.png");
        std::fs::write(
            &wide,
            include_bytes!("../../tests/fixtures/clipboard-wide.png"),
        )
        .map_err(|_| "fixture")?;
        for (id, paths) in [
            ("media-wide", vec![wide]),
            ("media-image", vec![image.clone()]),
            (
                "media-files",
                vec![
                    image,
                    root.join("Folder"),
                    root.join("Missing.txt"),
                    root.join("Hidden.txt"),
                ],
            ),
        ] {
            let text = serde_json::to_string(&paths).map_err(|_| "fixture")?;
            app.state::<AppState>()
                .database
                .lock()
                .unwrap()
                .as_mut()
                .ok_or("missing test database")?
                .clipboard_insert(&nami::clipboard::ClipboardRecord {
                    id: id.into(),
                    kind: "files".into(),
                    text: text.clone(),
                    date: "2099-01-01T00:00:00Z".into(),
                    is_favorite: false,
                    is_pinned: false,
                    source_app: None,
                    image_path: None,
                    fingerprint: nami::clipboard::fingerprint("files", text.as_bytes()),
                })?;
        }
        on_main(app, |app| {
            clipboard_host(app)?.show().map_err(|_| "window_show")?;
            Ok(())
        })?;
        for _ in 0..150 {
            let sample = evaluate_view_dom(
                app,
                "clipboard",
                r#"
if(!window.__TAURI_INTERNALS__?.invoke)return {ready:false};
if(!window.__namiPreviewBudgetCheck){
  window.__namiPreviewBudgetCheck={done:false};
  Promise.all([0,2049].map(maxPixels=>window.__TAURI_INTERNALS__.invoke('clipboard_image',{recordId:'media-wide',maxPixels}).then(()=>false,error=>error==='invalid_argument'))).then(results=>{window.__namiPreviewBudgetCheck={done:true,rejected:results.every(Boolean)};});
}
if(!window.__namiPreviewBudgetCheck.done)return {ready:false};
const image=document.querySelector('[data-record-id="media-image"] .clipboard-image-preview > img');
const files=document.querySelector('[data-record-id="media-files"] .clipboard-files');
const wide=document.querySelector('[data-record-id="media-wide"] .clipboard-image-preview > img');
if(!image?.naturalWidth||!wide?.naturalWidth||!files?.querySelector('img'))return {ready:false};
const wideRect=wide.getBoundingClientRect();
const rect=image.getBoundingClientRect(),style=getComputedStyle(files),rows=[...files.querySelectorAll('.clipboard-file')];
return {ready:true,budgetRejected:window.__namiPreviewBudgetCheck.rejected,wide:{naturalWidth:wide.naturalWidth,naturalHeight:wide.naturalHeight,width:wideRect.width,height:wideRect.height,dpr:devicePixelRatio},csp:document.querySelector('meta[http-equiv="Content-Security-Policy"]')?.content??null,sourceScheme:image.src.split(':')[0],image:{width:rect.width,height:rect.height,maxHeight:getComputedStyle(image).maxHeight,ratio:image.naturalWidth/image.naturalHeight},gap:style.rowGap,rows:rows.map(row=>{const icon=row.querySelector('img,svg'),rect=icon.getBoundingClientRect(),name=row.querySelector('.clipboard-file-name');return {name:name.textContent,path:row.title,iconWidth:rect.width,iconHeight:rect.height,gap:getComputedStyle(row).columnGap,decoration:getComputedStyle(name).textDecorationLine,missing:row.dataset.missing};})};
"#,
            )?;
            if sample["ready"] == true {
                if sample["image"]["maxHeight"] != "64px"
                    || sample["image"]["height"].as_f64().unwrap_or(100.) > 64.1
                    || sample["gap"] != "4px"
                    || sample["sourceScheme"] != "blob"
                    || sample["image"]["ratio"] != 2
                    || sample["budgetRejected"] != true
                {
                    return Err(format!("media image sizing: {sample}"));
                }
                let wide = &sample["wide"];
                let required = (wide["width"].as_f64().unwrap() * wide["dpr"].as_f64().unwrap())
                    .ceil()
                    .min(2048.0);
                if wide["naturalWidth"].as_f64().unwrap() < required
                    || wide["naturalWidth"].as_f64().unwrap() > 2048.0
                    || (wide["naturalWidth"].as_f64().unwrap()
                        / wide["naturalHeight"].as_f64().unwrap()
                        - 8.0)
                        .abs()
                        > 0.02
                {
                    return Err(format!("preview resolution below display pixels: {sample}"));
                }
                let image = &sample["image"];
                if (image["width"].as_f64().unwrap() / image["height"].as_f64().unwrap()
                    - image["ratio"].as_f64().unwrap())
                .abs()
                    > 0.02
                {
                    return Err(format!("image ratio: {sample}"));
                }
                let rows = sample["rows"].as_array().ok_or("file rows")?;
                if rows.len() != 3
                    || rows[0]["name"] != "Preview.JPG"
                    || rows[1]["name"] != "Folder"
                    || rows[2]["name"] != "Missing.txt"
                    || rows[2]["decoration"] != "line-through"
                    || rows[2]["missing"] != "true"
                {
                    return Err(format!("media file list: {sample}"));
                }
                for row in rows {
                    if row["iconWidth"] != 20
                        || row["iconHeight"] != 20
                        || row["gap"] != "4px"
                        || row["path"].as_str().unwrap_or_default().is_empty()
                    {
                        return Err(format!("media file row: {sample}"));
                    }
                }
                samples.push(sample);
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Err("clipboard media did not load".into())
    })();
    let _ = std::fs::remove_dir_all(&root);
    DomCase {
        name: "clipboard media layout",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_clipboard_editor_layout(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        let paths = serde_json::to_string(&vec![std::env::temp_dir()
            .join("nami-synthetic-layout.png")
            .to_string_lossy()
            .into_owned()])
        .map_err(|_| "fixture")?;
        app.state::<AppState>()
            .database
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("missing test database")?
            .clipboard_insert(&nami::clipboard::ClipboardRecord {
                id: "editor-layout".into(),
                kind: "files".into(),
                text: paths.clone(),
                date: "2099-01-01T00:00:00Z".into(),
                is_favorite: false,
                is_pinned: false,
                source_app: None,
                image_path: None,
                fingerprint: nami::clipboard::fingerprint("files", paths.as_bytes()),
            })?;
        on_main(app, |app| {
            clipboard_host(app)?.show().map_err(|_| "window_show")?;
            app.get_webview("clipboard")
                .ok_or("missing clipboard view")?
                .set_focus()
                .map_err(|_| "window_focus")?;
            Ok(())
        })?;
        for _ in 0..100 {
            let sample = evaluate_view_dom(
                app,
                "clipboard",
                r#"
const row=document.querySelector('[data-record-id="editor-layout"]');if(!row)return {ready:false};
const card=document.querySelector('.clipboard-editor');
if(!card){const note=[...document.querySelectorAll('.clipboard-context-menu button')].find(button=>button.textContent==='Note');if(note)note.click();else row.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:20,clientY:60}));return {ready:false};}
const input=card.querySelector('input'),content=card.querySelector('textarea');if(!content||input.disabled)return {ready:false};
const labels=[...card.querySelectorAll('.clipboard-editor-label')],style=getComputedStyle(content);
return {ready:true,headings:card.querySelectorAll('h1,h2,h3').length,hints:card.querySelectorAll('.clipboard-editor-hint').length,labels:labels.map(label=>label.textContent),weights:labels.map(label=>getComputedStyle(label).fontWeight),fontStyle:style.fontStyle,contentWeight:style.fontWeight,labelStyle:getComputedStyle(labels[1]).fontStyle,value:content.value,disabled:content.disabled,readonly:content.readOnly,color:style.color,editableColor:getComputedStyle(input).color,resize:style.resize,alignment:labels.map((label,index)=>{const a=label.getBoundingClientRect(),b=[input,content][index].getBoundingClientRect();return {left:Math.abs(a.left-b.left),right:Math.abs(a.right-b.right)};})};
"#,
            )?;
            if sample["ready"] == true {
                if sample["headings"] != 0
                    || sample["hints"] != 0
                    || sample["labels"] != serde_json::json!(["Title", "Content"])
                    || sample["weights"] != serde_json::json!(["600", "600"])
                    || sample["fontStyle"] != "italic"
                    || sample["contentWeight"] != "600"
                    || sample["labelStyle"] != "normal"
                    || sample["value"] != "Non-editable content"
                    || sample["disabled"] != true
                    || sample["readonly"] != true
                    || sample["color"] == sample["editableColor"]
                    || sample["resize"] != "none"
                {
                    return Err(format!("clipboard editor styling: {sample}"));
                }
                for gap in sample["alignment"]
                    .as_array()
                    .ok_or("missing editor alignment")?
                {
                    if gap["left"].as_f64().unwrap_or(99.0) > 0.5
                        || gap["right"].as_f64().unwrap_or(99.0) > 0.5
                    {
                        return Err(format!("clipboard editor label alignment: {sample}"));
                    }
                }
                samples.push(sample);
                let operation_before_blur =
                    app.state::<AppState>().operation.load(Ordering::SeqCst);
                on_main(app, |app| {
                    // Exercise the same shared-host hide used by automatic paste.
                    hide_after_clipboard_use(app)
                })?;
                for _ in 0..80 {
                    let dismissed = evaluate_view_dom(
                        app,
                        "clipboard",
                        "return !document.querySelector('.clipboard-editor');",
                    )?;
                    if dismissed == true {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                if app.state::<AppState>().operation.load(Ordering::SeqCst) != operation_before_blur
                {
                    return Err("clipboard focus loss cancelled the automatic paste token".into());
                }
                samples.push(serde_json::json!({"automaticHidePreservesPasteToken":true}));
                on_main(app, |app| {
                    clipboard_host(app)?;
                    Ok(())
                })?;
                let restored = evaluate_view_dom(app,"clipboard", "return {editor:!!document.querySelector('.clipboard-editor'),menu:!!document.querySelector('.clipboard-context-menu'),search:document.querySelector('.history-search input').value,all:document.querySelector('[aria-label=\"Favorite clipboard records\"]').getAttribute('aria-pressed')};")?;
                if restored["editor"] != false
                    || restored["menu"] != false
                    || restored["search"] != ""
                    || restored["all"] != "false"
                {
                    return Err(format!(
                        "clipboard reopen did not restore defaults: {restored}"
                    ));
                }
                samples.push(restored);
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Err("clipboard editor did not load".into())
    })();
    DomCase {
        name: "clipboard editor layout",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn wait_records(app: &AppHandle, script: &str) -> Result<serde_json::Value, String> {
    let mut last = serde_json::Value::Null;
    for _ in 0..120 {
        last = evaluate_view_dom(app, "clipboard", script)?;
        if last["ready"] == true {
            return Ok(last);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(format!("records WebView did not settle: {last}"))
}

fn records_mode(app: &AppHandle, mode: &'static str) -> Result<(), String> {
    on_main(app, move |app| {
        let window = create(app, "translation")?;
        {
            let state = app.state::<AppState>();
            let mut state = state.windows.lock().unwrap();
            state.set_record_view(if mode == "clipboard" {
                crate::model::RecordView::Clipboard
            } else {
                crate::model::RecordView::Translation
            });
            state.records_focus += 1;
            state.history_pinned = true;
            state.translation_pinned = true;
        }
        apply_translation(app)?;
        update_translation_level(app)?;
        window.show().map_err(|_| "window_show")?;
        window.set_focus().map_err(|_| "window_focus")?;
        activate_application();
        app.get_webview("clipboard")
            .ok_or("records child")?
            .set_focus()
            .map_err(|_| "records focus")?;
        Ok(())
    })?;
    wait_motion(app)?;
    let selector = if mode == "clipboard" {
        ".clipboard-panel"
    } else {
        "#translation-history"
    };
    wait_records(app, &format!("const p=document.querySelector('{selector}');return {{ready:!!p&&p.getClientRects().length>0&&!p.closest('[hidden]')&&!!p.querySelector('.history-row')&&document.hasFocus()&&document.activeElement===p.querySelector('.history-search input')}};"))?;
    evaluate_view_dom(app,"clipboard","window.__recordsPresented=false;requestAnimationFrame(()=>requestAnimationFrame(()=>{window.__recordsPresented=true;}));return true;")?;
    wait_records(
        app,
        "return {ready:window.__recordsPresented&&document.fonts.status==='loaded'};",
    )?;
    Ok(())
}

const RECORDS_SNAPSHOT: &str = r#"
const p=[...document.querySelectorAll('.history-panel')].find(p=>p.getClientRects().length&&!p.closest('[hidden]'));
if(!p||!p.querySelector('.history-row'))return {ready:false};
window.__recordNodeIds??=new WeakMap();window.__recordNodeSequence??=0;
const nodeId=node=>{if(!window.__recordNodeIds.has(node))window.__recordNodeIds.set(node,++window.__recordNodeSequence);return window.__recordNodeIds.get(node);};
const box=e=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};};
const icon=e=>{const s=getComputedStyle(e),i=e.querySelector('.history-symbol')||e;return {symbol:i.dataset.symbol,mask:getComputedStyle(i).webkitMaskImage,color:s.color,opacity:s.opacity,...box(i)};};
const row=p.querySelector('.history-list .history-row:not([hidden])')||p.querySelector('.history-row:not([hidden])');
const search=p.querySelector('.history-search'),input=search.querySelector('input'),footer=p.querySelector('.history-footer'),scroller=p.querySelector('[role=scrollbar]');
return {ready:true,kind:p.id==='translation-history'?'translation':'clipboard',panel:nodeId(p),width:p.getBoundingClientRect().width,
rows:[...p.querySelectorAll('.history-row:not([hidden])')].map(r=>({id:r.dataset.recordId,node:nodeId(r),selected:r.classList.contains('selected'),pinned:r.classList.contains('pinned')})),
search:{...box(search),border:getComputedStyle(search).borderTopWidth,radius:getComputedStyle(search).borderRadius,shadow:getComputedStyle(search).boxShadow,font:getComputedStyle(input).fontSize,caret:getComputedStyle(input).caretColor},query:input.value,
toolbar:[...p.querySelectorAll('.history-toolbar > button')].map(b=>({...box(b),...icon(b),pressed:b.getAttribute('aria-pressed'),expanded:b.getAttribute('aria-expanded'),disabled:b.disabled})),
actions:[...row.querySelectorAll('.history-row-actions button')].map(icon),actionDuration:getComputedStyle(row.querySelector('.history-row-actions')).transitionDuration,
timeDuration:getComputedStyle(row.querySelector('time')).transitionDuration,previewFont:getComputedStyle(row.querySelector('.history-preview')).fontSize,
origin:{...box(row.querySelector('.history-origin')),fit:getComputedStyle(row.querySelector('.history-origin')).objectFit,radius:getComputedStyle(row.querySelector('.history-origin')).borderRadius},
footer:{...box(footer),star:icon(footer.querySelector('.history-favorites')),keyboard:icon(footer.querySelector('.history-shortcuts')),count:footer.querySelector('.history-record-count').textContent},
scrollbar:scroller?{...box(scroller),visible:scroller.classList.contains('visible'),transition:getComputedStyle(scroller.firstElementChild).transitionDuration}:null,
mask:getComputedStyle(p.querySelector('.history-list')).webkitMaskImage,error:document.querySelector('.global-error')?.textContent};
"#;

fn run_record_action_timing(app: &AppHandle, mode: &'static str, name: &'static str) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        records_mode(app, mode)?;
        let sample = wait_records(
            app,
            r#"
const panel=[...document.querySelectorAll('.history-panel')].find(p=>p.getClientRects().length&&!p.closest('[hidden]'));
if(!panel)return {ready:false};
const row=panel.querySelector('.history-list .history-row:not([hidden])')||panel.querySelector('.history-row');if(!row)return {ready:false};
const button=row.querySelector('.history-row-select'),actions=row.querySelector('.history-row-actions'),time=row.querySelector('time');
button.focus();
return {ready:row.matches(':focus-within'),duration:getComputedStyle(actions).transitionDuration,delay:getComputedStyle(actions).transitionDelay,timeDuration:getComputedStyle(time).transitionDuration,symbols:[...actions.querySelectorAll('.history-symbol')].map(i=>i.dataset.symbol),icons:[...actions.querySelectorAll('.history-symbol')].map(i=>({width:getComputedStyle(i).width,height:getComputedStyle(i).height})),color:getComputedStyle(actions.querySelector('button')).color,deleteColor:getComputedStyle(actions.querySelector('.history-delete')||actions.lastElementChild).color};
"#,
        )?;
        if sample["duration"] != "0.15s"
            || sample["delay"] != "0s, 0s, 0s"
            || sample["timeDuration"] != "0.15s, 0.15s"
        {
            return Err(format!("record action timing: {sample}"));
        }
        if sample["symbols"]
            .as_array()
            .ok_or("symbols")?
            .iter()
            .any(|s| s == "languages")
        {
            return Err("record hover actions retained Retranslate".into());
        }
        for icon in sample["icons"].as_array().ok_or("icons")? {
            if icon["width"] != "14px" || icon["height"] != "14px" {
                return Err(format!("icon dimensions: {icon}"));
            }
        }
        samples.push(sample);
        Ok(())
    })();
    DomCase {
        name,
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_translation_action_timing(app: &AppHandle) -> DomCase {
    run_record_action_timing(app, "translation", "translation action timing")
}

fn run_records_geometry(app: &AppHandle) -> DomCase {
    if let Err(error) = on_main(app, |app| {
        let window = create(app, "translation")?;
        if std::env::var("NAMI_UI_TEST_VISIBLE").as_deref() == Ok("1") {
            let app_state = app.state::<AppState>();
            let mut state = app_state.windows.lock().unwrap();
            state.history_pinned = true;
            state.translation_pinned = true;
            drop(state);
            window.show().map_err(|_| "window_show")?;
            window.set_always_on_top(true).map_err(|_| "window_pin")?;
        }
        Ok(())
    }) {
        return DomCase {
            name: "records geometry",
            passed: false,
            error: Some(error),
            samples: Vec::new(),
        };
    }
    let mut cases = run(app);
    if std::env::var("NAMI_UI_TEST_VISIBLE").as_deref() == Ok("1") {
        cases.extend(run_visible_paced(app));
    }
    let hits = run_resize_hit_targets(app);
    let error = cases
        .iter()
        .find(|case| !case.passed)
        .map(|case| format!("{}: {:?}", case.name, case.error))
        .or(hits.error.clone());
    DomCase {
        name: "records geometry",
        passed: error.is_none(),
        error,
        samples: vec![serde_json::json!({"geometry":cases,"hitTargets":hits})],
    }
}

fn run_unified_records(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        let record = nami::clipboard::ClipboardRecord {
            id: "unified-clipboard-one".into(),
            kind: "text".into(),
            text: "Synthetic shared record input".into(),
            date: "2099-01-01T00:00:00Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
            image_path: None,
            fingerprint: nami::clipboard::fingerprint("text", b"Synthetic shared record input"),
        };
        app.state::<AppState>()
            .database
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("isolated database")?
            .clipboard_insert(&record)?;
        let translation_record = crate::model::HistoryRecord {
            id: "unified-translation".into(),
            text: "Synthetic shared record input".into(),
            source_language: "en".into(),
            target_languages: vec!["fr".into()],
            translations: vec!["Synthetic shared output".into()],
            date: "2099-01-01T00:00:00Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
        };
        app.state::<AppState>()
            .database
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("isolated database")?
            .insert(&translation_record, "edge", || true)?;
        on_main(app, move |app| history_add(app, translation_record))?;
        records_mode(app, "clipboard")?;
        let clipboard = wait_records(app, RECORDS_SNAPSHOT)?;
        if clipboard["error"].is_string() {
            return Err(format!("clipboard IPC failed: {clipboard}"));
        }
        records_mode(app, "translation")?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"window.__fixtureQueried=false;window.__TAURI_INTERNALS__.invoke('history_query',{query:'',favoritesOnly:false}).then(()=>{window.__fixtureQueried=true;});return true;"#,
        )?;
        wait_records(
            app,
            r#"return {ready:window.__fixtureQueried&&!!document.querySelector('#translation-history [data-record-id="unified-translation"]')};"#,
        )?;
        let translation = wait_records(app, RECORDS_SNAPSHOT)?;
        if translation["error"].is_string() {
            return Err(format!("translation IPC failed: {translation}"));
        }
        evaluate_view_dom(
            app,
            "clipboard",
            r#"window.__recordsPermission={done:false};(async()=>{try{await window.__TAURI_INTERNALS__.invoke('locale_set',{locale:'zh-CN'});window.__recordsPermission={done:true,denied:false};}catch(error){window.__recordsPermission={done:true,denied:true};}})();return true;"#,
        )?;
        let permission = wait_records(
            app,
            "return {ready:window.__recordsPermission?.done,...window.__recordsPermission};",
        )?;
        if permission["denied"] != true {
            return Err("record child gained settings command permission".into());
        }
        samples.push(permission);
        for key in [
            "search",
            "actionDuration",
            "timeDuration",
            "previewFont",
            "mask",
        ] {
            if clipboard[key] != translation[key] {
                return Err(format!(
                    "record {key} differs: clipboard={}, translation={}",
                    clipboard[key], translation[key]
                ));
            }
        }
        for (a, b) in clipboard["actions"]
            .as_array()
            .ok_or("clipboard actions")?
            .iter()
            .zip(
                translation["actions"]
                    .as_array()
                    .ok_or("translation actions")?,
            )
        {
            for key in ["symbol", "mask", "color", "opacity", "width", "height"] {
                if a[key] != b[key] {
                    return Err(format!("record icon {key} differs: {a} / {b}"));
                }
            }
        }
        for (a, b) in clipboard["toolbar"]
            .as_array()
            .ok_or("clipboard toolbar")?
            .iter()
            .zip(
                translation["toolbar"]
                    .as_array()
                    .ok_or("translation toolbar")?,
            )
        {
            for key in ["x", "y", "width", "height", "symbol", "mask"] {
                if a[key] != b[key] {
                    return Err(format!("toolbar {key} differs: {a} / {b}"));
                }
            }
        }
        for key in ["x", "y", "width", "height"] {
            if clipboard["footer"][key] != translation["footer"][key] {
                return Err(format!("footer {key} differs"));
            }
        }
        for key in ["star", "keyboard"] {
            if clipboard["footer"][key] != translation["footer"][key] {
                return Err(format!("footer {key} differs"));
            }
        }
        samples.extend([clipboard.clone(), translation.clone()]);
        let baseline = on_main(app, geometry)?;
        for mode in ["clipboard", "translation", "clipboard", "translation"] {
            records_mode(app, mode)?;
            let state = wait_records(app, RECORDS_SNAPSHOT)?;
            let expected = if mode == "clipboard" {
                &clipboard
            } else {
                &translation
            };
            if state["panel"] != expected["panel"] || state["rows"] != expected["rows"] {
                return Err("mode switch replaced record nodes or selection".into());
            }
            if state["scrollbar"].is_object() && state["scrollbar"]["visible"] != false {
                return Err("mode switch revealed scrollbar".into());
            }
            let current = on_main(app, geometry)?;
            equal(
                current.window_width,
                baseline.window_width,
                "mode switch window width",
            )?;
            equal(
                current.history_width,
                baseline.history_width,
                "mode switch list width",
            )?;
        }
        let mut menus = Vec::new();
        for mode in ["clipboard", "translation"] {
            records_mode(app, mode)?;
            evaluate_view_dom(
                app,
                "clipboard",
                r#"const p=[...document.querySelectorAll('.history-panel')].find(p=>p.getClientRects().length&&!p.closest('[hidden]'));const row=p.querySelector('.history-list .history-row:not([hidden])');row.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:100,clientY:120}));return true;"#,
            )?;
            let menu = wait_records(
                app,
                r#"const menu=document.querySelector('.record-context-menu');if(!menu)return {ready:false};const style=getComputedStyle(menu);return {ready:true,background:style.backgroundColor,blur:style.webkitBackdropFilter,radius:style.borderRadius,shadow:style.boxShadow,items:[...menu.querySelectorAll('button')].filter(b=>['Favorite','Pin record','Delete'].includes(b.textContent)).map(b=>{const s=getComputedStyle(b),i=b.querySelector('.history-symbol');return {label:b.textContent,color:s.color,height:s.height,opacity:s.opacity,symbol:i.dataset.symbol,mask:getComputedStyle(i).webkitMaskImage,width:getComputedStyle(i).width};})};"#,
            )?;
            menus.push(menu);
            evaluate_view_dom(app,"clipboard","document.querySelector('.record-context-menu').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));return true;")?;
        }
        if menus[0] != menus[1] {
            return Err(format!("record menus differ: {menus:?}"));
        }
        samples.push(serde_json::json!({"sharedMenus":menus}));
        for mode in ["clipboard", "translation"] {
            records_mode(app, mode)?;
            for width in [320.0, 454.0, 380.0] {
                on_main(app, move |app| {
                    let app_state = app.state::<AppState>();
                    let mut state = app_state.windows.lock().unwrap();
                    state.history_width = width;
                    state.history_extent = width;
                    drop(state);
                    layout_translation(app, false)?;
                    let child = view_frame(app, "history")?;
                    equal(child.size.width, width, "native record child width")?;
                    Ok(())
                })?;
                let state=wait_records(app,&format!("const p=document.querySelector('{}');return {{ready:Math.abs(p.getBoundingClientRect().width-{width})<.1,width:p.getBoundingClientRect().width}};",if mode=="clipboard" {".clipboard-panel"} else {"#translation-history"}))?;
                samples.push(state);
            }
        }
        records_mode(app, "translation")?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('[data-record-id="unified-translation"] .history-row-select').click();return true;"#,
        )?;
        wait_motion(app)?;
        wait_records(
            app,
            r#"return {ready:document.querySelector('[data-record-id="unified-translation"]').classList.contains('selected')};"#,
        )?;
        wait_dom(app,"return {ready:document.querySelector('.source-input')?.value==='Synthetic shared record input'&&document.querySelector('.result-column')?.textContent==='Synthetic shared output'};")?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('[data-record-id="unified-translation"] .history-row-select').click();return true;"#,
        )?;
        on_main(app, |app| {
            if !app
                .state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .translation_visible
            {
                return Err("repeated click collapsed details".into());
            }
            Ok(())
        })?;
        evaluate_view_dom(
            app,
            "clipboard",
            "document.querySelector('#translation-history .records-details').click();return true;",
        )?;
        wait_motion(app)?;
        wait_records(
            app,
            r#"return {ready:!document.querySelector('#translation-history .history-row.selected')};"#,
        )?;
        evaluate_view_dom(app,"clipboard","document.querySelector('#translation-history .history-toolbar [aria-label=Clipboard]').click();return true;")?;
        wait_records(app,"const p=document.querySelector('.clipboard-panel');return {ready:!!p.getClientRects().length&&!p.closest('[hidden]')};")?;
        on_main(app, |app| {
            if app
                .state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .translation_visible
            {
                return Err("clipboard mode left details visible".into());
            }
            Ok(())
        })?;
        {
            let state = app.state::<AppState>();
            crate::clipboard_runtime::persist_capture(
                state
                    .database
                    .lock()
                    .unwrap()
                    .as_mut()
                    .ok_or("isolated database")?,
                &std::env::temp_dir(),
                crate::platform::ClipboardCapture {
                    version: 123,
                    payload: crate::platform::ClipboardPayload::Text(
                        "Synthetic newly captured shared clipboard record".into(),
                    ),
                    source_app: None,
                },
            )?;
        }
        crate::clipboard_runtime::changed(app)?;
        wait_records(app,"return {ready:[...document.querySelectorAll('.clipboard-panel .history-preview')].some(p=>p.textContent==='Synthetic newly captured shared clipboard record')};")?;
        let token = app.state::<AppState>().operation.load(Ordering::SeqCst);
        on_main(app, hide_after_clipboard_use)?;
        wait_motion(app)?;
        on_main(app, reveal_clipboard_failure)?;
        wait_motion(app)?;
        let recovery = on_main(app, move |app| {
            let state = app.state::<AppState>().windows.lock().unwrap().clone();
            if !state.history_visible
                || state.record_view != crate::model::RecordView::Clipboard
                || app.state::<AppState>().operation.load(Ordering::SeqCst) != token
            {
                return Err(
                    "clipboard failure recovery changed paste token or failed to restore view"
                        .into(),
                );
            }
            let view = app.get_webview("clipboard").ok_or("clipboard view")?;
            let (send, receive) = mpsc::sync_channel(1);
            view.with_webview(move |view| unsafe {
                let hidden: bool = objc2::msg_send![view.inner().cast::<AnyObject>(), isHidden];
                let _ = send.send(hidden);
            })
            .map_err(|_| "clipboard hidden state")?;
            if receive
                .try_recv()
                .map_err(|_| "clipboard visibility deferred")?
            {
                return Err("clipboard failure recovery left the child hidden".into());
            }
            Ok(serde_json::json!({"failureRecovery":true,"pasteTokenPreserved":true}))
        })?;
        samples.push(recovery);
        on_main(app, |app| {
            app.get_webview("clipboard").ok_or("clipboard child")?.emit("clipboard_use_failed",serde_json::json!({"status":"failed","copyStatus":"not_written","code":"clipboard_file_missing"})).map_err(|_|"failure event".to_owned())
        })?;
        let mut visible_failure = false;
        for _ in 0..100 {
            visible_failure=evaluate_view_dom(app,"clipboard","return document.querySelector('.record-feedback')?.textContent==='The original file was moved or deleted and cannot be restored';")?==true;
            if visible_failure {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !visible_failure {
            return Err("restored clipboard lost operation failure feedback after reset".into());
        }
        samples.push(serde_json::json!({"failureFeedbackAfterReset":true}));
        for _ in 0..100 {
            let editor = evaluate_view_dom(
                app,
                "clipboard",
                r#"
const row=document.querySelector('[data-record-id="unified-clipboard-one"]');
if(!row)return false;
const card=document.querySelector('.clipboard-editor');
if(card)return !card.querySelector('input').disabled;
const note=[...document.querySelectorAll('.clipboard-context-menu button')].find(button=>button.textContent==='Note');
if(note)note.click();else row.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:20,clientY:60}));
return false;
"#,
            )?;
            if editor == true {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
const input=document.querySelector('.clipboard-editor input');if(!input||input.disabled)throw Error('note editor did not load');
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'Unsaved synthetic note');
input.dispatchEvent(new Event('input',{bubbles:true}));return true;
"#,
        )?;
        let mut dirty = false;
        for _ in 0..100 {
            dirty = app.state::<AppState>().records_dirty.load(Ordering::SeqCst);
            if dirty {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !dirty {
            return Err("note editor did not protect unsaved changes".into());
        }
        for action in ["Cancel", "Discard"] {
            if action == "Cancel" {
                *app.state::<AppState>().selection.lock().unwrap() =
                    Some(crate::selection::SelectionContext {
                        text: "Synthetic pending selected text".into(),
                        source_app: None,
                        error: None,
                    });
                *app.state::<AppState>().selection_source.lock().unwrap() =
                    Some(serde_json::json!({"name":"Synthetic source"}));
            }
            on_main(app, |app| show(app, "translation"))?;
            let mut confirmation = false;
            for _ in 0..100 {
                confirmation=evaluate_view_dom(app,"clipboard","return !!document.querySelector('[role=alertdialog][aria-label=\"Save changes?\"]');")?==true;
                if confirmation {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if !confirmation {
                return Err("global navigation bypassed note confirmation".into());
            }
            let guarded = on_main(app, |app| {
                Ok(app.state::<AppState>().windows.lock().unwrap().record_view
                    == crate::model::RecordView::Clipboard)
            })?;
            if !guarded {
                return Err("navigation switched view before confirmation".into());
            }
            evaluate_view_dom(app,"clipboard",&format!("const dialog=document.querySelector('[role=alertdialog]');const button=[...dialog.querySelectorAll('button')].find(b=>b.textContent==='{}');if(!button)throw Error('confirmation action missing');button.click();return true;",action))?;
            if action == "Cancel" {
                let mut retained = false;
                for _ in 0..100 {
                    retained=evaluate_view_dom(app,"clipboard","return !document.querySelector('[role=alertdialog]')&&document.querySelector('.clipboard-editor input')?.value==='Unsaved synthetic note';")?==true;
                    if retained {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                if !retained {
                    return Err("Cancel did not preserve note draft".into());
                }
                if app.state::<AppState>().selection.lock().unwrap().is_some()
                    || app
                        .state::<AppState>()
                        .selection_source
                        .lock()
                        .unwrap()
                        .is_some()
                    || app
                        .state::<AppState>()
                        .records_navigation
                        .lock()
                        .unwrap()
                        .is_some()
                {
                    return Err("Cancel retained pending selection or navigation context".into());
                }
            } else {
                let mut navigated = false;
                for _ in 0..100 {
                    navigated = on_main(app, |app| {
                        let state = app.state::<AppState>().windows.lock().unwrap().clone();
                        Ok(state.record_view == crate::model::RecordView::Translation
                            && state.translation_visible
                            && !state.history_visible)
                    })?;
                    if navigated {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                if !navigated {
                    return Err("Discard did not continue pending translation summon".into());
                }
                let note = app
                    .state::<AppState>()
                    .database
                    .lock()
                    .unwrap()
                    .as_ref()
                    .ok_or("database")?
                    .clipboard_detail("unified-clipboard-one")?;
                if !note.title.is_empty() {
                    return Err("Discard unexpectedly persisted note title".into());
                }
            }
            samples.push(serde_json::json!({"noteNavigationAction":action,"guarded":true}));
        }
        for mode in ["clipboard", "translation"] {
            records_mode(app, mode)?;
            evaluate_view_dom(
                app,
                "clipboard",
                r#"const p=[...document.querySelectorAll('.history-panel')].find(p=>p.getClientRects().length&&!p.closest('[hidden]'));p.querySelector('.history-search input').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',isComposing:true,bubbles:true}));return true;"#,
            )?;
            on_main(app, |app| {
                if !app
                    .state::<AppState>()
                    .windows
                    .lock()
                    .unwrap()
                    .history_visible
                {
                    return Err("composition Escape hid workspace".into());
                }
                Ok(())
            })?;
            evaluate_view_dom(
                app,
                "clipboard",
                r#"const p=[...document.querySelectorAll('.history-panel')].find(p=>p.getClientRects().length&&!p.closest('[hidden]'));p.querySelector('.history-search input').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));return true;"#,
            )?;
            for _ in 0..100 {
                if on_main(app, |app| {
                    let app_state = app.state::<AppState>();
                    let s = app_state.windows.lock().unwrap();
                    Ok(!s.history_visible && !s.translation_visible)
                })? {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            on_main(app, |app| {
                let app_state = app.state::<AppState>();
                let s = app_state.windows.lock().unwrap();
                if s.history_visible || s.translation_visible {
                    return Err("Escape failed to hide fixed workspace".into());
                }
                Ok(())
            })?;
        }
        samples.push(serde_json::json!({"sameRenderer":true,"rapidSwitchStable":true,"menus":true,"nativeResizeChild":true,"detailsRestore":true,"escapePinnedAndComposition":true}));
        Ok(())
    })();
    DomCase {
        name: "unified records panel",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_history_pagination(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let original_database = app.state::<AppState>().database.lock().unwrap().take();
    let original_paging = app.state::<AppState>().history_page.lock().unwrap().clone();
    let original_loaded = app
        .state::<AppState>()
        .history
        .lock()
        .unwrap()
        .loaded_page(original_paging.cursor.clone());
    let result = (|| {
        let mut db = crate::database::HistoryDatabase::open(std::path::Path::new(":memory:"))?;
        for index in 0..250 {
            db.insert(
                &crate::model::HistoryRecord {
                    id: format!("pagination-{index}"),
                    text: format!("Synthetic page token-{index:04}"),
                    source_language: "en".into(),
                    target_languages: vec!["fr".into()],
                    translations: vec!["Synthetic output".into()],
                    date: "2099-01-01T00:00:00Z".into(),
                    is_favorite: false,
                    is_pinned: false,
                    source_app: None,
                },
                "edge",
                || true,
            )?;
        }
        let page = db.page(&crate::database::HistoryQuery::default(), None, 100)?;
        *app.state::<AppState>().database.lock().unwrap() = Some(db);
        {
            let app_state = app.state::<AppState>();
            let mut paging = app_state.history_page.lock().unwrap();
            *paging = Default::default();
            paging.cursor = page.cursor.clone();
            paging.cache.insert(Default::default(), page.clone());
        }
        on_main(app, move |app| {
            app.state::<AppState>()
                .history
                .lock()
                .unwrap()
                .apply_page(page, true);
            crate::history_runtime::publish_history(app)
        })?;
        records_mode(app, "translation")?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"const input=document.querySelector('#translation-history .history-search input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'Synthetic page token');input.dispatchEvent(new Event('input',{bubbles:true}));return true;"#,
        )?;
        let mut queried = false;
        for _ in 0..100 {
            queried = app
                .state::<AppState>()
                .history_page
                .lock()
                .unwrap()
                .query
                .text
                == "Synthetic page token";
            if queried {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !queried {
            return Err("pagination UI did not establish its own query projection".into());
        }
        samples.push(wait_records(app,r#"const p=document.querySelector('#translation-history');const rows=[...p.querySelectorAll('.history-row')];return {ready:rows.length===100&&!p.querySelector('[role=progressbar]'),count:rows.length,total:p.querySelector('.history-record-count').textContent};"#)?);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"const p=document.querySelector('#translation-history'),list=p.querySelector('.history-list');window.__pagingNodes=[...list.querySelectorAll('.history-row')];list.scrollTop=list.scrollHeight-list.clientHeight-100;window.__pagingTop=list.scrollTop;list.dispatchEvent(new WheelEvent('wheel',{deltaY:1,bubbles:true}));return true;"#,
        )?;
        let appended = wait_records(
            app,
            r#"const p=document.querySelector('#translation-history'),list=p.querySelector('.history-list'),rows=[...list.querySelectorAll('.history-row')];return {ready:rows.length===200&&!p.querySelector('[role=progressbar]'),count:rows.length,retained:window.__pagingNodes.every((node,index)=>rows[index]===node),scrollDelta:Math.abs(list.scrollTop-window.__pagingTop),query:p.querySelector('.history-search input').value,favorites:p.querySelector('.history-favorites').getAttribute('aria-pressed'),active:!p.closest('[hidden]'),error:p.querySelector('[role=alert]')?.textContent};"#,
        )?;
        if appended["retained"] != true || appended["scrollDelta"].as_f64().unwrap_or(99.0) > 1.0 {
            return Err(format!(
                "pagination replaced nodes or moved viewport: {appended}"
            ));
        }
        samples.push(appended);
        let direct = app
            .state::<AppState>()
            .database
            .lock()
            .unwrap()
            .as_ref()
            .ok_or("test database")?
            .page(
                &crate::database::HistoryQuery {
                    text: "token-0000".into(),
                    favorites_only: false,
                },
                None,
                100,
            )?;
        if direct.records.len() != 1 || direct.records[0].id != "pagination-0" {
            return Err(format!(
                "deep query fixture mismatch: {:?}",
                direct
                    .records
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<Vec<_>>()
            ));
        }
        evaluate_view_dom(
            app,
            "clipboard",
            r#"const input=document.querySelector('#translation-history .history-search input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'token-0000');input.dispatchEvent(new Event('input',{bubbles:true}));return true;"#,
        )?;
        let found = wait_records(
            app,
            r#"const p=document.querySelector('#translation-history'),rows=[...p.querySelectorAll('.history-row')];return {ready:rows.length===1&&rows[0].dataset.recordId==='pagination-0'&&!p.querySelector('[role=progressbar]'),total:p.querySelector('.history-record-count').textContent,query:p.querySelector('.history-search input').value,rows:rows.map(r=>r.dataset.recordId),error:p.querySelector('[role=alert]')?.textContent};"#,
        )?;
        if found["total"] != "1 record" {
            return Err(format!("deep search count: {found}"));
        }
        samples.push(found);
        Ok(())
    })();
    let result = result.map_err(|error| {
        let state = app.state::<AppState>();
        let query = state.history_page.lock().unwrap().query.clone();
        format!(
            "{error}; backend query={:?}, favorites={}",
            query.text, query.favorites_only
        )
    });
    *app.state::<AppState>().database.lock().unwrap() = original_database;
    *app.state::<AppState>().history_page.lock().unwrap() = original_paging;
    let _ = on_main(app, move |app| {
        app.state::<AppState>()
            .history
            .lock()
            .unwrap()
            .apply_page(original_loaded, true);
        crate::history_runtime::publish_history(app)
    });
    let _ = evaluate_view_dom(
        app,
        "clipboard",
        r#"const input=document.querySelector('#translation-history .history-search input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'');input.dispatchEvent(new Event('input',{bubbles:true}));return true;"#,
    );
    DomCase {
        name: "shared record child pagination IPC",
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
    if let Ok(case) = std::env::var("NAMI_UI_TEST_CASE") {
        if ![
            "clipboard-actions",
            "clipboard-editor",
            "clipboard-media",
            "translation-actions",
            "unified-records",
            "records-geometry",
            "records-pagination",
        ]
        .contains(&case.as_str())
        {
            return Err("unknown native test case".into());
        }
        let app = app.clone();
        std::thread::spawn(move || {
            let result = if case == "records-pagination" {
                run_history_pagination(&app)
            } else if case == "records-geometry" {
                run_records_geometry(&app)
            } else if case == "unified-records" {
                run_unified_records(&app)
            } else if case == "translation-actions" {
                run_translation_action_timing(&app)
            } else if case == "clipboard-actions" {
                run_clipboard_action_timing(&app)
            } else if case == "clipboard-media" {
                run_clipboard_media_layout(&app)
            } else {
                run_clipboard_editor_layout(&app)
            };
            let passed = result.passed;
            let report = serde_json::json!({"runId":run_id,"testCase":case,"passed":passed,"cases":[],"domCases":[result],"scope":"selected synthetic records or native host case; no OS mouse or material pixel verification"});
            let saved = std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap());
            app.exit(if passed && saved.is_ok() { 0 } else { 1 });
        });
        return Ok(());
    }
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
        ];
        dom_cases.push(run_history_pagination(&app));
        dom_cases.push(run_source_editing(&app));
        dom_cases.push(run_resize_hit_targets(&app));
        dom_cases.push(run_unified_records(&app));
        dom_cases.push(run_translation_action_timing(&app));
        dom_cases.push(run_clipboard_action_timing(&app));
        dom_cases.push(run_clipboard_media_layout(&app));
        dom_cases.push(run_clipboard_editor_layout(&app));
        dom_cases.push(run_clipboard_capture_setting(&app));
        let clipboard_history = on_main(&app, |_| crate::platform::test_clipboard_history());
        dom_cases.push(DomCase {
            name: "clipboard history: text, image, files, private exclusion, PNG thumbnail and missing-file protection",
            passed: clipboard_history.is_ok(),
            error: clipboard_history.err(),
            samples: Vec::new(),
        });
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
