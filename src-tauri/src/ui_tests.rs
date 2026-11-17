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
            translation_content: state.record_view == crate::model::RecordView::Translation,
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
    wait_records_attempts(app, script, 120)
}

fn wait_records_attempts(
    app: &AppHandle,
    script: &str,
    attempts: usize,
) -> Result<serde_json::Value, String> {
    let mut last = serde_json::Value::Null;
    for _ in 0..attempts {
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
    wait_records(app, &format!("const p=document.querySelector('{selector}'),visible=!!p&&p.getClientRects().length>0&&!p.closest('[hidden]'),rows=p?.querySelectorAll('.history-row:not([hidden])').length??0,focused=document.hasFocus(),searchFocused=!!p&&document.activeElement===p.querySelector('.history-search input');return {{ready:visible&&rows>0&&focused&&searchFocused,panel:!!p,visible,rows,focused,searchFocused,activeElement:document.activeElement?.className,error:p?.querySelector('[role=alert]')?.textContent,loading:!!p?.querySelector('[role=progressbar]')}};"))?;
    evaluate_view_dom(app,"clipboard","window.__recordsPresented=false;requestAnimationFrame(()=>requestAnimationFrame(()=>{window.__recordsPresented=true;}));return true;")?;
    wait_records(
        app,
        "return {ready:window.__recordsPresented&&document.fonts.status==='loaded',presented:window.__recordsPresented,fonts:document.fonts.status};",
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
const row=p.querySelector('.history-items .history-row:not([hidden])')||p.querySelector('.history-row:not([hidden])');
const search=p.querySelector('.history-search'),input=search.querySelector('input'),footer=p.querySelector('.history-footer'),scroller=p.querySelector('[role=scrollbar]');
return {ready:true,kind:p.id==='translation-history'?'translation':'clipboard',panel:nodeId(p),width:p.getBoundingClientRect().width,
rows:[...p.querySelectorAll('.history-row:not([hidden])')].map(r=>({id:r.dataset.recordId,node:nodeId(r),selected:r.classList.contains('selected'),pinned:r.classList.contains('pinned')})),
search:{...box(search),border:getComputedStyle(search).borderTopWidth,radius:getComputedStyle(search).borderRadius,shadow:getComputedStyle(search).boxShadow,font:getComputedStyle(input).fontSize,caret:getComputedStyle(input).caretColor},query:input.value,
toolbar:[...p.querySelectorAll('.history-toolbar > button')].map(b=>({...box(b),...icon(b),pressed:b.getAttribute('aria-pressed'),expanded:b.getAttribute('aria-expanded'),disabled:b.disabled})),
actions:[...row.querySelectorAll('.history-row-actions button:not([aria-hidden="true"])')].map(icon),actionDuration:getComputedStyle(row.querySelector('.history-row-actions')).transitionDuration,
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
const row=panel.querySelector('.history-items .history-row:not([hidden])')||panel.querySelector('.history-row');if(!row)return {ready:false};
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

fn run_records_pinned(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let root = std::env::temp_dir().join(format!("nami-pinned-ui-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        std::fs::create_dir_all(&root).map_err(|_| "fixture")?;
        let image = root.join("Preview.jpg");
        std::fs::write(
            &image,
            include_bytes!("../../tests/fixtures/clipboard-preview.jpg"),
        )
        .map_err(|_| "fixture")?;
        for index in 0..24 {
            let kind = if index == 2 { "files" } else { "text" };
            let text = if kind == "files" {
                serde_json::to_string(&[&image]).map_err(|_| "fixture")?
            } else {
                format!("Synthetic pinned boundary record {index}\n").repeat(5)
            };
            app.state::<AppState>()
                .database
                .lock()
                .unwrap()
                .as_mut()
                .ok_or("isolated database")?
                .clipboard_insert(&nami::clipboard::ClipboardRecord {
                    id: format!("pinned-boundary-{index}"),
                    kind: kind.into(),
                    text: text.clone(),
                    date: format!("2099-01-01T00:00:{:02}Z", 24 - index),
                    is_favorite: false,
                    is_pinned: index < 2,
                    source_app: None,
                    image_path: None,
                    fingerprint: nami::clipboard::fingerprint(kind, text.as_bytes()),
                })?;
        }
        for mode in ["clipboard", "translation"] {
            records_mode(app, mode)?;
            if mode == "clipboard" {
                wait_records(app, "return {ready:!!document.querySelector('[data-record-id=\"pinned-boundary-2\"] img')?.naturalWidth};")?;
            }
            for top in [0, 80, 160, 100_000] {
                evaluate_view_dom(
                    app,
                    "clipboard",
                    &format!(
                        r#"
const p=[...document.querySelectorAll('.record-view:not([hidden]) .history-panel')][0];
p.querySelector('.history-list').scrollTop={top};
window.__pinnedFrame=false;requestAnimationFrame(()=>requestAnimationFrame(()=>window.__pinnedFrame=true));return true;
"#
                    ),
                )?;
                let sample = wait_records(
                    app,
                    r#"
if(!window.__pinnedFrame)return {ready:false};
const p=document.querySelector('.record-view:not([hidden]) .history-panel'),list=p.querySelector('.history-list'),pinned=p.querySelector('.history-pinned'),items=p.querySelector('.history-items');
const a=p.querySelector('.history-viewport').getBoundingClientRect(),b=pinned.getBoundingClientRect(),c=list.getBoundingClientRect(),style=getComputedStyle(pinned),line=getComputedStyle(pinned,'::after');
const rows=[...pinned.querySelectorAll('.history-row:not([hidden])')],first=rows[0].getBoundingClientRect(),last=rows.at(-1).getBoundingClientRect();
const ys=[a.top+1,first.bottom+1,last.bottom+1,b.bottom-1];
return {ready:rows.length>1,mode:p.id==='translation-history'?'translation':'clipboard',scroll:list.scrollTop,maximum:list.scrollHeight-list.clientHeight,topGap:b.top-a.top,leftGap:b.left-a.left,rightGap:a.right-b.right,
covered:ys.map(y=>pinned.contains(document.elementFromPoint(a.left+a.width/2,y))),
position:style.position,background:style.backgroundColor,ordinaryBackground:getComputedStyle(items.querySelector('.history-row')).backgroundColor,opacity:style.opacity,blur:style.webkitBackdropFilter,clipped:c.top>=b.bottom&&getComputedStyle(list).overflowY==='auto',
borders:rows.map(row=>getComputedStyle(row).borderTopColor),line:{content:line.content,height:line.height,bottom:line.bottom,gradient:line.backgroundImage},
singleOwner:pinned.parentElement===list.parentElement&&items.parentElement===list,fullTrack:Math.abs(p.querySelector('.history-scrollbar').getBoundingClientRect().height-a.height)<1};
"#,
                )?;
                samples.push(sample.clone());
                if sample["topGap"].as_f64().unwrap_or(100.0) > 0.5
                    || sample["leftGap"].as_f64().unwrap_or(100.0) > 0.5
                    || sample["rightGap"].as_f64().unwrap_or(100.0) > 0.5
                    || sample["covered"]
                        .as_array()
                        .is_none_or(|hits| hits.iter().any(|hit| hit != true))
                {
                    return Err(format!(
                        "{mode}: ordinary text or images can escape around pinned rows: {sample}"
                    ));
                }
                if sample["opacity"] != "1"
                    || sample["blur"] != "none"
                    || sample["background"] != "rgba(0, 0, 0, 0)"
                    || sample["ordinaryBackground"] != sample["background"]
                {
                    return Err(format!(
                        "{mode}: pinned region no longer shares the ordinary glass background: {sample}"
                    ));
                }
                if sample["position"] != "relative"
                    || sample["clipped"] != true
                    || sample["singleOwner"] != true
                    || sample["fullTrack"] != true
                {
                    return Err(format!(
                        "{mode}: pinned or scrollbar geometry changed: {sample}"
                    ));
                }
                if sample["borders"]
                    .as_array()
                    .is_none_or(|colors| colors.iter().any(|color| color != "rgba(0, 0, 0, 0)"))
                {
                    return Err(format!(
                        "{mode}: pinned row still has a visible border: {sample}"
                    ));
                }
                if sample["line"]["height"] != "1px"
                    || sample["line"]["bottom"] != "0px"
                    || !sample["line"]["gradient"]
                        .as_str()
                        .unwrap_or_default()
                        .starts_with("linear-gradient(to right,")
                    || !sample["line"]["gradient"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("0.28) 25%")
                    || !sample["line"]["gradient"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("0.28) 75%")
                {
                    return Err(format!(
                        "{mode}: pinned divider is missing or misplaced: {sample}"
                    ));
                }
            }
            evaluate_view_dom(
                app,
                "clipboard",
                r#"
const p=document.querySelector('.record-view:not([hidden]) .history-panel'),list=p.querySelector('.history-list');
list.scrollTop=80;requestAnimationFrame(()=>requestAnimationFrame(()=>p.querySelector('.history-pinned').style.height=`${p.querySelector('.history-viewport').clientHeight+100}px`));return true;
"#,
            )?;
            let overflow = wait_records(
                app,
                r#"
const p=document.querySelector('.record-view:not([hidden]) .history-panel'),list=p.querySelector('.history-list'),pinned=p.querySelector('.history-pinned');
return {ready:pinned.dataset.overflow==='true',scroll:list.scrollTop,position:getComputedStyle(pinned).position,singleOwner:list.contains(pinned)&&p.querySelectorAll('.history-list').length===1};
"#,
            )?;
            if overflow["position"] != "relative"
                || overflow["singleOwner"] != true
                || overflow["scroll"] != 80
            {
                return Err(format!("oversized pins became inaccessible: {overflow}"));
            }
            samples.push(overflow);
            evaluate_view_dom(
                app,
                "clipboard",
                r#"
const p=document.querySelector('.record-view:not([hidden]) .history-panel');p.querySelector('.history-pinned').style.height=`${p.querySelector('.history-viewport').clientHeight-1}px`;return true;
"#,
            )?;
            let narrow = wait_records(
                app,
                r#"
const p=document.querySelector('.record-view:not([hidden]) .history-panel'),list=p.querySelector('.history-list');
return {ready:p.querySelector('.history-pinned').dataset.overflow==='true',singleOwner:list.contains(p.querySelector('.history-pinned'))};
"#,
            )?;
            if narrow["singleOwner"] != true {
                return Err(format!(
                    "near-full pins clipped the ordinary viewport: {narrow}"
                ));
            }
            samples.push(narrow);
            evaluate_view_dom(app, "clipboard", "document.querySelector('.record-view:not([hidden]) .history-pinned').style.height='';return true;")?;
            let restored = wait_records(
                app,
                r#"
const p=document.querySelector('.record-view:not([hidden]) .history-panel'),list=p.querySelector('.history-list');
return {ready:p.querySelector('.history-pinned').dataset.overflow==='false',scroll:list.scrollTop};
"#,
            )?;
            if restored["scroll"] != 80 {
                return Err(format!("scroll owner change lost position: {restored}"));
            }
            samples.push(restored);

            evaluate_view_dom(
                app,
                "clipboard",
                r#"
window.__unpinnedAll=false;(async()=>{
const p=document.querySelector('.record-view:not([hidden]) .history-panel'),translation=p.id==='translation-history';
p.querySelector('.history-list').scrollTop=0;await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
const ids=[...p.querySelectorAll('.history-pinned .history-row:not([hidden])')].map(row=>row.dataset.recordId);
for(const recordId of ids)await window.__TAURI_INTERNALS__.invoke(translation?'history_record_update':'clipboard_record_update',{recordId,action:'pin'});
window.__unpinnedAll=true;})();return true;
"#,
            )?;
            let empty = wait_records(
                app,
                r#"
const p=document.querySelector('.record-view:not([hidden]) .history-panel'),pinned=p.querySelector('.history-pinned');
return {ready:window.__unpinnedAll&&pinned.classList.contains('empty'),divider:getComputedStyle(pinned,'::after').content,height:pinned.offsetHeight};
"#,
            )?;
            if empty["divider"] != "none" || empty["height"] != 0 {
                return Err(format!(
                    "{mode}: empty pinned section left a divider or gap: {empty}"
                ));
            }
            samples.push(empty);
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(root);
    DomCase {
        name: "shared pinned boundary coverage",
        passed: result.is_ok(),
        error: result.err(),
        samples,
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
        for index in 0..200 {
            let mut record = translation_record.clone();
            record.id = format!("selection-load-{index}");
            record.date = "2098-01-01T00:00:00Z".into();
            record.text = "Synthetic long history text ".repeat(1000);
            record.translations = vec!["Synthetic long result ".repeat(1000)];
            record.source_app = Some(
                serde_json::json!({"name":"Synthetic source","iconDataUrl":format!("data:image/png;base64,{}", "A".repeat(65536))}),
            );
            app.state::<AppState>()
                .database
                .lock()
                .unwrap()
                .as_mut()
                .ok_or("isolated database")?
                .insert(&record, "edge", || true)?;
        }
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
            r#"window.__fixtureQueried=false;(async()=>{await window.__TAURI_INTERNALS__.invoke('history_query',{query:'',favoritesOnly:false});await window.__TAURI_INTERNALS__.invoke('history_load_more');await window.__TAURI_INTERNALS__.invoke('history_load_more');window.__fixtureQueried=true;})();return true;"#,
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
        let clipboard_actions = clipboard["actions"]
            .as_array()
            .ok_or("clipboard actions")?
            .iter()
            .filter(|action| {
                !["languages", "ocr", "note"]
                    .contains(&action["symbol"].as_str().unwrap_or_default())
            })
            .collect::<Vec<_>>();
        let translation_actions = translation["actions"]
            .as_array()
            .ok_or("translation actions")?;
        if clipboard_actions.len() != translation_actions.len()
            || !clipboard["actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|action| action["symbol"] == "note")
        {
            return Err(format!(
                "shared record actions or clipboard note missing: {clipboard} / {translation}"
            ));
        }
        for (a, b) in clipboard_actions.into_iter().zip(translation_actions) {
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
        if clipboard["footer"]["keyboard"] != translation["footer"]["keyboard"] {
            return Err("footer keyboard differs".into());
        }
        for key in ["y", "width", "height", "symbol", "mask", "color", "opacity"] {
            if clipboard["footer"]["star"][key] != translation["footer"]["star"][key] {
                return Err(format!(
                    "footer favorite {key} differs: {clipboard} / {translation}"
                ));
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
                r#"const p=[...document.querySelectorAll('.history-panel')].find(p=>p.getClientRects().length&&!p.closest('[hidden]'));const row=p.querySelector('.history-items .history-row:not([hidden])');row.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:100,clientY:120}));return true;"#,
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
        evaluate_dom(
            app,
            r#"window.__selectionTiming={};const timing=window.__selectionTiming;const poll=()=>{const clip=document.querySelector('.translation-clip');if(document.querySelector('.source-input')?.value==='Synthetic shared record input'&&document.querySelector('.result-column')?.textContent==='Synthetic shared output'&&clip?.getAttribute('aria-hidden')==='false'){timing.readyAt=Date.now();}else requestAnimationFrame(poll);};requestAnimationFrame(poll);return true;"#,
        )?;
        let selection_broadcasts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let broadcasts = selection_broadcasts.clone();
        let selection_listener = app.listen_any("history_changed", move |_| {
            broadcasts.fetch_add(1, Ordering::SeqCst);
        });
        evaluate_view_dom(
            app,
            "clipboard",
            r#"window.__selectionStarted=Date.now();document.querySelector('[data-record-id="unified-translation"] .history-row-select').click();return true;"#,
        )?;
        let selection_timing = wait_dom(
            app,
            "return {ready:!!window.__selectionTiming.readyAt,...window.__selectionTiming};",
        )?;
        let selection_started =
            evaluate_view_dom(app, "clipboard", "return window.__selectionStarted;")?;
        let elapsed = selection_timing["readyAt"]
            .as_i64()
            .ok_or("selection ready timestamp")?
            - selection_started
                .as_i64()
                .ok_or("selection start timestamp")?;
        app.unlisten(selection_listener);
        let broadcasts = selection_broadcasts.load(Ordering::SeqCst);
        samples.push(serde_json::json!({"selectionReadyMs":elapsed,"selectionFullPageBroadcasts":broadcasts}));
        if broadcasts != 0 {
            return Err(format!(
                "selection rebroadcast {broadcasts} complete history pages before opening"
            ));
        }
        if elapsed > 150 {
            return Err(format!(
                "selected translation was not ready during opening: {elapsed}ms"
            ));
        }
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
            let presentation = app
                .state::<AppState>()
                .selection_revision
                .fetch_add(1, Ordering::SeqCst)
                + 1;
            *app.state::<AppState>().selection.lock().unwrap() =
                Some(crate::selection::SelectionContext {
                    text: "Synthetic pending selected text".into(),
                    presentation_id: Some(presentation),
                    source_app: None,
                    error: None,
                });
            *app.state::<AppState>()
                .selection_presentation
                .lock()
                .unwrap() = Some(presentation);
            *app.state::<AppState>().selection_source.lock().unwrap() =
                Some(serde_json::json!({"name":"Synthetic source"}));
            on_main(app, crate::selection::dispatch)?;
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
                        .selection_presentation
                        .lock()
                        .unwrap()
                        .is_some()
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
                wait_dom(app, "return {ready:document.querySelector('.source-input')?.value==='Synthetic pending selected text'};")?;
                if app
                    .state::<AppState>()
                    .selection_presentation
                    .lock()
                    .unwrap()
                    .is_some()
                {
                    return Err(
                        "Discard did not complete the prepared selection presentation".into(),
                    );
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

fn records_open_runtime_state(app: &AppHandle) -> serde_json::Value {
    let state = app.state::<AppState>();
    let regions = state.windows.lock().unwrap();
    serde_json::json!({"historyVisible":regions.history_visible,"translationVisible":regions.translation_visible,"recordView":regions.record_view,"recordsSession":regions.records_session,
        "captureEnabled":state.clipboard_enabled.load(Ordering::SeqCst),"clipboardVersion":state.clipboard_version.load(Ordering::SeqCst),"operation":state.operation.load(Ordering::SeqCst),
        "hasTarget":state.target.lock().unwrap().is_some(),"hasSelection":state.selection.lock().unwrap().is_some(),"selectionBusy":state.selection_busy.load(Ordering::SeqCst),"hasSelectionOrigin":state.selection_origin.lock().unwrap().is_some()})
}

fn records_open_native_visibility(app: &AppHandle) -> Result<serde_json::Value, String> {
    on_main(app, |app| {
        let window = app
            .get_window("translation")
            .ok_or("records host missing")?;
        let host = window.ns_window().map_err(|_| "native records host")? as usize;
        let (send, receive) = mpsc::sync_channel(1);
        app.get_webview("clipboard").ok_or("records view missing")?.with_webview(move |view| unsafe {
            let mut pointer=view.inner().cast::<AnyObject>();
            let attached_window:*mut AnyObject=objc2::msg_send![pointer,window];
            let hidden_ancestor:bool=objc2::msg_send![pointer,isHiddenOrHasHiddenAncestor];
            let mut ancestors=Vec::new();
            while !pointer.is_null()&&ancestors.len()<12 {
                let hidden:bool=objc2::msg_send![pointer,isHidden];
                let alpha:f64=objc2::msg_send![pointer,alphaValue];
                let frame:objc2_foundation::NSRect=objc2::msg_send![pointer,frame];
                let bounds:objc2_foundation::NSRect=objc2::msg_send![pointer,bounds];
                let visible:objc2_foundation::NSRect=objc2::msg_send![pointer,visibleRect];
                ancestors.push(serde_json::json!({"class":(&*pointer).class().name().to_string_lossy(),"hidden":hidden,"alpha":alpha,"frame":LayerFrame::from(frame),"bounds":LayerFrame::from(bounds),"visibleRect":LayerFrame::from(visible)}));
                pointer=objc2::msg_send![pointer,superview];
            }
            let _=send.send(serde_json::json!({"webviewWindowMatchesHost":attached_window as usize==host,"hiddenAncestor":hidden_ancestor,"ancestors":ancestors}));
        }).map_err(|_|"records visibility read")?;
        let view = receive
            .try_recv()
            .map_err(|_| "records visibility read deferred")?;
        let native = host as *mut AnyObject;
        let (occlusion, key, frame) = unsafe {
            let occlusion: usize = objc2::msg_send![native, occlusionState];
            let key: bool = objc2::msg_send![native, isKeyWindow];
            let frame: objc2_foundation::NSRect = objc2::msg_send![native, frame];
            (occlusion, key, frame)
        };
        Ok(
            serde_json::json!({"visible":window.is_visible().map_err(|_|"window visibility")?,"focused":window.is_focused().map_err(|_|"window focus")?,"keyWindow":key,"occlusionState":occlusion,"occlusionVisible":occlusion&2!=0,"windowFrame":LayerFrame::from(frame),"webview":view,"state":records_open_runtime_state(app)}),
        )
    })
}

fn records_hide_handoff(app: &AppHandle) -> Result<serde_json::Value, String> {
    let (send, receive) = mpsc::sync_channel(1);
    let was_focused = on_main(app, move |app| {
        let window = app
            .get_window("translation")
            .ok_or("records host missing")?;
        let focused = window.is_focused().map_err(|_| "window focus")?;
        window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Focused(false)) {
                let _ = send.try_send(());
            }
        });
        hide(app, "clipboard")?;
        Ok(focused)
    })?;
    if was_focused {
        receive
            .recv_timeout(Duration::from_secs(2))
            .map_err(|error| {
                format!(
                    "records hide did not deliver blur: {error}; state={}",
                    records_open_runtime_state(app)
                )
            })?;
    }
    let handoff = on_main(app, |app| {
        let window = app
            .get_window("translation")
            .ok_or("records host missing")?;
        let visible = window.is_visible().map_err(|_| "window visibility")?;
        let focused = window.is_focused().map_err(|_| "window focus")?;
        let state = records_open_runtime_state(app);
        if visible
            || focused
            || state["historyVisible"] != false
            || state["translationVisible"] != false
        {
            return Err(format!("records hide did not settle before next summon: visible={visible},focused={focused},state={state}"));
        }
        Ok(serde_json::json!({"nativeVisible":visible,"nativeFocused":focused,"state":state}))
    })?;
    Ok(handoff)
}

fn records_open_measurement(
    app: &AppHandle,
    label: &'static str,
    reopen: bool,
) -> Result<serde_json::Value, String> {
    let hide_handoff = if reopen {
        Some(records_hide_handoff(app)?)
    } else {
        None
    };
    let started = std::time::Instant::now();
    on_main(app, |app| resume_navigation(app, "clipboard"))?;
    let script = format!(
        r#"
const token={};
window.__recordsOpenVisibilityLog??=[];
if(!window.__recordsOpenVisibilityBound){{
  window.__recordsOpenVisibilityBound=true;
  const record=event=>{{const log=window.__recordsOpenVisibilityLog;log.push({{event,at:performance.now(),visibility:document.visibilityState,focused:document.hasFocus()}});if(log.length>24)log.shift();}};
  document.addEventListener('visibilitychange',()=>record('visibilitychange'));
  window.addEventListener('focus',()=>record('focus'));
  window.addEventListener('blur',()=>record('blur'));
}}
if(window.__recordsOpenProbe?.token!==token){{
  const probe=window.__recordsOpenProbe={{token,frames:0,staleFilteredFrames:0,loadingFrames:0,firstObserved:null,presented:false}};
  const sample=()=>{{
    if(window.__recordsOpenProbe!==probe||probe.presented)return;
    const p=document.querySelector('.clipboard-panel'),input=p?.querySelector('.history-search input'),rows=[...p?.querySelectorAll('.history-row:not([hidden])')??[]],count=p?.querySelector('.history-record-count')?.textContent;
    const visible=!!p?.getClientRects().length&&!p.closest('[hidden]'),query=input?.value,favorite=p?.querySelector('.history-favorites')?.getAttribute('aria-pressed'),kind=p?.querySelector('.history-kind[aria-pressed=true]')?.getAttribute('aria-label')??null,loading=!!p?.querySelector('[role=progressbar]');
    if(visible){{
      probe.frames++;probe.staleFilteredFrames+=Number(!!query||favorite!=='false'||kind!==null);probe.loadingFrames+=Number(loading);
      probe.firstObserved??={{rows:rows.length,count,query,favorite,kind,loading}};
    }}
    if(visible&&rows.length===100&&count==='100 records'&&query===''&&favorite==='false'&&kind===null&&input&&!input.disabled&&!input.readOnly&&document.activeElement===input&&document.hasFocus()&&document.fonts.status==='loaded'){{
      requestAnimationFrame(()=>{{probe.presented=true;}});
    }}else requestAnimationFrame(sample);
  }};
  requestAnimationFrame(sample);
}}
const p=document.querySelector('.clipboard-panel'),input=p?.querySelector('.history-search input'),rows=[...p?.querySelectorAll('.history-row:not([hidden])')??[]],probe=window.__recordsOpenProbe;
if(p&&!window.__recordsOpenPanel)window.__recordsOpenPanel=p;
return {{ready:probe.presented&&rows.length===100&&p?.querySelector('.history-record-count')?.textContent==='100 records'&&input?.value===''&&p?.querySelector('.history-favorites')?.getAttribute('aria-pressed')==='false'&&!p?.querySelector('.history-kind[aria-pressed=true]'),rows:rows.length,total:p?.querySelector('.history-record-count')?.textContent,query:input?.value,favorite:p?.querySelector('.history-favorites')?.getAttribute('aria-pressed'),kind:p?.querySelector('.history-kind[aria-pressed=true]')?.getAttribute('aria-label')??null,samePanel:!!p&&window.__recordsOpenPanel===p,searchReady:!!input&&!input.disabled&&!input.readOnly&&document.activeElement===input,syntheticOnly:rows.every(row=>row.dataset.recordId.startsWith('open-record-')),visible:!!p?.getClientRects().length&&!p.closest('[hidden]'),documentFocused:document.hasFocus(),activeElement:document.activeElement?.className,documentVisibility:document.visibilityState,visibilityEvents:window.__recordsOpenVisibilityLog,fonts:document.fonts.status,error:p?.querySelector('[role=alert]')?.textContent,...probe}};
"#,
        serde_json::to_string(label).unwrap()
    );
    let mut sample = wait_records(app, &script).map_err(|error| {
        let native = records_open_native_visibility(app);
        format!("{error}; native={native:?}")
    })?;
    let native_visible = on_main(app, |app| {
        app.get_window("translation")
            .ok_or_else(|| "records host missing".to_owned())?
            .is_visible()
            .map_err(|_| "window visibility".to_owned())
    })?;
    if sample["samePanel"] != true
        || sample["syntheticOnly"] != true
        || sample["searchReady"] != true
        || !native_visible
    {
        return Err(format!(
            "records open functional state: {sample}; nativeVisible={native_visible}"
        ));
    }
    sample["elapsedMs"] = serde_json::json!(started.elapsed().as_secs_f64() * 1000.0);
    sample["phase"] = label.into();
    sample["functionalChecksOnly"] = true.into();
    sample["performanceGateApplied"] = false.into();
    sample["nativeVisible"] = native_visible.into();
    if let Some(handoff) = hide_handoff {
        sample["hideHandoff"] = handoff;
    }
    Ok(sample)
}

fn run_records_open(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let mut original_database = None;
    let result = (|| {
        if app.config().identifier != "dev.local.nami.ui-tests"
            || app.get_window("translation").is_some()
        {
            return Err("records-open requires a fresh isolated host process".into());
        }
        let mut database =
            crate::database::HistoryDatabase::open(std::path::Path::new(":memory:"))?;
        for index in 0..100 {
            let text = if index == 0 {
                "Synthetic opening open-target".to_owned()
            } else {
                format!("Synthetic opening record {index}")
            };
            database.clipboard_insert(&nami::clipboard::ClipboardRecord {
                id: format!("open-record-{index}"),
                kind: "text".into(),
                fingerprint: nami::clipboard::fingerprint("text", text.as_bytes()),
                text,
                date: "2099-01-01T00:00:00Z".into(),
                is_favorite: index % 4 == 0,
                is_pinned: false,
                source_app: None,
                image_path: None,
            })?;
        }
        original_database = Some(
            app.state::<AppState>()
                .database
                .lock()
                .unwrap()
                .replace(database),
        );
        {
            let state = app.state::<AppState>();
            let mut regions = state.windows.lock().unwrap();
            regions.dismiss();
            regions.sync_records_session();
        }
        let prewarm = std::env::var("NAMI_UI_TEST_OPEN_PREWARM").as_deref() == Ok("1");
        if prewarm {
            let before = records_open_runtime_state(app);
            let started = std::time::Instant::now();
            on_main(app, |app| {
                prewarm_records(app)?;
                let window = app.get_window("translation").ok_or("prewarm host")?;
                if window.is_visible().map_err(|_| "window visibility")? {
                    return Err("prewarm showed the hidden host".into());
                }
                Ok(())
            })?;
            let ready = wait_records(
                app,
                r#"const p=document.querySelector('.clipboard-panel'),rows=[...p?.querySelectorAll('.history-row:not([hidden])')??[]];return {ready:rows.length===100&&p?.querySelector('.history-record-count')?.textContent==='100 records',rows:rows.length,error:p?.querySelector('[role=alert]')?.textContent};"#,
            )?;
            let after = records_open_runtime_state(app);
            if before != after
                || app
                    .get_window("translation")
                    .ok_or("prewarm host")?
                    .is_visible()
                    .map_err(|_| "window visibility")?
            {
                return Err(format!("prewarm changed visibility, regions, capture or selection: before={before}, after={after}"));
            }
            samples.push(serde_json::json!({"phase":"hidden-prewarm","elapsedMs":started.elapsed().as_secs_f64()*1000.0,"hidden":true,"runtimeUnchanged":true,"ready":ready,"functionalChecksOnly":true,"performanceGateApplied":false}));
        }
        let mut first = records_open_measurement(
            app,
            if prewarm {
                "prewarmed-first-open"
            } else {
                "cold-first-open"
            },
            false,
        )?;
        first["prewarmed"] = prewarm.into();
        samples.push(first);
        for phase in ["immediate-reopen-1", "immediate-reopen-2"] {
            samples.push(records_open_measurement(app, phase, true)?);
        }
        evaluate_view_dom(
            app,
            "clipboard",
            r#"const p=document.querySelector('.clipboard-panel'),input=p.querySelector('.history-search input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'open-target');input.dispatchEvent(new Event('input',{bubbles:true}));return true;"#,
        )?;
        samples.push(wait_records(app,r#"const p=document.querySelector('.clipboard-panel'),rows=[...p.querySelectorAll('.history-row:not([hidden])')];return {ready:rows.length===1&&rows[0].dataset.recordId==='open-record-0'&&p.querySelector('.history-search input').value==='open-target',phase:'established-search',rows:rows.length,total:p.querySelector('.history-record-count').textContent};"#)?);
        samples.push(records_open_measurement(
            app,
            "immediate-reopen-reset-filters",
            true,
        )?);
        Ok(())
    })();
    if let Some(database) = original_database {
        *app.state::<AppState>().database.lock().unwrap() = database;
        let _ = crate::clipboard_runtime::changed(app);
    }
    DomCase {
        name: "records open and reopen readiness",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_selection_open(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |_| crate::platform::test_selection_clipboard())?;
        if app.config().identifier != "dev.local.nami.ui-tests"
            || app.get_window("translation").is_some()
        {
            return Err("selection-open requires a fresh isolated host process".into());
        }
        let clipboard_version = app
            .state::<AppState>()
            .clipboard_version
            .load(Ordering::SeqCst);
        on_main(app, |app| {
            {
                let state = app.state::<AppState>();
                let mut regions = state.windows.lock().unwrap();
                regions.dismiss();
                regions.sync_records_session();
            }
            prewarm_records(app)
        })?;
        wait_dom(
            app,
            r#"const input=document.querySelector('.source-input'),connected=document.documentElement.dataset.nativeHistory==='true';if(input&&connected&&!window.__selectionInitialProbe){window.__selectionInitialProbe={done:false};(async()=>{try{const context=await window.__TAURI_INTERNALS__.invoke('selection_take');await window.__TAURI_INTERNALS__.invoke('selection_present',{presentationId:0});await window.__TAURI_INTERNALS__.invoke('translation_cancel');window.__selectionInitialProbe={done:true,empty:context===null};}catch(error){window.__selectionInitialProbe={done:true,error:String(error)};}})();}return {ready:!!input&&connected&&window.__selectionInitialProbe?.done&&window.__selectionInitialProbe.empty,probe:window.__selectionInitialProbe,input:!!input,connected};"#,
        )?;
        if app
            .get_window("translation")
            .ok_or("prewarm host")?
            .is_visible()
            .map_err(|_| "window visibility")?
        {
            return Err("selection prewarm showed the host".into());
        }
        for (index, phase) in [
            "selection-first-open",
            "selection-reopen-clear-draft",
            "selection-reopen-empty",
        ]
        .into_iter()
        .enumerate()
        {
            let handoff = if index > 0 {
                if index == 1 {
                    evaluate_dom(
                        app,
                        r#"const input=document.querySelector('.source-input');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(input,'Synthetic stale draft');input.dispatchEvent(new Event('input',{bubbles:true}));return true;"#,
                    )?;
                    wait_dom(
                        app,
                        r#"const input=document.querySelector('.source-input');return {ready:input?.value==='Synthetic stale draft',draft:input?.value};"#,
                    )?;
                }
                Some(records_hide_handoff(app)?)
            } else {
                None
            };
            evaluate_dom(
                app,
                r#"window.__selectionOpenProbe={presented:false,scheduled:false,staleOnFocus:false};window.addEventListener('focus',()=>{window.__selectionOpenProbe.staleOnFocus=!!document.querySelector('.source-input')?.value;},{once:true});window.__selectionOpenPanel??=document.querySelector('.translation-clip');window.__selectionOpenInput??=document.querySelector('.source-input');return true;"#,
            )?;
            let revision = app
                .state::<AppState>()
                .translation_revision
                .load(Ordering::SeqCst);
            let started = std::time::Instant::now();
            on_main(app, |app| {
                activate_application();
                crate::selection::test_empty_summon(app)
            })?;
            let script = r#"
const input=document.querySelector('.source-input'),panel=document.querySelector('.translation-clip'),probe=window.__selectionOpenProbe;
const ready=!!input&&input.value===''&&panel?.getAttribute('aria-hidden')==='false'&&document.activeElement===input&&document.hasFocus()&&document.visibilityState==='visible';
if(ready&&!probe.scheduled){probe.scheduled=true;requestAnimationFrame(()=>requestAnimationFrame(()=>{if(window.__selectionOpenProbe===probe)probe.presented=true;}));}
return {ready:ready&&probe.presented,empty:input?.value==='',focused:document.activeElement===input,documentFocused:document.hasFocus(),visibility:document.visibilityState,panelShown:panel?.getAttribute('aria-hidden')==='false',samePanel:panel===window.__selectionOpenPanel,sameInput:input===window.__selectionOpenInput,staleOnFocus:probe.staleOnFocus,emptyResults:[...document.querySelectorAll('.result-column')].every(n=>!n.textContent),presented:probe.presented,error:document.querySelector('[role=alert]')?.textContent};
"#;
            let mut sample = wait_dom(app, script).map_err(|error| {
                format!(
                    "{phase}: {error}; native={:?}",
                    records_open_native_visibility(app)
                )
            })?;
            let state = app.state::<AppState>();
            let windows = state.windows.lock().unwrap();
            if !windows.translation_visible
                || windows.history_visible
                || state.selection_busy.load(Ordering::SeqCst)
                || state.selection.lock().unwrap().is_some()
                || state.selection_presentation.lock().unwrap().is_some()
                || state.translation_revision.load(Ordering::SeqCst) <= revision
                || state.clipboard_version.load(Ordering::SeqCst) != clipboard_version
                || sample["samePanel"] != true
                || sample["sameInput"] != true
                || sample["staleOnFocus"] != false
                || sample["emptyResults"] != true
            {
                return Err(format!(
                    "selection-open did not finish blank summon: {sample}"
                ));
            }
            drop(windows);
            let visible = app
                .get_window("translation")
                .ok_or("selection host")?
                .is_visible()
                .map_err(|_| "window visibility")?;
            if !visible {
                return Err("selection-open host remained hidden".into());
            }
            sample["phase"] = phase.into();
            sample["elapsedMs"] = serde_json::json!(started.elapsed().as_secs_f64() * 1000.0);
            sample["functionalChecksOnly"] = true.into();
            sample["performanceGateApplied"] = false.into();
            sample["nativeVisible"] = true.into();
            sample["clipboardVersionUnchanged"] = true.into();
            if let Some(handoff) = handoff {
                sample["hideHandoff"] = handoff;
            }
            samples.push(sample);
        }
        Ok(())
    })();
    DomCase {
        name: "selection empty summon and clipboard protection",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn clipboard_text_fixture(text: bool) -> Result<Vec<u8>, String> {
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{ns_string, NSData, NSPoint, NSRect, NSSize, NSString};
    unsafe {
        let image: *mut AnyObject = msg_send![class!(NSImage), alloc];
        let image: *mut AnyObject = msg_send![image, initWithSize: NSSize::new(640.0,160.0)];
        let image = Retained::from_raw(image).ok_or("synthetic image allocation")?;
        let _: () = msg_send![&*image, lockFocus];
        let white: Retained<AnyObject> = msg_send![class!(NSColor), whiteColor];
        let _: () = msg_send![&*white, setFill];
        let _: () = msg_send![class!(NSBezierPath), fillRect: NSRect::new(NSPoint::new(0.0,0.0),NSSize::new(640.0,160.0))];
        if text {
            let attributes: Retained<AnyObject> =
                msg_send![class!(NSMutableDictionary), dictionary];
            let font: Retained<AnyObject> = msg_send![class!(NSFont), boldSystemFontOfSize: 48f64];
            let color: Retained<AnyObject> = msg_send![class!(NSColor), blackColor];
            let _: () = msg_send![&*attributes, setObject: &*font, forKey: ns_string!("NSFont")];
            let _: () = msg_send![&*attributes, setObject: &*color, forKey: ns_string!("NSColor")];
            let label = NSString::from_str("HELLO NAMI");
            let _: () = msg_send![&*label, drawAtPoint: NSPoint::new(48.0,48.0), withAttributes: &*attributes];
        }
        let _: () = msg_send![&*image, unlockFocus];
        let tiff: Retained<NSData> = msg_send![&*image, TIFFRepresentation];
        let bitmap: Retained<AnyObject> =
            msg_send![class!(NSBitmapImageRep), imageRepWithData: &*tiff];
        let properties: Retained<AnyObject> = msg_send![class!(NSDictionary), dictionary];
        let png: Retained<NSData> =
            msg_send![&*bitmap, representationUsingType: 4usize, properties: &*properties];
        Ok(png.to_vec())
    }
}

fn run_clipboard_details(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let mut original_database = None;
    let result = (|| {
        if app.config().identifier != "dev.local.nami.ui-tests" {
            return Err("clipboard details tests require the isolated application identity".into());
        }
        let mut database =
            crate::database::HistoryDatabase::open(std::path::Path::new(":memory:"))?;
        let full_text = "Synthetic complete clipboard text 👋\n".repeat(120);
        let application = "Synthetic Application With A Complete Untruncated Name";
        for (id, text, pinned, date) in [
            (
                "detail-pinned",
                "Pinned synthetic text".to_string(),
                true,
                "2099-01-03T00:00:00Z",
            ),
            (
                "detail-first",
                full_text.clone(),
                false,
                "2099-01-02T00:00:00Z",
            ),
            (
                "detail-second",
                "Second synthetic detail".to_string(),
                false,
                "2099-01-01T00:00:00Z",
            ),
        ] {
            database.clipboard_insert(&nami::clipboard::ClipboardRecord {
                id: id.into(),
                kind: "text".into(),
                fingerprint: nami::clipboard::fingerprint("text", text.as_bytes()),
                text,
                date: date.into(),
                is_favorite: false,
                is_pinned: pinned,
                source_app: Some(serde_json::json!({"name":application})),
                image_path: None,
            })?;
        }
        original_database = Some(
            app.state::<AppState>()
                .database
                .lock()
                .unwrap()
                .replace(database),
        );
        app.state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .records_session += 1;
        crate::clipboard_runtime::changed(app)?;
        records_mode(app, "clipboard")?;
        wait_records(app, "return {ready:document.querySelectorAll('.clipboard-panel .history-row:not([hidden])').length===3};")?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"window.__detailList=document.querySelector('.clipboard-panel');document.querySelector('.clipboard-panel .records-details').click();return true;"#,
        )?;
        let expected = serde_json::to_string(&full_text).unwrap();
        let application_json = serde_json::to_string(application).unwrap();
        let first = wait_dom(
            app,
            &format!(
                r#"
const p=document.querySelector('.clipboard-detail'),text=p?.querySelector('.clipboard-detail-text')?.textContent;
const metadata=p?.querySelector('.clipboard-detail-metadata'),actions=[...p?.querySelectorAll('.history-action:not([aria-hidden=true])')??[]].map(e=>e.dataset.action);
const chars=metadata&&[...metadata.children].find(e=>e.firstElementChild?.textContent==='Characters')?.lastElementChild?.textContent;
const app=p?.querySelector('.clipboard-detail-metadata .history-languages')?.textContent;
const scroller=p?.querySelector('.history-list'),scrollbar=p?.querySelector('[role=scrollbar]');
const footerBounds=metadata?.getBoundingClientRect(),viewportBounds=p?.querySelector('.history-viewport')?.getBoundingClientRect();
window.__detailInput??=document.querySelector('.source-input');window.__detailPane??=p;
return {{ready:text==={expected}&&!!scrollbar&&actions.includes('note'),length:text?.length,application:app,characters:Number(chars?.replaceAll(',','')),formattedCharacters:chars,actions,translationHidden:document.querySelector('.translation-panel')?.hidden,scrollbarHidden:!!scrollbar&&!scrollbar.classList.contains('visible'),overflow:scroller?.scrollHeight>scroller?.clientHeight,metadata:[...metadata?.children??[]].map(e=>e.firstElementChild?.textContent),icons:[...p?.querySelectorAll('.history-row-actions .history-symbol')??[]].map(e=>{{const r=e.getBoundingClientRect();return {{width:r.width,height:r.height}};}}),completeApplication:app==={application_json},footerBounds:footerBounds&&{{top:footerBounds.top,bottom:footerBounds.bottom}},viewportBottom:viewportBounds?.bottom,footerAligned:footerBounds?.bottom>innerHeight-80&&footerBounds.bottom<=innerHeight&&viewportBounds?.bottom<=footerBounds.top}};
"#
            ),
        )?;
        if first["characters"] != full_text.chars().count()
            || first["translationHidden"] != true
            || first["scrollbarHidden"] != true
            || first["overflow"] != true
            || first["footerAligned"] != true
            || first["completeApplication"] != true
            || first["metadata"] != serde_json::json!(["Application", "Characters"])
            || first["actions"]
                != serde_json::json!(["copy", "translate", "note", "favorite", "pin", "delete"])
            || first["icons"].as_array().is_none_or(|icons| {
                icons.len() != 6
                    || icons
                        .iter()
                        .any(|icon| icon["width"] != 14 || icon["height"] != 14)
            })
        {
            return Err(format!("clipboard detail contents or layout: {first}"));
        }
        samples.push(first);
        let native = on_main(app, native_chrome_sample)?;
        if native["buttons"].as_array().is_none_or(|buttons| {
            buttons.len() != 3 || buttons.iter().any(|button| button["hidden"] != true)
        }) {
            return Err(format!(
                "translation controls overlap clipboard details: {native}"
            ));
        }
        samples.push(native);
        let list = wait_records(
            app,
            r#"const b=document.querySelector('.clipboard-panel .history-toolbar > button'),r=b?.getBoundingClientRect(),id=document.querySelector('.clipboard-panel .history-row.selected')?.dataset.recordId;return {ready:id==='detail-first',selected:id,toolbarCenter:r.y+r.height/2,sameList:window.__detailList===document.querySelector('.clipboard-panel')};"#,
        )?;
        let toolbar = evaluate_dom(
            app,
            r#"const b=document.querySelector('.clipboard-detail-toolbar button'),r=b.getBoundingClientRect();return {toolbarCenter:r.y+r.height/2};"#,
        )?;
        if (list["toolbarCenter"]
            .as_f64()
            .ok_or("left toolbar center")?
            - toolbar["toolbarCenter"]
                .as_f64()
                .ok_or("detail toolbar center")?)
        .abs()
            > 1.0
            || list["sameList"] != true
        {
            return Err(format!(
                "detail toolbar does not align with records: {list} / {toolbar}"
            ));
        }
        samples.push(serde_json::json!({"records":list,"details":toolbar}));
        evaluate_dom(
            app,
            r#"window.__detailPointerSettled=false;document.querySelector('.clipboard-detail [role=scrollbar]').dispatchEvent(new PointerEvent('pointermove',{bubbles:true,pointerId:7}));requestAnimationFrame(()=>requestAnimationFrame(()=>window.__detailPointerSettled=true));return true;"#,
        )?;
        let pointer = wait_dom(
            app,
            r#"return {ready:window.__detailPointerSettled,visible:document.querySelector('.clipboard-detail [role=scrollbar]')?.classList.contains('visible')};"#,
        )?;
        if pointer["visible"] != false {
            return Err(format!(
                "detail scrollbar appeared without scrolling: {pointer}"
            ));
        }
        samples.push(pointer);
        evaluate_dom(
            app,
            r#"document.querySelector('.clipboard-detail .history-list').dispatchEvent(new WheelEvent('wheel',{bubbles:true,deltaY:80}));return true;"#,
        )?;
        samples.push(wait_dom(app, r#"return {ready:document.querySelector('.clipboard-detail [role=scrollbar]')?.classList.contains('visible')===true};"#)?);
        evaluate_dom(
            app,
            r#"document.querySelector('.clipboard-detail [aria-label="Note"]').click();return true;"#,
        )?;
        wait_records(
            app,
            r#"const editor=document.querySelector('.clipboard-editor');return {ready:!!editor&&!editor.querySelector('input').disabled};"#,
        )?;
        samples.push(wait_dom(
            app,
            r#"return {ready:document.querySelector('.clipboard-detail')?.inert===true};"#,
        )?);
        if !app
            .state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .records_editor_open
        {
            return Err("detail note did not mark the shared editor open".into());
        }
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('.clipboard-editor').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));return true;"#,
        )?;
        wait_records(
            app,
            "return {ready:!document.querySelector('.clipboard-editor')};",
        )?;
        samples.push(wait_dom(
            app,
            "return {ready:document.querySelector('.clipboard-detail')?.inert===false};",
        )?);
        if app
            .state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .records_editor_open
        {
            return Err("closing detail note left the shared editor open".into());
        }
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('.clipboard-panel [data-record-id="detail-second"]').dispatchEvent(new MouseEvent('mouseover',{bubbles:true}));return true;"#,
        )?;
        samples.push(wait_dom(app, r#"const text=document.querySelector('.clipboard-detail-text')?.textContent;return {ready:text==='Second synthetic detail',text};"#)?);
        wait_records(
            app,
            r#"return {ready:document.querySelector('.clipboard-panel .history-row.selected')?.dataset.recordId==='detail-second'};"#,
        )?;
        wait_motion(app)?;
        let initial = on_main(app, geometry)?;
        initial.adjacent()?;
        equal(
            initial.translation_width,
            480.0,
            "detail default minimum width",
        )?;
        samples.push(on_main(app, |app| {
            let history = view_frame(app, "history")?;
            let detail = view_frame(app, "translation")?;
            equal(detail.origin.y, history.origin.y, "detail top matches records")?;
            equal(detail.size.height, history.size.height, "detail height matches records")?;
            let window = app.get_window("translation").ok_or("missing_test_window")?;
            let native = window.ns_window().map_err(|_| "window_geometry")?.cast::<objc2::runtime::AnyObject>();
            let minimum: objc2_foundation::NSSize = unsafe { objc2::msg_send![native, contentMinSize] };
            equal(minimum.width, history.size.width + 480.0, "detail shrink limit")?;
            let native_frame = native_translation_frame(&window)?;
            native_resize::test_drag(&window, objc2_foundation::NSPoint::new(native_frame.size.width - 1.0, native_frame.size.height / 2.0), objc2_foundation::NSPoint::new(-120.0, 0.0))?;
            let resized = geometry(app)?;
            equal(resized.translation_width, 480.0, "detail cannot shrink below default")?;
            equal(resized.history_width, history.size.width, "minimum drag preserves records width")?;
            Ok(serde_json::json!({"detailWidth":resized.translation_width,"historyHeight":history.size.height,"detailHeight":detail.size.height,"sameTop":detail.origin.y==history.origin.y,"minimumWidth":480}))
        })?);
        for _ in 0..3 {
            evaluate_view_dom(
                app,
                "clipboard",
                "document.querySelector('.clipboard-panel .records-details').click();return true;",
            )?;
            wait_records(app, "return {ready:document.querySelector('.clipboard-panel .records-details')?.getAttribute('aria-expanded')==='false'};")?;
            wait_motion(app)?;
            evaluate_view_dom(
                app,
                "clipboard",
                "document.querySelector('.clipboard-panel .records-details').click();return true;",
            )?;
            wait_records(app, "return {ready:document.querySelector('.clipboard-panel .records-details')?.getAttribute('aria-expanded')==='true'};")?;
            wait_motion(app)?;
            let frame = on_main(app, geometry)?;
            frame.adjacent()?;
            equal(
                frame.window_left,
                initial.window_left,
                "detail reopen left anchor",
            )?;
            equal(
                frame.window_width,
                initial.window_width,
                "detail reopen window width",
            )?;
            equal(
                frame.history_width,
                initial.history_width,
                "detail reopen records width",
            )?;
            samples.push(wait_dom(app, r#"return {ready:document.querySelector('.clipboard-detail-text')?.textContent?.startsWith('Synthetic complete clipboard text')&&document.querySelector('.source-input')===window.__detailInput&&document.querySelector('.clipboard-detail')===window.__detailPane,sameTranslation:document.querySelector('.source-input')===window.__detailInput,sameDetails:document.querySelector('.clipboard-detail')===window.__detailPane};"#)?);
        }
        on_main(app, |app| {
            {
                let state = app.state::<AppState>();
                let mut windows = state.windows.lock().unwrap();
                windows.history_visible = false;
                windows.translation_pinned = true;
            }
            apply_translation(app)
        })?;
        wait_motion(app)?;
        wait_records(app, "return {ready:document.querySelector('.clipboard-panel')?.closest('.record-view')?.inert===true};")?;
        evaluate_dom(
            app,
            r#"document.querySelector('.clipboard-detail [aria-label="Note"]').click();return true;"#,
        )?;
        wait_records(
            app,
            r#"const editor=document.querySelector('.clipboard-editor');return {ready:!!editor&&!editor.querySelector('input').disabled};"#,
        )?;
        samples.push(wait_dom(
            app,
            "return {ready:document.querySelector('.clipboard-detail')?.inert===true};",
        )?);
        {
            let state = app.state::<AppState>();
            let windows = state.windows.lock().unwrap();
            if !windows.history_visible
                || !windows.records_editor_open
                || !windows.translation_visible
            {
                return Err(
                    "detail-only Note failed to reveal and protect the shared editor".into(),
                );
            }
        }
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('.clipboard-editor').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));return true;"#,
        )?;
        wait_records(
            app,
            "return {ready:!document.querySelector('.clipboard-editor')};",
        )?;
        samples.push(wait_dom(
            app,
            "return {ready:document.querySelector('.clipboard-detail')?.inert===false};",
        )?);
        Ok(())
    })();
    if let Some(database) = original_database {
        *app.state::<AppState>().database.lock().unwrap() = database;
        let _ = crate::clipboard_runtime::changed(app);
    }
    DomCase {
        name: "clipboard full details and hover navigation",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_clipboard_kind_text(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let mut original_database = None;
    let mut owned_images = Vec::new();
    let result = (|| {
        if app.config().identifier != "dev.local.nami.ui-tests" {
            return Err("clipboard kind tests require the isolated application identity".into());
        }
        let mut database =
            crate::database::HistoryDatabase::open(std::path::Path::new(":memory:"))?;
        let root = crate::clipboard_runtime::image_root(app)?;
        let blank_png = on_main(app, |_| clipboard_text_fixture(false))?;
        let text_png = on_main(app, |_| clipboard_text_fixture(true))?;
        let mut image_paths = Vec::new();
        for bytes in [blank_png.as_slice(), text_png.as_slice()] {
            let (path, created) = nami::clipboard::store_image_with_status(&root, bytes, "png")?;
            if created {
                owned_images.push(root.join(&path));
            }
            image_paths.push(path);
        }
        let full_text = format!("Needle {}", "synthetic full text ".repeat(40));
        for (id, kind, text, favorite, image) in [
            ("kind-text-favorite", "text", full_text.clone(), true, None),
            (
                "kind-text-ordinary",
                "text",
                "Ordinary synthetic text".into(),
                false,
                None,
            ),
            (
                "kind-image-blank",
                "image",
                "Image".into(),
                true,
                Some(image_paths[0].clone()),
            ),
            (
                "kind-image-text",
                "image",
                "Needle image".into(),
                true,
                Some(image_paths[1].clone()),
            ),
            (
                "kind-files",
                "files",
                serde_json::to_string(&[root.join("Needle.txt")]).map_err(|_| "fixture")?,
                true,
                None,
            ),
        ] {
            database.clipboard_insert(&nami::clipboard::ClipboardRecord {
                id: id.into(),
                kind: kind.into(),
                fingerprint: nami::clipboard::fingerprint(kind, text.as_bytes()),
                text,
                date: "2099-01-01T00:00:00Z".into(),
                is_favorite: favorite,
                is_pinned: false,
                source_app: None,
                image_path: image,
            })?;
        }
        original_database = Some(
            app.state::<AppState>()
                .database
                .lock()
                .unwrap()
                .replace(database),
        );
        app.state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .records_session += 1;
        crate::clipboard_runtime::changed(app)?;
        records_mode(app, "clipboard")?;
        wait_records(
            app,
            r#"const p=document.querySelector('.clipboard-panel'),rows=[...p.querySelectorAll('.history-row:not([hidden])')].map(row=>row.dataset.recordId),total=p.querySelector('.history-record-count')?.textContent;return {ready:rows.length===5&&total==='5 records',rows,total,query:p.querySelector('.history-search input')?.value,favorites:p.querySelector('.history-favorites')?.getAttribute('aria-pressed'),kind:p.querySelector('.history-kind[aria-pressed=true]')?.getAttribute('aria-label'),visible:!!p.getClientRects().length&&!p.closest('[hidden]'),loading:!!p.querySelector('[role=progressbar]'),error:p.querySelector('[role=alert]')?.textContent};"#,
        )?;

        for width in [454.0, 280.0, 360.0] {
            on_main(app, move |app| {
                let state = app.state::<AppState>();
                let mut windows = state.windows.lock().unwrap();
                windows.history_width = width;
                windows.history_extent = width;
                drop(windows);
                layout_translation(app, false)
            })?;
            let sample = wait_records(
                app,
                &format!(
                    r#"
const p=document.querySelector('.clipboard-panel'),f=p.querySelector('.history-footer'),filters=f.querySelector('.history-filters');
const box=e=>{{const r=e.getBoundingClientRect();return {{left:r.left,right:r.right,width:r.width,height:r.height}};}};
return {{ready:Math.abs(p.getBoundingClientRect().width-{width})<.1,width:p.getBoundingClientRect().width,count:box(f.querySelector('.history-record-count')),filters:box(filters),keyboard:box(f.querySelector('.history-shortcuts')),icons:[...filters.querySelectorAll('.history-symbol')].map(box)}};
"#
                ),
            )?;
            if sample["count"]["right"].as_f64().unwrap()
                > sample["filters"]["left"].as_f64().unwrap() + 0.5
                || sample["filters"]["right"].as_f64().unwrap()
                    > sample["keyboard"]["left"].as_f64().unwrap() + 0.5
            {
                return Err(format!("footer filters overlap at {width}: {sample}"));
            }
            if sample["icons"].as_array().is_none_or(|icons| {
                icons.len() != 4
                    || icons
                        .iter()
                        .any(|icon| icon["width"] != 14 || icon["height"] != 14)
            }) {
                return Err(format!("footer filter icon sizing: {sample}"));
            }
            samples.push(sample);
        }
        for (label, expected) in [
            (
                "Text clipboard records",
                vec!["kind-text-favorite", "kind-text-ordinary"],
            ),
            (
                "Image clipboard records",
                vec!["kind-image-blank", "kind-image-text"],
            ),
            ("File clipboard records", vec!["kind-files"]),
        ] {
            evaluate_view_dom(app,"clipboard",&format!("document.querySelector('.clipboard-panel [aria-label={}]').click();return true;",serde_json::to_string(label).unwrap()))?;
            let expected = serde_json::to_string(&expected).unwrap();
            samples.push(wait_records(app,&format!(r#"const p=document.querySelector('.clipboard-panel'),ids=[...p.querySelectorAll('.history-row:not([hidden])')].map(r=>r.dataset.recordId).sort(),total=p.querySelector('.history-record-count').textContent;return {{ready:JSON.stringify(ids)===JSON.stringify({expected}.sort())&&total===`${{ids.length}} ${{ids.length===1?'record':'records'}}`,ids,total}};"#))?);
        }
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('.clipboard-panel [aria-label="Favorite clipboard records"]').click();const input=document.querySelector('.clipboard-panel .history-search input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'Needle');input.dispatchEvent(new Event('input',{bubbles:true}));return true;"#,
        )?;
        for (label, id) in [
            ("Text clipboard records", "kind-text-favorite"),
            ("Image clipboard records", "kind-image-text"),
            ("File clipboard records", "kind-files"),
        ] {
            evaluate_view_dom(app,"clipboard",&format!("document.querySelector('.clipboard-panel [aria-label={}]').click();return true;",serde_json::to_string(label).unwrap()))?;
            samples.push(wait_records(app,&format!(r#"const p=document.querySelector('.clipboard-panel'),rows=[...p.querySelectorAll('.history-row:not([hidden])')];return {{ready:rows.length===1&&rows[0].dataset.recordId==='{id}'&&p.querySelector('.history-record-count').textContent==='1 record'&&p.querySelector('.history-favorites').getAttribute('aria-pressed')==='false',id:rows[0]?.dataset.recordId,total:p.querySelector('.history-record-count').textContent,favorite:p.querySelector('.history-favorites').getAttribute('aria-pressed'),query:p.querySelector('.history-search input').value}};"#))?);
        }
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('.clipboard-panel [aria-label="File clipboard records"]').click();const input=document.querySelector('.clipboard-panel .history-search input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'');input.dispatchEvent(new Event('input',{bubbles:true}));return true;"#,
        )?;
        let actions = wait_records_attempts(
            app,
            r#"
const p=document.querySelector('.clipboard-panel'),rows=[...p.querySelectorAll('.history-row:not([hidden])')];
const actions=id=>[...p.querySelectorAll(`[data-record-id="${id}"] .history-action:not([aria-hidden=true])`)].map(a=>a.dataset.action);
const text=actions('kind-text-favorite'),image=actions('kind-image-text'),blank=actions('kind-image-blank'),files=actions('kind-files');
return {ready:rows.length===5&&image.includes('ocr'),text,image,blank,files,icons:[...p.querySelectorAll('.history-action .history-symbol')].map(i=>({width:getComputedStyle(i).width,height:getComputedStyle(i).height}))};
"#,
            1500,
        )?;
        for (key, expected) in [
            ("text", vec!["copy", "translate", "note", "favorite", "pin"]),
            (
                "image",
                vec!["copy", "translate", "ocr", "note", "favorite", "pin"],
            ),
            ("blank", vec!["copy", "note", "favorite", "pin"]),
            ("files", vec!["copy", "note", "favorite", "pin"]),
        ] {
            if actions[key] != serde_json::json!(expected) {
                return Err(format!("clipboard text actions: {actions}"));
            }
        }
        if actions["icons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|icon| icon["width"] != "14px" || icon["height"] != "14px")
        {
            return Err(format!("record icon sizing: {actions}"));
        }
        samples.push(actions);
        let version = app
            .state::<AppState>()
            .clipboard_version
            .load(Ordering::SeqCst);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"window.__kindIpc={done:false};(async()=>{try { const invoke=window.__TAURI_INTERNALS__.invoke;const blank=await invoke('clipboard_record_has_text',{recordId:'kind-image-blank'}),image=await invoke('clipboard_record_has_text',{recordId:'kind-image-text'});let textOcrRejected=false;try{await invoke('clipboard_record_process',{recordId:'kind-text-favorite',action:'ocr'});}catch(error){textOcrRejected=error==='invalid_argument';}window.__kindIpc={done:true,blank,image,textOcrRejected};}catch(error){window.__kindIpc={done:true,error:String(error)};}})();return true;"#,
        )?;
        let ipc = wait_records(
            app,
            "return {ready:window.__kindIpc?.done,...window.__kindIpc};",
        )?;
        if ipc["blank"] != false
            || ipc["image"] != true
            || ipc["textOcrRejected"] != true
            || app
                .state::<AppState>()
                .clipboard_version
                .load(Ordering::SeqCst)
                != version
            || app
                .state::<AppState>()
                .database
                .lock()
                .unwrap()
                .as_ref()
                .ok_or("test database")?
                .clipboard_page("", false, None, 100)?
                .total_records
                != 5
        {
            return Err(format!("OCR detection or invalid-action protection: {ipc}"));
        }
        samples.push(ipc);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"(async()=>{for(const recordId of ['kind-text-favorite','kind-image-text'])await window.__TAURI_INTERNALS__.invoke('clipboard_record_update',{recordId,action:'favorite'});})();return true;"#,
        )?;
        samples.push(wait_records(app, r#"
const p=document.querySelector('.clipboard-panel');
const actions=id=>[...p.querySelectorAll(`[data-record-id="${id}"] .history-row-actions button:not([aria-hidden=true])`)].map(button=>button.getAttribute('aria-label'));
const text=actions('kind-text-favorite'),ordinary=actions('kind-text-ordinary'),image=actions('kind-image-text');
return {ready:text.includes('Favorite')&&text.includes('Translate')&&ordinary.includes('Translate')&&image.includes('Favorite')&&image.includes('Translate')&&image.includes('Extract text'),text,ordinary,image};
"#)?);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('.clipboard-panel [data-record-id="kind-text-favorite"] [aria-label="Translate"]').click();return true;"#,
        )?;
        let translated = wait_dom(app,&format!("return {{ready:document.querySelector('.source-input')?.value==={},length:document.querySelector('.source-input')?.value.length}};",serde_json::to_string(&full_text).unwrap()))?;
        {
            let state = app.state::<AppState>();
            let windows = state.windows.lock().unwrap();
            if windows.record_view != crate::model::RecordView::Translation
                || !windows.translation_visible
            {
                return Err("clipboard Translate failed to open translation details".into());
            }
        }
        samples.push(translated);
        let file_image = root.join(format!("ocr-file-{}.png", uuid::Uuid::new_v4()));
        std::fs::write(&file_image, &text_png).map_err(|_| "fixture image file")?;
        owned_images.push(file_image.clone());
        {
            let state = app.state::<AppState>();
            let mut database = state.database.lock().unwrap();
            let database = database.as_mut().ok_or("test database")?;
            for (id, paths) in [
                ("kind-image-file", vec![file_image.clone()]),
                (
                    "kind-multiple-files",
                    vec![file_image.clone(), root.join("other.png")],
                ),
            ] {
                let text = serde_json::to_string(&paths).map_err(|_| "fixture file list")?;
                database.clipboard_insert(&nami::clipboard::ClipboardRecord {
                    id: id.into(),
                    kind: "files".into(),
                    fingerprint: nami::clipboard::fingerprint("files", text.as_bytes()),
                    text,
                    date: "2099-01-02T00:00:00Z".into(),
                    is_favorite: false,
                    is_pinned: false,
                    source_app: None,
                    image_path: None,
                })?;
            }
        }
        crate::clipboard_runtime::changed(app)?;
        records_mode(app, "clipboard")?;
        samples.push(wait_records_attempts(app, r#"
const p=document.querySelector('.clipboard-panel');
const actions=id=>[...p.querySelectorAll(`[data-record-id="${id}"] .history-row-actions button:not([aria-hidden=true])`)].map(button=>button.getAttribute('aria-label'));
const image=actions('kind-image-file'),multiple=actions('kind-multiple-files');
return {ready:image.includes('Translate')&&image.includes('Extract text')&&multiple.includes('Copy')&&!multiple.includes('Translate')&&!multiple.includes('Extract text'),image,multiple};
"#, 1500)?);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('.clipboard-panel [data-record-id="kind-image-file"] [aria-label="Translate"]').click();return true;"#,
        )?;
        samples.push(wait_dom(app, r#"const text=document.querySelector('.source-input')?.value;return {ready:text?.includes('HELLO NAMI'),text};"#)?);
        std::fs::write(&file_image, &blank_png).map_err(|_| "fixture image update")?;
        records_mode(app, "clipboard")?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"window.__fileOcr={done:false};(async()=>{try{const hasText=await window.__TAURI_INTERNALS__.invoke('clipboard_record_has_text',{recordId:'kind-image-file'});window.__fileOcr={done:true,hasText};}catch(error){window.__fileOcr={done:true,error:String(error)};}})();return true;"#,
        )?;
        let updated = wait_records(
            app,
            "return {ready:window.__fileOcr?.done,...window.__fileOcr};",
        )?;
        if updated["hasText"] != false {
            return Err(format!(
                "mutable image file OCR used stale result: {updated}"
            ));
        }
        samples.push(updated);
        samples.push(wait_records(app, r#"const row=document.querySelector('.clipboard-panel [data-record-id="kind-image-file"]');return {ready:!!row&&!row.querySelector('[aria-label="Translate"]:not([aria-hidden=true])')&&!row.querySelector('[aria-label="Extract text"]:not([aria-hidden=true])')};"#)?);
        Ok(())
    })();
    if let Some(database) = original_database {
        *app.state::<AppState>().database.lock().unwrap() = database;
        let _ = crate::clipboard_runtime::changed(app);
    }
    for path in owned_images {
        let _ = std::fs::remove_file(path);
    }
    DomCase {
        name: "clipboard kind filters and text actions",
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
            "clipboard-kind-text",
            "clipboard-details",
            "translation-actions",
            "unified-records",
            "records-geometry",
            "records-pinned",
            "records-pagination",
            "records-open",
            "selection-open",
        ]
        .contains(&case.as_str())
        {
            return Err("unknown native test case".into());
        }
        let app = app.clone();
        std::thread::spawn(move || {
            let result = if case == "selection-open" {
                run_selection_open(&app)
            } else if case == "records-open" {
                run_records_open(&app)
            } else if case == "clipboard-kind-text" {
                run_clipboard_kind_text(&app)
            } else if case == "clipboard-details" {
                run_clipboard_details(&app)
            } else if case == "records-pagination" {
                run_history_pagination(&app)
            } else if case == "records-geometry" {
                run_records_geometry(&app)
            } else if case == "records-pinned" {
                run_records_pinned(&app)
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
        dom_cases.push(run_clipboard_kind_text(&app));
        dom_cases.push(run_clipboard_details(&app));
        dom_cases.push(run_clipboard_capture_setting(&app));
        dom_cases.push(run_records_pinned(&app));
        let clipboard_history = on_main(&app, |_| crate::platform::test_clipboard_history());
        dom_cases.push(DomCase {
            name: "clipboard history: text, image, files, private exclusion, PNG thumbnail and missing-file protection",
            passed: clipboard_history.is_ok(),
            error: clipboard_history.err(),
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
