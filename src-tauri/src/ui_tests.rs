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
    wait_view_dom(app, "translation", script)
}

fn wait_view_dom(
    app: &AppHandle,
    label: &'static str,
    script: &str,
) -> Result<serde_json::Value, String> {
    let mut last = serde_json::Value::Null;
    for _ in 0..40 {
        let value = evaluate_view_dom(app, label, script)?;
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
    for retired_tag in [102isize, 103] {
        let retired: *mut AnyObject =
            unsafe { objc2::msg_send![&*stage, viewWithTag: retired_tag] };
        if !retired.is_null() {
            return Err(format!(
                "retired native control {retired_tag} is still installed"
            ));
        }
    }
    let tag = 101isize;
    {
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
    equal(
        right - number(source, "screenRight")?,
        24.0,
        "native source right inset",
    )?;
    equal(
        number(value, "height")? - number(source, "y")?,
        40.0,
        "native source top inset",
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

fn run_translation_partial(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |app| show(app, "translation"))?;
        wait_dom(
            app,
            "return {ready:!!document.querySelector('.source-input')};",
        )?;
        let persisted = |id: &str| {
            let state = app.state::<AppState>();
            let database = state.database.lock().unwrap();
            database
                .as_ref()
                .ok_or("isolated test database")?
                .record(id)
        };
        let history_total = || {
            let state = app.state::<AppState>();
            let database = state.database.lock().unwrap();
            database
                .as_ref()
                .ok_or("isolated test database")?
                .page(&nami::database::HistoryQuery::default(), None, 1)
                .map(|page| page.total_records)
        };
        let initial_total = history_total()?;
        evaluate_dom(
            app,
            r#"
window.__partialArgs = {text:'Synthetic split primary',source:'auto',targets:['en','ja'],fromSelection:false};
window.__partialRun = null;
(async()=>{
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const reject = async action => {try{await action();return 'unexpected success';}catch(e){return String(e);}};
  try {
    const {partial} = await invoke('translation_run',window.__partialArgs);
    await invoke('translation_sample_copy',{sampleId:partial.id,part:'primary'});
    const badCopy=await reject(()=>invoke('translation_sample_copy',{sampleId:partial.id,part:'secondary'}));
    const badRetry=await reject(()=>invoke('translation_run',{...window.__partialArgs,text:'Changed original',retryId:partial.id}));
    window.__partialRun={partial,badCopy,badRetry};
  } catch(e) { window.__partialRun={error:String(e)}; }
})();return true;
"#,
        )?;
        let initial = wait_dom(
            app,
            "return {ready:!!window.__partialRun,...window.__partialRun};",
        )?;
        let partial_id = initial["partial"]["id"]
            .as_str()
            .ok_or_else(|| format!("missing split response: {initial}"))?;
        if initial["partial"]["translations"] != serde_json::json!(["Synthetic translation en", ""])
            || initial["partial"]["errors"]
                != serde_json::json!([null, "Synthetic service unavailable"])
            || initial["badCopy"] != "invalid_argument"
            || initial["badRetry"] != "invalid_argument"
            || persisted(partial_id)?.is_some()
            || on_main(app, |_| {
                Ok(crate::platform::read_clipboard(|_| false)?.map(|capture| capture.payload))
            })? != Some(crate::platform::ClipboardPayload::Text(
                "Synthetic translation en".into(),
            ))
        {
            return Err(format!(
                "partial copy, isolation or retry validation failed: {initial}"
            ));
        }
        samples.push(serde_json::json!({"primarySurvives":true,"secondaryCopyRejected":true,"requestMutationRejected":true,"partialNotPersisted":true}));
        evaluate_dom(
            app,
            r#"
window.__retryRun=null;
(async()=>{
  const invoke=window.__TAURI_INTERNALS__.invoke, retryId=window.__partialRun.partial.id;
  try {
    const record=await invoke('translation_run',{...window.__partialArgs,retryId});
    let staleCopy;
    try{await invoke('translation_sample_copy',{sampleId:retryId,part:'primary'});}catch(e){staleCopy=String(e);}
    window.__retryRun={record,staleCopy};
  } catch(e) { window.__retryRun={error:String(e)}; }
})();return true;
"#,
        )?;
        let retried = wait_dom(
            app,
            "return {ready:!!window.__retryRun,...window.__retryRun};",
        )?;
        let saved_id = retried["record"]["id"]
            .as_str()
            .ok_or_else(|| format!("retry did not complete: {retried}"))?;
        let saved = persisted(saved_id)?.ok_or("complete retry not stored")?;
        if saved.translations != ["Synthetic translation en", "Synthetic translation ja"]
            || retried["staleCopy"] != "invalid_argument"
        {
            return Err(format!("retry storage or token check failed: {retried}"));
        }
        samples.push(serde_json::json!({"onlyFailedColumnRetried":true,"completePersisted":true,"expiredCopyRejected":true}));

        evaluate_dom(
            app,
            r#"
window.__nextRun=null;
(async()=>{
  const invoke=window.__TAURI_INTERNALS__.invoke;
  try {
    const {partial}=await invoke('translation_run',{...window.__partialArgs,text:'Synthetic split secondary'});
    await invoke('translation_sample_copy',{sampleId:partial.id,part:'secondary'});
    let totalError;
    try{await invoke('translation_run',{...window.__partialArgs,text:'Synthetic split total failure'});}catch(e){totalError=String(e);}
    const heldArgs={...window.__partialArgs,text:'Synthetic held retry'};
    const held=(await invoke('translation_run',heldArgs)).partial;
    window.__nextRun={partial,totalError,held};
    window.__heldResult=null;
    invoke('translation_run',{...heldArgs,retryId:held.id}).then(value=>window.__heldResult={value}, error=>window.__heldResult={error:String(error)});
  } catch(e) {window.__nextRun={error:String(e)};}
})();return true;
"#,
        )?;
        let next = wait_dom(
            app,
            "return {ready:!!window.__nextRun,...window.__nextRun};",
        )?;
        let second_id = next["partial"]["id"]
            .as_str()
            .ok_or_else(|| format!("missing secondary: {next}"))?;
        let held_id = next["held"]["id"]
            .as_str()
            .ok_or("missing cancellation fixture")?;
        if next["partial"]["translations"] != serde_json::json!(["", "Synthetic translation ja"])
            || next["partial"]["errors"]
                != serde_json::json!(["Synthetic service unavailable", null])
            || next["totalError"] != "Synthetic service unavailable"
            || persisted(second_id)?.is_some()
            || on_main(app, |_| {
                Ok(crate::platform::read_clipboard(|_| false)?.map(|capture| capture.payload))
            })? != Some(crate::platform::ClipboardPayload::Text(
                "Synthetic translation ja".into(),
            ))
        {
            return Err(format!(
                "secondary preservation or total error failed: {next}"
            ));
        }
        samples.push(serde_json::json!({"secondarySurvives":true,"doubleFailureRejected":true}));
        let state = app.state::<AppState>();
        let retrying = || {
            state
                .pending_translation
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|draft| draft.retrying)
        };
        for _ in 0..50 {
            if retrying() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !retrying() {
            return Err("held retry did not start".into());
        }
        evaluate_dom(
            app,
            r#"
window.__heldCopy=null;
(async()=>{
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const retryId=window.__nextRun.held.id;
  try {
    await invoke('translation_sample_copy',{sampleId:retryId,part:'primary'});
    let doubleRetry;
    try { await invoke('translation_run',{...window.__partialArgs,text:'Synthetic held retry',retryId}); } catch(e) { doubleRetry=String(e); }
    await invoke('translation_cancel');
    window.__heldCopy={doubleRetry};
  } catch(e) {window.__heldCopy={error:String(e)};}
})();return true;
"#,
        )?;
        let during = wait_dom(
            app,
            "return {ready:!!window.__heldCopy,...window.__heldCopy};",
        )?;
        if during["doubleRetry"] != "invalid_argument"
            || on_main(app, |_| {
                Ok(crate::platform::read_clipboard(|_| false)?.map(|capture| capture.payload))
            })? != Some(crate::platform::ClipboardPayload::Text(
                "Synthetic translation en".into(),
            ))
        {
            return Err(format!(
                "copy or duplicate retry protection during failed-column retry: {during}"
            ));
        }
        samples.push(serde_json::json!({"copyDuringRetry":true,"duplicateRetryRejected":true}));
        let cancelled = wait_dom(
            app,
            "return {ready:!!window.__heldResult,...window.__heldResult};",
        )?;
        let total = history_total()?;
        if cancelled["error"] != "cancelled"
            || persisted(held_id)?.is_some()
            || total != initial_total + 1
        {
            return Err(format!("cancelled retry changed history: {cancelled}, before={initial_total}, after={total}"));
        }
        samples.push(
            serde_json::json!({"inFlightRetryCancelled":true,"onlyOneCompleteHistoryRecord":true}),
        );
        evaluate_dom(
            app,
            r#"
window.__alternateRun=null;
(async()=>{
  const invoke=window.__TAURI_INTERNALS__.invoke, args=window.__partialArgs;
  try {
    const {partial}=await invoke('translation_run',args);
    const mixed=await invoke('translation_run',{...args,retryId:partial.id,retryEngine:'google'});
    const full=await invoke('translation_run',{...args,text:'Synthetic split total failure',retryEngine:'google'});
    const snapshot=await invoke('app_snapshot');
    window.__alternateRun={mixed,full,engine:snapshot.translationEngine};
  } catch(e) {window.__alternateRun={error:String(e)};}
})();return true;
"#,
        )?;
        let swapped = wait_dom(
            app,
            "return {ready:!!window.__alternateRun,...window.__alternateRun};",
        )?;
        let mixed_id = swapped["mixed"]["id"]
            .as_str()
            .ok_or_else(|| format!("alternate retry failed: {swapped}"))?;
        let mixed = persisted(mixed_id)?.ok_or("mixed result was not saved")?;
        if mixed.translations
            != [
                "Synthetic translation en",
                "Synthetic Google translation ja",
            ]
            || mixed
                .translation
                .is_none_or(|metadata| metadata.target_engines != ["edge", "google"])
            || swapped["full"]["translations"]
                != serde_json::json!([
                    "Synthetic Google translation en",
                    "Synthetic Google translation ja"
                ])
            || swapped["engine"] != "edge"
            || history_total()? != initial_total + 3
        {
            return Err(format!(
                "alternate engine reran a successful column or changed the default: {swapped}"
            ));
        }
        samples.push(serde_json::json!({"failedColumnOnly":true,"providerProvenancePreserved":true,"totalFailureRecovered":true,"preferredEngineUnchanged":true}));
        Ok(())
    })();
    DomCase {
        name: "translation partial columns and retry through WKWebView IPC",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_right_control_dom(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |app| create(app, "translation").map(|_| ()))?;
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
        wait_dom(app, "return { ready: document.querySelector('.source-bar > .language-picker').textContent === 'Chinese (Traditional)' && !document.querySelector('.language-popover') && document.activeElement===document.querySelector('.source-input') };")?;
        for width in [700.0, 920.0, 1200.0] {
            on_main(app, move |app| reset(app, 360.0, width))?;
            settle_chrome_dom(app)?;
            samples.push(check_right_control_dom(app, width)?);
        }
        samples.push(evaluate_dom(app, r#"
const button=document.querySelector('.translation-footer .language-picker');
button.focus({preventScroll:true});
const style=getComputedStyle(button), outline=style.outlineStyle;
if(document.activeElement!==button||outline!=='none'||style.borderTopWidth!=='0px')throw Error('target language button still draws a focus border');
return {languagePickerOutline:outline};
"#)?);
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
        settle_chrome_dom(app)?;
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
        on_main(app, |app| {
            app.state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .history_visible = false;
            apply_translation(app)
        })?;
        wait_motion(app)?;
        on_main(app, |app| {
            crate::sync_global_shortcuts(app);
            Ok(())
        })?;
        let press_history = || {
            on_main(app, |app| {
                send_registered_hotkey(
                    app,
                    crate::shortcuts::KeyBinding::new("Slash", &["meta"]),
                    6,
                )
            })
        };
        press_history()?;
        samples.push(wait_records(app,"const row=document.querySelector('#translation-history .history-row-select'),input=document.querySelector('#translation-history input[type=search]');return {ready:!!row&&document.activeElement===input&&!row.closest('[hidden],[inert]'),historyShortcut:'opened'};")?);
        press_history()?;
        samples.push(wait_dom(app, "return {ready:document.querySelector('.history-trigger').getAttribute('aria-expanded')==='false'&&!document.querySelector('.action-menu'),historyShortcut:'closed'};")?);
        wait_motion(app)?;
        let expect_state = |history: bool, translation: bool, visible: bool| {
            for _ in 0..100 {
                let ready = on_main(app, move |app| {
                    use crate::model::RecordView;
                    let state = app.state::<AppState>();
                    let regions = state.windows.lock().unwrap();
                    let host = app.get_window("translation").ok_or("window_unavailable")?;
                    Ok(regions.history_visible == history
                        && regions.translation_visible == translation
                        && (!history || regions.record_view == RecordView::Translation)
                        && host.is_visible().map_err(|_| "window_geometry")? == visible)
                })?;
                if ready {
                    return wait_motion(app);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(format!("history hotkey did not settle: history={history}, translation={translation}, visible={visible}"))
        };
        for details in [false, true] {
            // Seed the clipboard region without reading the system clipboard.
            on_main(app, move |app| {
                let host = create(app, "translation")?;
                {
                    let state = app.state::<AppState>();
                    let mut regions = state.windows.lock().unwrap();
                    regions.dismiss();
                    regions.set_record_view(crate::model::RecordView::Clipboard);
                    if details {
                        regions.toggle_record_details();
                    }
                }
                apply_translation(app)?;
                host.show().map_err(|_| "window_show".to_owned())
            })?;
            wait_motion(app)?;
            press_history()?;
            expect_state(true, details, true)?;
            press_history()?;
            expect_state(false, details, details)?;
            samples.push(serde_json::json!({"historyShortcut":"clipboard", "details":details}));
        }
        on_main(app, |app| hide(app, "translation"))?;
        press_history()?;
        expect_state(true, false, true)?;
        press_history()?;
        expect_state(false, false, false)?;
        samples.push(serde_json::json!({"historyShortcut":"hidden host"}));
        on_main(app, |app| show(app, "translation"))?;
        evaluate_dom(app, "const input=document.querySelector('.source-input');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(input,'');input.dispatchEvent(new Event('input',{bubbles:true}));return true;")?;
        wait_dom(
            app,
            "return {ready:document.querySelector('.source-input').value===''};",
        )?;
        evaluate_dom(app, "const input=document.querySelector('.source-input');input.focus();input.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',code:'Enter',metaKey:true,bubbles:true,cancelable:true}));return true;")?;
        let blank = evaluate_dom(app, "return {empty:document.querySelector('.source-input').value==='',status:!!document.querySelector('.translation-status')};")?;
        if blank["empty"] != true || blank["status"] != false {
            return Err(format!(
                "empty source submitted through the translation shortcut: {blank}"
            ));
        }
        evaluate_dom(app, "const input = document.querySelector('.source-input'); Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(input, 'x'.repeat(5001)); input.dispatchEvent(new Event('input', {bubbles:true})); return true;")?;
        evaluate_dom(app, "const input=document.querySelector('.source-input');input.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',code:'Enter',metaKey:true,bubbles:true,cancelable:true}));return true;")?;
        samples.push(wait_dom(app, "const status = document.querySelector('.translation-status'); return { ready: !!status, translatedNotice: status?.textContent }; ")?);
        if !samples.last().unwrap()["translatedNotice"]
            .as_str()
            .unwrap_or_default()
            .contains("Input limit:")
        {
            return Err("translation shortcut did not invoke validated submit behavior".into());
        }
        samples.push(on_main(app, native_chrome_sample)?);
        samples.extend(check_retranslation_shortcut(app)?);
        samples.extend(check_translation_copy_shortcuts(app)?);
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

fn check_translation_copy_shortcuts(app: &AppHandle) -> Result<Vec<serde_json::Value>, String> {
    let id = "sample-07";
    let record = app
        .state::<AppState>()
        .history
        .lock()
        .unwrap()
        .record(id)
        .ok_or("missing single-target fixture")?;
    if record.translations.len() != 1 {
        return Err("copy fixture must have one translation".into());
    }
    evaluate_view_dom(
        app,
        "clipboard",
        "window.__TAURI_INTERNALS__.invoke('records_details_toggle');return true;",
    )?;
    wait_records(app, "const row=document.querySelector('[data-record-id=sample-07] .history-row-select');if(!row||document.querySelector('.translation-history .records-details[aria-expanded=true]'))return {ready:false};row.focus();return {ready:document.hasFocus()&&document.activeElement===row&&document.querySelector('#translation-history .records-details')?.getAttribute('aria-expanded')==='false'};")?;
    let mut samples = Vec::new();
    for (index, (code, flags, key, text)) in [
        (18, 1 << 20, "1", record.translations[0].as_str()),
        (19, 1 << 20, "2", record.translations[0].as_str()),
        (8, (1 << 20) | (1 << 17), "C", record.text.as_str()),
    ]
    .into_iter()
    .enumerate()
    {
        post_records_key(app, code, flags, key, key, false)?;
        let mut copied = false;
        for _ in 0..80 {
            let state = app.state::<AppState>();
            let page = state
                .database
                .lock()
                .unwrap()
                .as_ref()
                .ok_or("missing_test_database")?
                .clipboard_page("", false, None, 100)?;
            if page.total_records == index + 1 {
                let id = &page.records.first().ok_or("missing clipboard copy")?.id;
                let saved = state
                    .database
                    .lock()
                    .unwrap()
                    .as_ref()
                    .ok_or("missing_test_database")?
                    .clipboard_record(id)?;
                let clipboard = on_main(app, |_| {
                    crate::platform::read_clipboard(|_| false).map_err(str::to_owned)
                })?;
                copied = saved.text == text && clipboard.is_some_and(|capture| matches!(capture.payload, crate::platform::ClipboardPayload::Text(value) if value == text));
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if !copied
            || app
                .state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .translation_visible
        {
            return Err(format!(
                "translation copy key {key} did not save full text without opening details"
            ));
        }
        samples.push(
            serde_json::json!({"copyKey":key,"clipboardRecords":index+1,"detailsOpen":false}),
        );
    }
    Ok(samples)
}

fn check_retranslation_shortcut(app: &AppHandle) -> Result<Vec<serde_json::Value>, String> {
    let mut samples = Vec::new();
    records_mode(app, "translation")?;
    for detail in [false, true] {
        if detail {
            on_main(app, |app| {
                app.get_webview("translation")
                    .ok_or("translation view")?
                    .set_focus()
                    .map_err(|_| "detail focus")?;
                Ok(())
            })?;
            wait_dom(app, "const result=document.querySelector('.result-column');result.focus();return {ready:document.hasFocus()&&document.activeElement===result};")?;
        } else {
            wait_records(app, "const row=document.querySelector('#translation-history .history-items .history-row-select');row?.focus();return {ready:!!row&&document.hasFocus()&&document.activeElement===row};")?;
        }
        post_records_key(app, 36, 1 << 20, "\r", "\r", false)?;
        samples.push(wait_dom(app, &format!("const error=document.querySelector('.translation-status [role=alert]')?.textContent;return {{ready:error==='translation_network_disabled_in_tests',detail:{detail},error}};"))?);
        if !detail {
            // Restore the displayed translation so the detail shortcut starts on a saved record.
            let id = app
                .state::<AppState>()
                .history
                .lock()
                .unwrap()
                .selected_id()
                .ok_or("shortcut did not select a history record")?
                .to_owned();
            on_main(app, move |app| {
                crate::commands::select_history_sample(app, Some(id), false, false)
            })?;
            wait_dom(app, "return {ready:!!document.querySelector('.result-column span')&&!document.querySelector('.translation-status [role=alert]')};")?;
        }
    }
    let mut result = {
        let state = app.state::<AppState>();
        let history = state.history.lock().unwrap();
        history
            .record(history.selected_id().ok_or("missing source selection")?)
            .ok_or("source record missing")?
    };
    let origin = result.id.clone();
    result.id = format!("retranslated-{}", uuid::Uuid::new_v4());
    result.is_pinned = false;
    result.is_favorite = false;
    let id = result.id.clone();
    let handle = app.clone();
    let (send, receive) = mpsc::channel();
    tauri::async_runtime::spawn(async move {
        let revision = handle
            .state::<AppState>()
            .translation_revision
            .load(Ordering::SeqCst);
        let _ = send.send(
            crate::history_runtime::commit_translation(
                &handle,
                result,
                "edge".into(),
                revision,
                Some(crate::history_runtime::TranslationFocus::History(origin)),
            )
            .await,
        );
    });
    receive
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "retranslation commit timeout")??;
    samples.push(wait_records(app, &format!("const row=document.querySelector('[data-record-id={id}] .history-row-select');return {{ready:!!row&&row.getAttribute('aria-pressed')==='true'&&document.hasFocus()&&document.activeElement===row,retranslatedFocus:{id:?},focused:document.activeElement?.closest('[data-record-id]')?.dataset.recordId}};"))?);
    post_records_key(app, 3, 1 << 20, "f", "f", false)?;
    samples.push(wait_dom(app, "return {ready:document.hasFocus()&&document.activeElement===document.querySelector('.source-input'),focusInputFromHistory:true};")?);
    let handle = app.clone();
    let (send, receive) = mpsc::channel();
    tauri::async_runtime::spawn(async move {
        let _ = send
            .send(crate::history_runtime::update_record(&handle, id, "delete".into(), None).await);
    });
    receive
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "retranslation cleanup timeout")??;
    Ok(samples)
}

fn check_right_control_dom(app: &AppHandle, width: f64) -> Result<serde_json::Value, String> {
    let script = format!(
        r#"
        const source = document.querySelector('.source-bar > .language-picker');
        const input = document.querySelector('.source-input');
        const s = source.getBoundingClientRect(), i = input.getBoundingClientRect();
        const footer = document.querySelector('.translation-footer');
        const pickers = [...footer.querySelectorAll('.language-slot:not(.secondary-off) .language-picker')].map(p => p.getBoundingClientRect().toJSON());
        return {{ pickers, ready: Math.abs(innerWidth - {width}) < .01 && i.right <= s.left - 1,
            width: innerWidth, native: document.documentElement.dataset.native,
            sourcePosition: getComputedStyle(source).position, sourceVisibility: getComputedStyle(source).visibility,
            sourceAriaHidden: source.getAttribute('aria-hidden'),
            nativeChrome: document.querySelector('.translation-panel').dataset.nativeChrome,
            sourceRightInset: innerWidth - s.right, sourceTopInset: s.top,
            footerHeight: footer.getBoundingClientRect().height, footerButtons: footer.querySelectorAll('button').length,
            inputPickerGap: s.left - i.right, sourceLabel: source.textContent }};
    "#
    );
    let value = wait_dom(app, &script)?;
    if value["sourcePosition"] != "fixed"
        || value["nativeChrome"] != "true"
        || value["sourceVisibility"] != "hidden"
        || value["sourceAriaHidden"] != "true"
    {
        return Err(format!(
            "web source control remained visible or accessible: {value}"
        ));
    }
    if value["footerButtons"] != 2 || value["footerHeight"] != 28.0 {
        return Err(format!(
            "translation footer must only contain language pickers in one row: {value}"
        ));
    }
    let native = on_main(app, native_chrome_sample)?;
    assert_chrome_anchors(&native)?;
    if native["buttons"][0]["title"] != value["sourceLabel"] {
        return Err("native source label did not follow React selection".into());
    }
    if value["native"] != "true" {
        return Err("DOM test did not load the native stylesheet".into());
    }
    for (key, expected) in [("sourceRightInset", 24.0), ("sourceTopInset", 8.0)] {
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

fn run_translation_settings(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let visible = std::env::var("NAMI_UI_TEST_VISIBLE").as_deref() == Ok("1");
    let result = (|| {
        on_main(app, move |app| {
            let window = create(app, "settings")?;
            window
                .set_size(tauri::LogicalSize::new(760.0, 640.0))
                .map_err(|_| "window_size")?;
            if visible {
                show(app, "settings")?;
            }
            Ok(())
        })?;
        wait_view_dom(
            app,
            "settings",
            r#"const nav=document.querySelector('button[aria-label="Translation"]');if(!nav)return {ready:false};if(!nav.hasAttribute('aria-current'))nav.click();return {ready:!!document.querySelector('#translation-proxy-url')};"#,
        )?;
        on_main(app, |app| {
            create(app, "settings")?
                .set_size(tauri::LogicalSize::new(460.0, 640.0))
                .map_err(|_| "window_size".to_owned())
        })?;
        samples.push(wait_view_dom(app, "settings", r#"
if(!matchMedia('(max-width: 480px)').matches)return {ready:false};
const field=document.querySelector('#translation-proxy-url'), toggle=document.querySelector('#translation-proxy-enabled');
const row=toggle.parentElement, page=document.querySelector('.settings-content');
const box=node=>node.getBoundingClientRect(), address=box(field), checkbox=box(toggle);
if(Math.abs(address.left+address.width/2-box(row).left-box(row).width/2)>.5||box(row.querySelector('label')).bottom>address.top||Math.abs(checkbox.top+checkbox.height/2-address.top-address.height/2)>.1||address.width<170||address.width>240||page.scrollWidth!==page.clientWidth)throw Error('narrow proxy layout crowds the address or spills outside its page');
return {ready:true,narrowAddressWidth:address.width};
"#)?);
        on_main(app, |app| {
            create(app, "settings")?
                .set_size(tauri::LogicalSize::new(760.0, 640.0))
                .map_err(|_| "window_size".to_owned())
        })?;
        wait_view_dom(app, "settings", "return {ready:innerWidth>700};")?;
        evaluate_view_dom(
            app,
            "settings",
            r#"document.querySelector('#translation-proxy-enabled').click();return true;"#,
        )?;
        samples.push(wait_view_dom(app, "settings", r#"const field=document.querySelector('#translation-proxy-url'), toggle=document.querySelector('#translation-proxy-enabled'), page=document.querySelector('.settings-content');return {ready:field.getAttribute('aria-invalid')==='true'&&!toggle.checked&&page.scrollWidth===page.clientWidth,rejectedEmpty:!toggle.checked,horizontalOverflow:page.scrollWidth-page.clientWidth};"#)?);
        evaluate_view_dom(
            app,
            "settings",
            r#"const field=document.querySelector('#translation-proxy-url');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(field,'http://proxy.example.test:8080');field.dispatchEvent(new Event('input',{bubbles:true}));return true;"#,
        )?;
        samples.push(evaluate_view_dom(app, "settings", r#"
const field=document.querySelector('#translation-proxy-url'), row=field.parentElement;
const address=field.getBoundingClientRect(), bounds=row.getBoundingClientRect();
const save=row.querySelector('button[type=submit]'), checkbox=row.querySelector('input[type=checkbox]');
if(!save||Math.abs(address.left+address.width/2-bounds.left-bounds.width/2)>.5||save.getBoundingClientRect().right>checkbox.getBoundingClientRect().left)throw Error('showing Save must not shift the input or overlap the checkbox');
return {centeredWhileEditing:true};
"#)?);
        evaluate_view_dom(
            app,
            "settings",
            r#"document.querySelector('#translation-proxy-enabled').click();return true;"#,
        )?;
        samples.push(wait_view_dom(app, "settings", r#"const field=document.querySelector('#translation-proxy-url'), toggle=document.querySelector('#translation-proxy-enabled');return {ready:toggle.checked&&!field.readOnly&&field.getAttribute('aria-invalid')==='false',enabled:toggle.checked};"#)?);
        let state = app.state::<AppState>();
        if state.translation_preferences.lock().unwrap().proxy_url
            != "http://proxy.example.test:8080"
        {
            return Err("proxy was enabled without the submitted address".into());
        }
        evaluate_view_dom(
            app,
            "settings",
            r#"document.querySelector('#translation-proxy-enabled').click();return true;"#,
        )?;
        wait_view_dom(
            app,
            "settings",
            r#"return {ready:!document.querySelector('#translation-proxy-enabled').checked&&!document.querySelector('#translation-proxy-url').readOnly};"#,
        )?;
        let styles = evaluate_view_dom(
            app,
            "settings",
            r#"
const controls=[...document.querySelectorAll('.settings-dropdown')];
if(document.querySelector('.settings-recorder'))throw Error('translation preferences still contain shortcut recorders');
const transparent='rgba(0, 0, 0, 0)', idle=controls.filter(control=>!control.matches(':hover, :focus-visible'));
if(!idle.length||idle.some(control=>getComputedStyle(control).backgroundColor!==transparent||getComputedStyle(control).borderTopColor!==transparent))throw Error('idle controls still draw a surface or boundary');
const title=parseFloat(getComputedStyle(document.querySelector('.settings-page h1')).fontSize);
const headings=[...document.querySelectorAll('.settings-page h2')].map(h=>parseFloat(getComputedStyle(h).fontSize));
if(title!==22||headings.some(size=>size!==16)||document.querySelector('.settings-page :is(h1,h2) svg'))throw Error('settings titles must use a compact, text-only hierarchy');
const groups=[...document.querySelectorAll('.settings-group')];
if(groups.some(group=>getComputedStyle(group).borderTopWidth!=='0px'||getComputedStyle(group).backgroundColor===transparent))throw Error('settings groups must use fill without an enclosing line');
const divider=getComputedStyle(document.querySelector('.settings-field + .settings-field')).borderTopWidth;
if(divider!=='1px')throw Error('row separators were removed');
const proxy=document.querySelector('#translation-proxy-url'), toggle=document.querySelector('#translation-proxy-enabled');
const row=toggle.closest('.settings-field'), label=row.querySelector('label');
const box=node=>node.getBoundingClientRect(), checkbox=box(toggle), address=box(proxy);
if(row!==proxy.closest('.settings-field')||toggle.checked||proxy.readOnly||getComputedStyle(row).opacity!=='1')throw Error('the disabled proxy should retain an editable address in the same row');
if(checkbox.width!==16||checkbox.height!==16||Math.abs(checkbox.right-box(row).right)>.1||box(label).right>=address.left||address.right>=checkbox.left||Math.abs(checkbox.top+checkbox.height/2-address.top-address.height/2)>.1)throw Error('proxy label, address and checkbox do not align in one row');
if(address.width>240||address.width<200||Math.abs(address.left+address.width/2-box(row).left-box(row).width/2)>.5||getComputedStyle(proxy).textAlign!=='center')throw Error('proxy input and its text must be centered in the row');
const sectionHeadings=[...document.querySelectorAll('.settings-section-heading')];
if(sectionHeadings.length!==2||sectionHeadings.some(heading=>box(heading).height!==28))throw Error('translation must use the shared settings section heading');
proxy.blur();
const idleProxyBackground=getComputedStyle(proxy).backgroundColor;
proxy.focus({preventScroll:true});
const proxyStyle=getComputedStyle(proxy), proxyFocused=proxy.matches(':focus');
if(document.activeElement!==proxy||proxyStyle.outlineStyle!=='none'||(proxyFocused&&proxyStyle.backgroundColor===idleProxyBackground))throw Error('proxy focus must use a tinted background without an outer outline');
const control=controls.find(item=>!item.matches(':hover'));
control.focus({preventScroll:true});
const focus=getComputedStyle(control);
const focusVisible=control.matches(':focus-visible');
if(document.activeElement!==control||focus.outlineStyle!=='none'||focus.borderTopColor!==transparent||focus.textDecorationLine!=='none'||(focusVisible&&focus.backgroundColor===transparent))throw Error('keyboard focus must use background without a frame or underline');
return {title,headings,cards:groups.length,divider,proxyFocused,focusVisible,focusBackground:focus.backgroundColor};
"#,
        )?;
        if visible && (styles["proxyFocused"] != true || styles["focusVisible"] != true) {
            return Err(format!(
                "visible proxy input or dropdown did not show focus: {styles}"
            ));
        }
        samples.push(styles);
        evaluate_view_dom(
            app,
            "settings",
            "document.querySelector('.settings-dropdown').click();return true;",
        )?;
        samples.push(wait_view_dom(app, "settings", r#"
const popup=document.querySelector('.settings-popover');if(!popup)return {ready:false};
const option=popup.querySelector('button[data-active=true]');if(!option)return {ready:false};
if(getComputedStyle(popup).borderTopWidth!=='0px'||getComputedStyle(option).outlineStyle!=='none')throw Error('settings picker still draws an enclosing or focus line');
option.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
return {ready:true,framelessPicker:true};
"#)?);
        wait_view_dom(
            app,
            "settings",
            r#"
const nav=document.querySelector('button[aria-label="HotKey"]');if(!nav)return {ready:false};
if(!nav.hasAttribute('aria-current'))nav.click();
return {ready:!!document.querySelector('#translation-shortcuts-heading')};
"#,
        )?;
        if visible {
            wait_view_dom(app, "settings", "return {ready:document.hasFocus()};")?;
        }
        let clear_style = evaluate_view_dom(
            app,
            "settings",
            r#"
const recorder=document.querySelector('[aria-label="Open Translation shortcut"]'), row=recorder.parentElement;
const restore=document.querySelector('#translation-shortcuts-heading + button'), transparent='rgba(0, 0, 0, 0)';
for(const control of [recorder,restore].filter(node=>!node.matches(':hover, :focus-visible'))) {
  const style=getComputedStyle(control);
  if(style.backgroundColor!==transparent||style.borderTopColor!==transparent)throw Error('idle shortcut controls still draw a surface or boundary');
}
const clear=row.querySelector('.settings-clear');
if(!row.matches(':hover')&&getComputedStyle(clear).opacity!=='0')throw Error('clear action is visible on an idle recorder');
clear.focus({preventScroll:true});
const clearFocus=getComputedStyle(clear);
const focusVisible=clear.matches(':focus-visible');
if(document.activeElement!==clear||clearFocus.backgroundColor!==transparent||clearFocus.borderTopColor!==transparent||(focusVisible&&(clearFocus.opacity!=='1'||clearFocus.outlineStyle==='none')))throw Error('clear action must keep a keyboard focus indicator without a button surface');
const long=document.querySelector('[aria-label="Pin / Unpin Window shortcut"]');
const bounds=button=>button.getBoundingClientRect(), keyWidth=button=>bounds(button.querySelector('.keycaps')).width;
const shortWidth=bounds(recorder).width, longWidth=bounds(long).width;
if(shortWidth-keyWidth(recorder)>34||Math.abs((longWidth-shortWidth)-(keyWidth(long)-keyWidth(recorder)))>.1||longWidth<=shortWidth)throw Error('shortcut controls do not fit their keycaps');
const layout=bounds(row.closest('.settings-shortcut-row')), label=bounds(row.previousElementSibling), checkbox=bounds(row.nextElementSibling);
if(Math.abs(label.left-layout.left)>.1||label.right>bounds(recorder).left||checkbox.width!==16||Math.abs(checkbox.right-layout.right)>.1)throw Error('shortcut row is not laid out as name, recorder, checkbox: '+JSON.stringify({layout,label,recorder:bounds(recorder),checkbox}));
window.__shortcutRect=bounds(recorder).toJSON();
recorder.click();
return {focusVisible};
"#,
        )?;
        if visible && clear_style["focusVisible"] != true {
            return Err("visible clear button did not show keyboard focus".into());
        }
        let suspended = || {
            state
                .shortcuts_suspended
                .load(std::sync::atomic::Ordering::SeqCst)
        };
        for _ in 0..40 {
            if suspended() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if !suspended() {
            return Err("recording did not suspend global shortcuts".into());
        }
        samples.push(wait_view_dom(app, "settings", r#"
const recorder=document.querySelector('[aria-label="Open Translation shortcut"]'), prompt=recorder.querySelector('.settings-recorder-prompt');
if(!prompt)return {ready:false};
const before=window.__shortcutRect, after=recorder.getBoundingClientRect();
const anchored=Math.abs(before.left-after.left)<.1&&['y','height'].every(key=>Math.abs(before[key]-after[key])<.1);
const clear=getComputedStyle(recorder.parentElement.querySelector('.settings-clear'));
if(clear.backgroundColor!=='rgba(0, 0, 0, 0)'||clear.borderTopColor!=='rgba(0, 0, 0, 0)')throw Error('clear action draws a surface during recording');
if(!anchored||after.width-prompt.getBoundingClientRect().width>34||clear.opacity!=='1'||recorder.querySelector('.keycaps')||prompt.textContent!=='Press shortcut')throw Error('shortcut recording loses alignment or fails to fit the prompt');
return {ready:true,anchored,idleWidth:before.width,recordingWidth:after.width};
"#)?);
        if visible {
            on_main(app, |app| hide(app, "settings"))?;
        } else {
            evaluate_view_dom(
                app,
                "settings",
                "window.dispatchEvent(new FocusEvent('blur'));return true;",
            )?;
        }
        for _ in 0..40 {
            if !suspended() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if suspended() {
            return Err("settings deactivation left global shortcuts suspended".into());
        }
        samples.push(serde_json::json!({"nativeWindowDeactivation": visible, "recordingEnded": !suspended()}));
        on_main(app, |app| show(app, "settings"))?;
        wait_view_dom(app, "settings", "return {ready:document.hasFocus()};")?;
        evaluate_view_dom(
            app,
            "settings",
            r#"
const recorder=document.querySelector('[aria-label="Copy shortcut"]');recorder.scrollIntoView();recorder.click();
recorder.dispatchEvent(new KeyboardEvent('keydown',{key:'Meta',code:'MetaLeft',bubbles:true,cancelable:true}));
return true;
"#,
        )?;
        wait_view_dom(
            app,
            "settings",
            r#"return {ready:document.querySelector('[aria-label="Copy shortcut"]')?.getAttribute('aria-pressed')==='true'};"#,
        )?;
        post_window_key(app, "settings", 8, 1 << 20, "c", "c", false)?;
        samples.push(wait_view_dom(app,"settings", r#"
const recorder=document.querySelector('[aria-label="Copy shortcut"]');return {ready:recorder?.textContent==='⌘C',recorded:recorder?.textContent};
"#)?);
        Ok::<(), String>(())
    })();
    DomCase {
        name: "translation settings: proxy validation and shortcut capture in WKWebView",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_clipboard_capture_setting(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |app| {
            crate::clipboard_preferences::recording_menu_item(app)
                .map_err(|err| err.to_string())?;
            create(app, "translation")?;
            let window = create(app, "settings")?;
            if std::env::var("NAMI_UI_TEST_VISIBLE").as_deref() == Ok("1") {
                window.show().map_err(|_| "window_show")?;
                window.set_focus().map_err(|_| "window_focus")?;
            }
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
        samples.push(wait_view_dom(app, "settings", r#"
const capture=document.querySelector('#clipboard-capture-heading'), excluded=document.querySelector('#clipboard-excluded-heading');
const row=document.querySelector('.clipboard-recording-types .settings-field');
const button=document.querySelector('[aria-label="About Excluded Apps"]'), list=document.querySelector('.clipboard-excluded-list');
if(!row||!button||!list)return {ready:false};
const rowSize=parseFloat(getComputedStyle(row).fontSize), sizes=[capture,excluded].map(heading=>parseFloat(getComputedStyle(heading).fontSize));
const clean=!excluded.closest('section').textContent.includes('Checks the active app only.');
const sizesOfIcons=[...list.querySelectorAll('img.clipboard-excluded-icon')].filter(img=>img.naturalWidth).map(img=>[img.naturalWidth,img.naturalHeight]);
if(!sizesOfIcons.length)return {ready:false,iconCount:list.querySelectorAll('img').length};
if(sizesOfIcons.some(size=>Math.max(...size)>40))throw Error('excluded app icon exceeds the thumbnail size');
if(capture.parentElement.querySelector('[role=switch]')||sizes.some(size=>size<=rowSize)||!clean)throw Error('clipboard section hierarchy or disclosure is incorrect');
if(document.querySelector('.settings-recorder'))throw Error('clipboard preferences still contain shortcut recorders');
const icons=[...document.querySelectorAll('.clipboard-settings .clipboard-icon-button')];
const iconColor=getComputedStyle(icons[0]).color;
if(icons.length<2||iconColor===getComputedStyle(row).color||icons.some(icon=>getComputedStyle(icon).color!==iconColor))throw Error('app remove and storage refresh must share a muted icon color');
window.__clipboardMutedIcon=iconColor;
button.click();
return {ready:true,sizes,rowSize,clean,sizesOfIcons,iconColor};
"#)?);
        samples.push(wait_view_dom(app, "settings", r#"
const panel=document.querySelector('.clipboard-tip-panel');if(!panel)return {ready:false};
const height=panel.getBoundingClientRect().height;
if(panel.textContent!=='Checks the active app only.'||panel.querySelector('button')||height>40)throw Error('excluded app tip is not compact');
document.querySelector('[aria-label="About Excluded Apps"]').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
return {ready:true,helpVisible:true,height};
"#)?);
        wait_view_dom(
            app,
            "settings",
            r#"return {ready:!document.querySelector('.clipboard-tip-panel')&&document.querySelector('[aria-label="About Excluded Apps"]').getAttribute('aria-expanded')==='false'};"#,
        )?;
        samples.push(wait_view_dom(app, "settings", r#"
const types=[...document.querySelectorAll('.clipboard-recording-types .settings-checkbox')];
if(types.length!==3||document.querySelector('#clipboard-image-index-heading')||document.querySelector('[aria-label="Index image text automatically"]'))return {ready:false};
const right=types[0].getBoundingClientRect().right, recordingRight=types.map(box=>box.getBoundingClientRect().right);
if(recordingRight.some(value=>Math.abs(value-right)>0.25))throw Error(`clipboard checkboxes must share a right edge: ${recordingRight} vs ${right}`);
return {ready:true,recordingRight};
"#)?);
        let state = app.state::<AppState>();
        {
            let mut database = state.database.lock().unwrap();
            let database = database.as_mut().ok_or("missing_test_database")?;
            for (id, is_favorite, is_pinned) in [
                ("settings-regular", false, false),
                ("settings-favorite", true, false),
                ("settings-pinned", false, true),
                ("settings-both", true, true),
            ] {
                database.clipboard_insert(&nami::clipboard::ClipboardRecord {
                    id: id.into(),
                    kind: "text".into(),
                    text: format!("Synthetic {id}"),
                    date: "2020-01-01T00:00:00Z".into(),
                    is_favorite,
                    is_pinned,
                    source_app: None,
                    image_path: None,
                    fingerprint: id.into(),
                })?;
            }
        }
        samples.push(wait_view_dom(
            app,
            "settings",
            r#"
const section=document.querySelector('#clipboard-search-heading').closest('section');
const trigger=section.querySelector('[aria-label="Search Rule: Exact Match"]');
const scopes=[...section.querySelectorAll('fieldset input')];
if(!trigger||scopes.length!==3||scopes.some(box=>!box.checked))return {ready:false};
trigger.click();return {ready:true,exactAndAllScopesDefault:true};
"#,
        )?);
        wait_view_dom(
            app,
            "settings",
            r#"
const options=[...document.querySelectorAll('[role=option]')];
const mixed=options.find(option=>option.textContent==='Mixed Match');
if(!mixed||options.length!==4)return {ready:false};
mixed.click();return {ready:true};
"#,
        )?;
        samples.push(wait_view_dom(
            app,
            "settings",
            r#"
const section=document.querySelector('#clipboard-search-heading').closest('section');
return {ready:!!section.querySelector('[aria-label="Search Rule: Mixed Match"]'),ruleSaved:true};
"#,
        )?);
        {
            let database = state.database.lock().unwrap();
            if crate::clipboard_preferences::ClipboardOptions::load(
                database.as_ref().ok_or("missing_test_database")?,
            )?
            .search
            .mode
                != nami::clipboard::ClipboardSearchMode::Mixed
            {
                return Err("clipboard search rule did not persist".into());
            }
        }
        wait_dom(app, "return {ready:!!window.__TAURI_INTERNALS__};")?;
        evaluate_dom(
            app,
            r#"
window.__clipboardSearchProbe=null;
window.__TAURI_INTERNALS__.invoke('clipboard_query',{query:'stngs-fvrt',favoritesOnly:true,kind:'text',cursor:null})
  .then(result=>window.__clipboardSearchProbe={ids:result.page.records.map(record=>record.id),total:result.page.totalRecords,noAlternate:result.alternate===null},error=>window.__clipboardSearchProbe={error:String(error)});
return true;
"#,
        )?;
        let search_probe = wait_dom(
            app,
            "return {ready:!!window.__clipboardSearchProbe,...window.__clipboardSearchProbe};",
        )?;
        if search_probe["ids"] != serde_json::json!(["settings-favorite"])
            || search_probe["total"] != 1
            || search_probe["noAlternate"] != true
        {
            return Err(format!(
                "clipboard query did not use the saved search rule: {search_probe}"
            ));
        }
        samples.push(search_probe);
        let set_recording = |enabled| {
            tauri::async_runtime::block_on(crate::clipboard_preferences::set_recording(
                app,
                Some(enabled),
            ))
        };
        set_recording(false)?;
        evaluate_view_dom(
            app,
            "settings",
            r#"
const image=document.querySelector('#clipboard-type-image'), row=image.closest('label'), bounds=image.getBoundingClientRect();
if(image.matches(':disabled')||!image.checked)throw Error('image capture must be configurable while paused');
const box=getComputedStyle(image), check=getComputedStyle(image,'::after');
const centered=check.left===check.right&&check.top===check.bottom&&check.marginLeft===check.marginRight&&check.marginTop===check.marginBottom;
const dimmed=color=>Number(color.split(',').at(-1).replace(')',''))<.6;
if(image.type!=='checkbox'||image.hasAttribute('role')||bounds.width!==16||bounds.height!==16||check.clipPath==='none'||!centered||!dimmed(box.borderTopColor)||!dimmed(check.backgroundColor)||getComputedStyle(row).opacity!=='1')throw Error('content type must use a muted checkbox with a centered check');
row.querySelector('.settings-field-content').click();return true;
"#,
        )?;
        samples.push(wait_view_dom(app, "settings", r#"
const image=document.querySelector('#clipboard-type-image');
if(image.checked||image.getAttribute('aria-disabled'))return {ready:false};
const opacity=Number(getComputedStyle(image.closest('label')).opacity), adjacentOpacity=getComputedStyle(document.querySelector('#clipboard-type-text').closest('label')).opacity;
if(!(opacity>0&&opacity<1)||adjacentOpacity!=='1'||image.matches(':disabled')||getComputedStyle(image,'::after').content!=='none')throw Error('unchecked row must dim without disabling it or affecting its neighbors');
return {ready:true,editableWhilePaused:!image.matches(':disabled'),opacity};
"#)?);
        let stored = state
            .database
            .lock()
            .unwrap()
            .as_ref()
            .ok_or("missing_test_database")?
            .preference("clipboard_recording_types")?
            .ok_or("missing_types")?;
        if serde_json::from_str::<serde_json::Value>(&stored).map_err(|error| error.to_string())?
            ["image"]
            != false
        {
            return Err("image setting did not persist through IPC".into());
        }
        for enabled in [true, false] {
            set_recording(enabled)?;
            let script = format!(
                r#"
const capture=document.querySelector('#clipboard-capture-heading').closest('section');
const image=capture.querySelector('#clipboard-type-image'), paused=!!capture.querySelector('[role=status]');
return {{ready:paused!=={enabled}&&!image.checked&&!image.matches(':disabled'),paused,imageEnabled:image.checked}};
"#
            );
            samples.push(wait_view_dom(app, "settings", &script)?);
            let menu = state
                .clipboard_recording_menu
                .lock()
                .unwrap()
                .clone()
                .ok_or("missing recording menu")?;
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
                || menu.text().map_err(|error| error.to_string())?
                    != crate::clipboard_preferences::recording_menu_label(enabled)
            {
                return Err("recording menu, runtime state and stored preference diverged".into());
            }
        }
        let mut picker = serde_json::Value::Null;
        for _ in 0..80 {
            picker = evaluate_view_dom(app, "settings", "const button=document.querySelector('button[aria-label=\"When Activating an Item: Paste into Previous App\"]');if(!button)return {ready:false};if(button.getAttribute('aria-expanded')==='false')button.click();return {ready:!!document.querySelector('.settings-popover')};")?;
            if picker["ready"] == true {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if picker["ready"] != true {
            return Err("settings behavior dropdown did not open".into());
        }
        let scrolled = evaluate_view_dom(app, "settings", "document.querySelector('.settings-content').dispatchEvent(new Event('scroll'));return true;")?;
        if scrolled != true {
            return Err("settings scroll did not dispatch".into());
        }
        let mut dismissed = false;
        for _ in 0..80 {
            if evaluate_view_dom(app, "settings", "return !document.querySelector('.settings-popover') && document.querySelector('button[aria-label=\"When Activating an Item: Paste into Previous App\"]').getAttribute('aria-expanded')==='false';")? == true {
                dismissed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if !dismissed {
            return Err("settings dropdown stayed open after scrolling".into());
        }
        samples.push(serde_json::json!({"settingsScrollDismissedPicker": dismissed}));
        evaluate_view_dom(
            app,
            "settings",
            "document.querySelector('.clipboard-storage-value button').click();return true;",
        )?;
        wait_view_dom(
            app,
            "settings",
            r#"
const storage=document.querySelector('.clipboard-storage-value [aria-live]');
if(!storage||!(parseFloat(storage.textContent)>0))return {ready:false};
window.__storageBeforeCleanup=storage.textContent;
return {ready:true};
"#,
        )?;
        evaluate_view_dom(
            app,
            "settings",
            "const trigger=document.querySelector('.clipboard-clear-records');if(!trigger.parentElement.contains(document.querySelector('#clipboard-storage-heading')))throw Error('clear action is outside the title row');trigger.click();return true;",
        )?;
        let mut confirmation = serde_json::Value::Null;
        for _ in 0..80 {
            confirmation = evaluate_view_dom(app, "settings", "const dialog=document.querySelector('dialog[open]'), cancel=dialog?.querySelector('[data-initial-focus]');return {open:!!dialog,initialFocus:cancel?.textContent,active:document.hasFocus()?document.activeElement?.textContent:null,outline:cancel&&getComputedStyle(cancel).outlineStyle,decoration:cancel&&getComputedStyle(cancel).textDecorationLine};")?;
            if confirmation["open"] == true {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if confirmation["open"] != true
            || confirmation["initialFocus"] != "CancelEsc"
            || confirmation["outline"] != "none"
            || confirmation["decoration"] != "none"
            || (!confirmation["active"].is_null() && confirmation["active"] != "CancelEsc")
        {
            return Err(format!(
                "cleanup confirmation must start on Cancel: {confirmation}"
            ));
        }
        evaluate_view_dom(
            app,
            "settings",
            "document.querySelector('dialog [data-initial-focus]').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));return true;",
        )?;
        samples.push(confirmation);
        evaluate_view_dom(
            app,
            "settings",
            "const trigger=document.querySelector('.clipboard-clear-records');trigger.focus();const style=getComputedStyle(trigger);if(style.outlineStyle!=='none'||style.textDecorationLine!=='none')throw Error('clear action still draws a border or underline');trigger.click();return true;",
        )?;
        wait_view_dom(
            app,
            "settings",
            r#"
const dialog=document.querySelector('dialog[open]');if(!dialog)return {ready:false};
const options=[...dialog.querySelectorAll('input[type=checkbox]')], confirm=dialog.querySelector('.clipboard-confirm-action');
if(options.length!==3||options.some(option=>option.checked)||!confirm.disabled||dialog.querySelector('p'))throw Error('cleanup must show just three unselected categories and actions');
if(dialog.getBoundingClientRect().width>330||dialog.getBoundingClientRect().height>185||confirm.querySelector('kbd')?.textContent!=='↵')throw Error('cleanup dialog should be compact and show Enter');
const counts=[...dialog.querySelectorAll('.clipboard-cleanup-count')].map(node=>Number(node.textContent));
if(counts.join(',')!=='1,2,2')throw Error('favorite and pinned rows must count in both categories');
options[2].click();return {ready:true};
"#,
        )?;
        wait_view_dom(
            app,
            "settings",
            r#"const confirm=document.querySelector('dialog[open] .clipboard-confirm-action');return {ready:!!confirm&&!confirm.disabled};"#,
        )?;
        evaluate_view_dom(
            app,
            "settings",
            "const dialog=document.querySelector('dialog[open]');dialog.tabIndex=-1;dialog.focus();if(document.activeElement!==dialog)throw Error('cleanup dialog did not receive focus');return true;",
        )?;
        if std::env::var("NAMI_UI_TEST_VISIBLE").as_deref() == Ok("1") {
            wait_view_dom(app, "settings", "return {ready:document.hasFocus()};")?;
            post_window_key(app, "settings", 36, 0, "\r", "\r", false)?;
        } else {
            evaluate_view_dom(app, "settings", "document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}));return true;")?;
        }
        samples.push(wait_view_dom(app, "settings", r#"
const section=document.querySelector('[aria-labelledby=clipboard-storage-heading]');
const before=window.__storageBeforeCleanup, after=section.querySelector('.clipboard-storage-value [aria-live]').textContent;
return {ready:!document.querySelector('dialog[open]')&&before!==after&&parseFloat(after)>=0&&!section.querySelector('[role=status], [role=alert]'),before,after};
"#)?);
        evaluate_view_dom(
            app,
            "settings",
            r#"
const image=document.querySelector('#clipboard-type-image');
if(image.matches(':disabled')||image.checked)throw Error('pause must retain the editable image preference');
document.querySelector('button[aria-label="HotKey"]').click();return true;
"#,
        )?;
        wait_view_dom(
            app,
            "settings",
            r#"
const groups=[...document.querySelectorAll('.hotkey-group')];
if(!groups.length)return {ready:false};
if(groups.map(group=>group.children.length).join()!=='16,7,6')throw Error('shortcut groups must contain the general and panel actions');
const allRows=[...document.querySelectorAll('.hotkey-group .settings-shortcut-row')];
const rows=allRows.filter(row=>row.querySelector('.settings-recorder'));
if(rows.length!==allRows.length)throw Error('every shortcut needs a recorder');
for(const [name,key] of [['Navigate Up','↑'],['Navigate Down','↓'],['Previous Category','←'],['Next Category','→'],['Paste (Context Menu)','↵'],['Paste as Plain Text (Context Menu)','⌘↵'],['Save Image (Context Menu)','⌘S']]) {
  if(rows.find(row=>row.querySelector('.settings-field-content')?.textContent===name)?.querySelector('.keycaps')?.textContent!==key)throw Error('missing editable default: '+name);
}
if(rows.some(row=>!row.querySelector('.settings-recorder')||!row.querySelector('input[type=checkbox]:checked')))throw Error('each shortcut needs an available recorder and an enabled checkbox');
for(const selector of ['.settings-recorder','.keycaps kbd','input']) {
  const starts=rows.map(row=>row.querySelector(selector)).filter(Boolean).map(control=>control.getBoundingClientRect().left);
  if(Math.max(...starts)-Math.min(...starts)>.1)throw Error('shortcut column is not left aligned: '+selector+' '+starts);
}
if(getComputedStyle(document.querySelector('.hotkey-group .settings-clear')).color!==window.__clipboardMutedIcon)throw Error('shortcut clear must share the muted icon color');
const button=document.querySelector('[aria-label="Open Clipboard History shortcut"]');
if(!groups[1].contains(button)||button.disabled||!groups[2].contains(document.querySelector('[aria-label="Open Translation shortcut"]')))throw Error('global launch shortcuts must be in their own groups');
return {ready:true};
"#,
        )?;
        samples.push(evaluate_view_dom(
            app,
            "settings",
            r#"
window.clipboardSettingsProbe={done:false,stage:"shortcut"};
(async()=>{
  const invoke=(name,args)=>window.__TAURI_INTERNALS__.invoke(name,args).catch(error=>{
    window.clipboardSettingsProbe.lastRejectedCommand=name;
    throw error;
  });
  if((await invoke('app_snapshot')).clipboardControls.recordsOpenFocus!=='search')throw Error('expected search focus by default');
  await invoke('records_open_focus_set',{focus:'list'});
  if((await invoke('app_snapshot')).clipboardControls.recordsOpenFocus!=='list')throw Error('failed to change initial focus');
  await invoke('records_open_focus_set',{focus:'search'});
  const binding={code:'KeyJ',modifiers:['control','alt','shift']};
  const set=binding=>invoke('clipboard_shortcut_set',{binding});
  await set({code:'F6',modifiers:[]});
  if((await invoke('app_snapshot')).clipboardBinding.code!=='F6')throw Error('clipboard did not save a plain function key');
  const selection=binding=>invoke('translation_shortcut_set',{action:'selection',binding});
  await selection({code:'F7',modifiers:[]});
  let registration;
  try {await selection({code:'ShiftLeft',modifiers:[]});} catch(error) {registration=String(error);}
  const keys=(await invoke('app_snapshot')).translationPreferences.shortcuts, selected=keys.selection;
  if(keys.openTranslation?.code!=='KeyT')throw Error('remapping Autofill Input also changed Open Translation');
  if(registration!=='shortcut_registration'||selected.code!=='F7')throw Error('global registration did not preserve the previous binding: '+JSON.stringify({registration,selected}));
  await selection({code:'KeyT',modifiers:['meta']});
  await set(binding);
  let conflict;
  try {await set({code:'KeyT',modifiers:['meta']});} catch(error) {conflict=String(error);}
  const snapshot=await invoke('app_snapshot');
  const local=(action,binding,enabled)=>invoke('clipboard_record_shortcut_set',{action,binding,enabled});
  let localConflict;
  try {await local('pin',{code:'KeyF',modifiers:['alt']});} catch(error) {localConflict=String(error);}
  if(localConflict!=='shortcut_clipboard_panel')throw Error('local shortcut accepted a duplicate favorite binding');
  await local('pin',null,false);
  await local('pin',{code:'KeyG',modifiers:['alt']});
  for (const action of ['copy','paste']) {
    let accepted=false;
    try {await local(action,null,false);accepted=true;} catch (_) {}
    if(accepted)throw Error('retired shortcut is writable: '+action);
  }
  const controls=(await invoke('app_snapshot')).clipboardControls;
  if(controls.shortcuts.pin.enabled||controls.shortcuts.pin.binding.code!=='KeyG')throw Error('local shortcut edit unexpectedly enabled or lost the binding');
  for (const code of ['Space','F6']) {
    await local('preview',{code,modifiers:[]});
    const saved=(await invoke('app_snapshot')).clipboardControls.shortcuts.preview.binding;
    if(saved.code!==code||saved.modifiers.length)throw Error('recorder did not keep a bare '+code);
  }
  for (const [action,code] of [['preview','F6'],['switchView','KeyJ'],['copyRecord','KeyC'],['note','KeyN'],['ocr','KeyO'],['translate','KeyT'],['favorite','KeyH'],['delete','KeyM'],['pasteRecord','KeyL'],['pastePlain','KeyI'],['saveImage','KeyU']]) {
    await local(action,null,false);
    await local(action,{code,modifiers:['alt']});
    const saved=(await invoke('app_snapshot')).clipboardControls.shortcuts[action];
    if(saved.enabled||saved.binding.code!==code)throw Error('settings failed to save '+action+' while disabled');
  }
  window.clipboardSettingsProbe.stage='options';
  const defaults=await invoke('clipboard_options_get');
  if(defaults.activation!=='paste'||defaults.historyPolicy.limit!==500||!defaults.excludedApps.some(app=>app.bundleId==='com.apple.Passwords'))throw Error('unexpected default options');
  if('autoImageIndex' in defaults)throw Error('retired image index option is still exposed');
  let retiredImageIndex=false;
  try {await invoke('clipboard_auto_image_index_set',{enabled:false});} catch (_) {retiredImageIndex=true;}
  if(!retiredImageIndex)throw Error('retired image index command is still callable');
  await invoke('clipboard_behavior_set',{activation:'copy'});
  if((await invoke('clipboard_options_get')).activation!=='copy')throw Error('copy preference did not update');
  const excluded=await invoke('clipboard_excluded_app_remove',{bundleId:'com.apple.Passwords'});
  if(excluded.some(app=>app.bundleId==='com.apple.Passwords'))throw Error('exclusion was not removed');
  window.clipboardSettingsProbe.stage='storage';
  const view='clipboard', storage=await invoke('records_storage_info',{view});
  const policy={limit:1};
  window.clipboardSettingsProbe.stage='preview';
  const preview=await invoke('records_cleanup_preview',{view,policy});
  if(preview.count!==0||storage.limit!==500||storage.bytes<1)throw Error('clipboard count-only default or old row is missing');
  let stale;
  try {await invoke('records_cleanup_apply',{view,policy,token:'0'.repeat(64)});} catch(error) {stale=String(error);}
  if(stale!=='clipboard_cleanup_changed')throw Error('cleanup accepted a mismatched preview');
  window.clipboardSettingsProbe.stage='apply';
  const result=await invoke('records_cleanup_apply',{view,policy,token:preview.token});
  if(result.removed!==preview.count||result.mediaCleanupFailed)throw Error('cleanup did not match the preview');
  const saved=await invoke('clipboard_options_get');
  if(saved.historyPolicy.limit!==1)throw Error('confirmed count was not saved');
  const translation={view:'translation',policy:{limit:3}};
  const before=await invoke('records_storage_info',{view:translation.view});
  const translationPreview=await invoke('records_cleanup_preview',translation);
  if(before.limit!==500||before.bytes===0||translationPreview.count<1)throw Error('translation history is missing');
  const translationResult=await invoke('records_cleanup_apply',{...translation,token:translationPreview.token});
  const after=await invoke('records_storage_info',{view:translation.view});
  if(translationResult.removed!==translationPreview.count||after.limit!==3||after.bytes>=before.bytes||(await invoke('records_storage_info',{view})).limit!==1)throw Error('translation cleanup or scope is incorrect');
  window.clipboardSettingsProbe={done:true,conflict,afterConflict:snapshot.clipboardBinding,removed:result.removed};
})().catch(error=>window.clipboardSettingsProbe={...window.clipboardSettingsProbe,done:true,error:String(error)});
return {paused:true};
"#,
        )?);
        let mut probe = serde_json::Value::Null;
        for _ in 0..80 {
            probe = evaluate_view_dom(app, "settings", "return window.clipboardSettingsProbe;")?;
            if probe["done"] == true {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if probe["conflict"] != "shortcut_conflict:openTranslation"
            || probe["afterConflict"]["code"] != "KeyJ"
        {
            return Err(format!("settings IPC or shortcut probe failed: {probe}"));
        }
        on_main(app, |app| {
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            let expected = crate::binding_shortcut(&nami::shortcuts::KeyBinding::new(
                "KeyJ",
                &["control", "alt", "shift"],
            ))?;
            if !app.global_shortcut().is_registered(expected)
                || app.global_shortcut().is_registered(crate::binding_shortcut(
                    &nami::shortcuts::clipboard_binding(),
                )?)
            {
                return Err("the native registry did not replace the previous shortcut".into());
            }
            Ok(())
        })?;
        evaluate_view_dom(
            app,
            "settings",
            r#"
const row=document.querySelector('[aria-label="Open Clipboard History shortcut"]').closest('.settings-shortcut-row');
window.__shortcutGeometry=()=>[...row.querySelectorAll('.settings-recorder, kbd, .settings-clear svg, input')].map(node=>{
  const {x,y,width,height}=node.getBoundingClientRect(); return [x,y,width,height];
});
window.__shortcutGeometryBefore=window.__shortcutGeometry();
return true;
"#,
        )?;
        for enabled in [false, true] {
            let key = if enabled { "K" } else { "J" };
            evaluate_view_dom(app, "settings", "document.querySelector('[aria-label=\"Open Clipboard History shortcut\"]').closest('.settings-shortcut-row').querySelector('input[type=checkbox]').click();return true;")?;
            let script = format!(
                r#"
const box=document.querySelector('[aria-label="Open Clipboard History shortcut"]').closest('.settings-shortcut-row').querySelector('input[type=checkbox]');
const row=box.closest('.settings-shortcut-row'), recorder=row.querySelector('.settings-recorder');
if(box.checked!=={enabled}||box.getAttribute('aria-disabled'))return {{ready:false}};
const before=window.__shortcutGeometryBefore, after=window.__shortcutGeometry();
const stable=after.length===before.length&&after.every((rect,i)=>rect.every((n,j)=>Math.abs(n-before[i][j])<.1));
if(!stable||recorder.disabled||!recorder.querySelector('.keycaps')?.textContent.endsWith('{key}'))throw Error('changing enabled state moved the shortcut or disabled its recorder');
return {{ready:true,stable,opacity:Number(getComputedStyle(row).opacity)}};
"#
            );
            let sample = wait_view_dom(app, "settings", &script)?;
            if (sample["opacity"] == 1) != enabled {
                return Err("only disabled shortcut rows should be dimmed".into());
            }
            let stored = state
                .database
                .lock()
                .unwrap()
                .as_ref()
                .ok_or("missing_test_database")?
                .preference("clipboard_shortcut_enabled")?;
            if stored.as_deref() != Some(if enabled { "true" } else { "false" }) {
                return Err("clipboard shortcut enabled state did not persist".into());
            }
            if !enabled {
                evaluate_view_dom(app, "settings", "document.querySelector('[aria-label=\"Open Clipboard History shortcut\"]').click();return true;")?;
                wait_view_dom(
                    app,
                    "settings",
                    r#"
const prompt=document.querySelector('[aria-label="Open Clipboard History shortcut"] .settings-recorder-prompt');
if(!prompt)return {ready:false};
prompt.dispatchEvent(new KeyboardEvent('keydown',{key:'K',code:'KeyK',ctrlKey:true,altKey:true,shiftKey:true,bubbles:true}));
return {ready:true};
"#,
                )?;
                samples.push(wait_view_dom(app, "settings", r#"
const row=document.querySelector('[aria-label="Open Clipboard History shortcut"]').closest('.settings-shortcut-row');
if(!row.querySelector('.keycaps')?.textContent.endsWith('K'))return {ready:false};
if(row.querySelector('input').checked)throw Error('editing unexpectedly enabled the shortcut');
window.__shortcutGeometryBefore=window.__shortcutGeometry();
return {ready:true,editedWhileUnchecked:true};
"#)?);
            }
            on_main(app, move |app| {
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                let registered = |code| -> Result<bool, String> {
                    let binding =
                        nami::shortcuts::KeyBinding::new(code, &["control", "alt", "shift"]);
                    Ok(app
                        .global_shortcut()
                        .is_registered(crate::binding_shortcut(&binding)?))
                };
                if registered("KeyJ")? || registered("KeyK")? != enabled {
                    return Err("checkbox and edited global shortcut registry diverged".into());
                }
                Ok(())
            })?;
            samples.push(sample);
        }
        {
            let database = state.database.lock().unwrap();
            let database = database.as_ref().ok_or("missing_test_database")?;
            if database.clipboard_record("settings-regular").is_err()
                || database.clipboard_record("settings-both").is_ok()
                || !database.clipboard_record("settings-favorite")?.is_favorite
                || database.clipboard_record("settings-pinned").is_ok()
                || database
                    .preference("clipboard_excluded_apps")?
                    .is_none_or(|stored| stored.contains("iconDataUrl"))
                || !database
                    .preference("clipboard_record_shortcuts")?
                    .is_some_and(|value| value.contains("KeyG"))
                || database.preference("clipboard_activation")?.as_deref() != Some("\"copy\"")
            {
                return Err(
                    "cleanup, exclusion or activation violated durable clipboard state".into(),
                );
            }
        }
        evaluate_view_dom(
            app,
            "settings",
            r#"
document.querySelector('#clipboard-shortcut-heading').parentElement.querySelector('button').click();return true;
"#,
        )?;
        samples.push(wait_view_dom(
            app,
            "settings",
            r#"
const group=document.querySelector('[role=group][aria-labelledby=clipboard-shortcut-heading]');
const bindings=[...group.querySelectorAll('.settings-recorder')].map(button=>button.textContent);
const enabled=[...group.querySelectorAll('input[type=checkbox]')].every(input=>input.checked);
const general=document.querySelector('[role=group][aria-labelledby=general-shortcut-heading]');
const kept=[...general.querySelectorAll('.settings-recorder')].some(button=>button.textContent==='⌥G'&&!button.closest('.settings-shortcut-row').querySelector('input').checked);
const preview=general.querySelector('[aria-label="Preview shortcut"]');
return {ready:enabled&&kept&&preview?.textContent==='⌥F6'&&!preview.closest('.settings-shortcut-row').querySelector('input').checked&&bindings.join()===['⌃`','⌘T','⌘O','⇧⌘O','↵','⌘↵','⌘S'].join(),bindings,kept};
"#,
        )?);
        {
            let database = state.database.lock().unwrap();
            let database = database.as_ref().ok_or("missing_test_database")?;
            if database
                .preference("clipboard_shortcut_enabled")?
                .as_deref()
                != Some("true")
                || database
                    .preference("clipboard_shortcut")?
                    .is_none_or(|stored| !stored.contains("Backquote"))
                || database
                    .preference("clipboard_record_shortcuts")?
                    .is_none_or(|stored| !stored.contains("KeyM") || !stored.contains("KeyG"))
            {
                return Err("clipboard shortcut defaults were not saved".into());
            }
        }
        evaluate_view_dom(
            app,
            "settings",
            r#"
window.__customSelection = false;
window.__TAURI_INTERNALS__.invoke('translation_shortcut_set',{action:'selection',binding:{code:'F7',modifiers:[]}})
  .then(()=>window.__TAURI_INTERNALS__.invoke('translation_shortcut_set',{action:'openTranslation',binding:{code:'F8',modifiers:[]}}))
  .then(()=>window.__customSelection=true,error=>window.__customSelection=String(error));return true;
"#,
        )?;
        wait_view_dom(app, "settings", "if(typeof window.__customSelection==='string')throw Error(window.__customSelection);return {ready:window.__customSelection===true};")?;
        let default_claim = on_main(app, |_| {
            let mut reference = std::ptr::null_mut();
            let code = unsafe {
                RegisterEventHotKey(
                    17,
                    1 << 8,
                    TestHotKeyId {
                        signature: u32::from_be_bytes(*b"NAMI"),
                        id: 991,
                    },
                    GetApplicationEventTarget(),
                    0,
                    &mut reference,
                )
            };
            if code != 0 || reference.is_null() {
                return Err(format!("cannot claim test shortcut: {code}"));
            }
            Ok(reference as usize)
        })?;
        let rejected_restore = (|| {
            evaluate_view_dom(
                app,
                "settings",
                r#"
const invoke=window.__TAURI_INTERNALS__.invoke;
window.__occupiedRestore=null;
(async()=>{
  let error;try {await invoke('translation_shortcuts_restore',{general:false});} catch(value) {error=String(value);}
  const preferences=(await invoke('app_snapshot')).translationPreferences;
  window.__occupiedRestore={error,selection:preferences.shortcuts.selection?.code,open:preferences.shortcuts.openTranslation?.code};
})();return true;
"#,
            )?;
            let observed = wait_view_dom(
                app,
                "settings",
                "return {ready:!!window.__occupiedRestore,...window.__occupiedRestore};",
            )?;
            if observed["error"] != "shortcut_registration"
                || observed["selection"] != "F7"
                || observed["open"] != "F8"
            {
                return Err(format!("occupied default changed selection: {observed}"));
            }
            on_main(app, |app| {
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                if !app.global_shortcut().is_registered(crate::binding_shortcut(
                    &nami::shortcuts::KeyBinding::new("F7", &[]),
                )?) {
                    return Err("occupied default unregistered the previous shortcut".into());
                }
                Ok(())
            })?;
            let database = state.database.lock().unwrap();
            let stored = database
                .as_ref()
                .ok_or("missing_test_database")?
                .preference("translation_shortcuts")?;
            if !stored.is_some_and(|value| value.contains("F7")) {
                return Err("occupied default replaced the saved shortcut".into());
            }
            Ok(observed)
        })();
        on_main(app, move |_| {
            let code = unsafe { UnregisterEventHotKey(default_claim as *mut std::ffi::c_void) };
            if code != 0 {
                return Err(format!("cannot release test shortcut: {code}"));
            }
            Ok(())
        })?;
        samples.push(rejected_restore?);
        evaluate_view_dom(
            app,
            "settings",
            r#"
const invoke=window.__TAURI_INTERNALS__.invoke;
window.__groupRestore=null;
(async()=>{
  const local=(action,binding,enabled)=>invoke('clipboard_record_shortcut_set',{action,binding,enabled});
  const set=(action,binding,enabled)=>invoke('translation_shortcut_set',{action,binding,enabled});
  const rejected=async request=>{try{await request();}catch(error){if(String(error)==='shortcut_clipboard_panel')return;throw error;}throw Error('same-scope record shortcut conflict accepted');};
  const initial=await invoke('app_snapshot');
  await rejected(()=>set('translate',initial.clipboardControls.shortcuts.preview.binding));
  await rejected(()=>local('preview',initial.translationPreferences.shortcuts.translate));
  const reuse={code:'KeyB',modifiers:['alt']};
  await set('copyPrimary',reuse);
  await local('ocr',reuse);
  await rejected(()=>local('preview',reuse));
  await rejected(()=>set('focusSearch',reuse));
  await rejected(()=>set('copySecondary',initial.clipboardControls.shortcuts.preview.binding));
  await local('ocr',initial.clipboardControls.shortcuts.ocr.binding);
  await set('copyPrimary',initial.translationPreferences.shortcuts.copyPrimary);
  await set('translate',initial.clipboardControls.shortcuts.pastePlain.binding);
  await local('pastePlain',initial.translationPreferences.shortcuts.translate);
  await set('focusSearch',{code:'KeyU',modifiers:['meta']});
  await set('openTranslation',{code:'F8',modifiers:[]});
  await set('openTranslation',null,false);
  await set('focusSearch',null,false);
  await set('translate',{code:'KeyZ',modifiers:['alt']});
  await set('translate',null,false);
  await set('selection',null,false);
  await local('ocr',{code:'KeyF',modifiers:['meta']});
  let conflict;
  try {await invoke('translation_shortcuts_restore',{general:true});} catch(error) {conflict=String(error);}
  const read=()=>invoke('app_snapshot');
  let p=await read();
  if(conflict!=='shortcut_clipboard_panel'||p.translationPreferences.shortcuts.focusSearch.code!=='KeyU'||p.clipboardControls.shortcuts.pin.binding.code!=='KeyG')throw Error('conflicting restore changed saved shortcuts');
  await local('ocr',{code:'KeyO',modifiers:['shift','meta']});
  await local('preview',{code:'KeyY',modifiers:['alt']});
  const waitRestored=async(predicate)=>{
    for(let i=0;i<60;i++){
      const snapshot=await read();
      if(predicate(snapshot))return snapshot;
      const alert=document.querySelector('[role=alertdialog]');if(alert)throw Error(alert.textContent);
      await new Promise(r=>setTimeout(r,10));
    }
    throw Error('restore did not publish new bindings');
  };
  document.querySelector('#general-shortcut-heading').parentElement.querySelector('button').click();
  p=await waitRestored(snapshot=>snapshot.translationPreferences.shortcuts.focusSearch.code==='KeyF');
  let t=p.translationPreferences,c=p.clipboardControls.shortcuts;
  if(t.shortcuts.focusSearch.code!=='KeyF'||t.disabledShortcuts.includes('focusSearch')||c.switchView.binding.code!=='Tab'||!c.switchView.enabled||c.pin.binding.code!=='KeyP'||!c.pin.enabled||c.preview.binding.code!=='KeyY'||c.preview.binding.modifiers[0]!=='meta'||!c.preview.enabled)throw Error('general defaults were not restored');
  if(t.shortcuts.translate.code!=='KeyZ'||!t.disabledShortcuts.includes('translate')||t.selectionEnabled)throw Error('general restore changed another section');
  await set('focusSearch',{code:'KeyU',modifiers:['meta']});
  document.querySelector('#translation-shortcuts-heading').parentElement.querySelector('button').click();
  p=await waitRestored(snapshot=>snapshot.translationPreferences.shortcuts.translate.code==='Enter');t=p.translationPreferences;
  if(t.shortcuts.focusSearch.code!=='KeyU'||t.shortcuts.translate.code!=='Enter'||t.disabledShortcuts.includes('translate')||!t.selectionEnabled||t.shortcuts.openTranslation?.code!=='KeyT'||t.disabledShortcuts.includes('openTranslation')||p.clipboardControls.shortcuts.preview.binding.modifiers[0]!=='meta')throw Error('translation restore changed another section or missed a translation default');
  window.__groupRestore={done:true};
})().catch(error=>window.__groupRestore={done:true,error:String(error)});
return true;
"#,
        )?;
        samples.push(wait_view_dom(app, "settings", "if(window.__groupRestore?.error)throw Error(window.__groupRestore.error);return {ready:window.__groupRestore?.done===true,shortcutGroupsRestored:true};")?);
        evaluate_dom(
            app,
            r#"
window.clipboardActivationProbe=null;
window.__TAURI_INTERNALS__.invoke('clipboard_record_use',{recordId:'settings-favorite',action:'activate'}).then(
  result=>window.clipboardActivationProbe=result,
  error=>window.clipboardActivationProbe={error:String(error)}
);
return true;
"#,
        )?;
        let mut activation = serde_json::Value::Null;
        for _ in 0..80 {
            activation = evaluate_dom(app, "return window.clipboardActivationProbe;")?;
            if !activation.is_null() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if activation["status"] != "copied" || activation["copyStatus"] != "current" {
            return Err(format!("Copy Only activated the paste path: {activation}"));
        }
        samples.push(activation);
        samples.push(probe);
        Ok::<(), String>(())
    })();
    DomCase {
        name: "clipboard settings: recording, shortcuts, storage and protected cleanup through real WebView IPC",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn verify_record_beam_motion(
    app: &AppHandle,
    samples: &mut Vec<serde_json::Value>,
) -> Result<(), String> {
    let sample = evaluate_view_dom(
        app,
        "clipboard",
        r#"
const values=[];
for(const width of [300,900]) {
 const track=document.createElement('div');track.className='activity-beam';track.dataset.visible='true';track.style.width=width+'px';track.append(document.createElement('span'));document.body.append(track);
 const beam=track.firstElementChild,style=getComputedStyle(beam),animations=beam.getAnimations();
 if(matchMedia('(prefers-reduced-motion: reduce)').matches){values.push({reduced:true,stopped:animations.length===0});track.remove();continue;}
 const animation=animations[0];if(!animation){track.remove();return {error:'missing animation'};}animation.pause();
 const frames=[0,300,600].map(time=>{animation.currentTime=time;const box=beam.getBoundingClientRect();return {left:box.left,width:box.width,opacity:Number(getComputedStyle(beam).opacity)};});
 getComputedStyle(track).opacity;track.dataset.visible='false';getComputedStyle(track).opacity;
 const transition=track.getAnimations().find(value=>value.transitionProperty==='opacity');if(!transition){track.remove();return {error:'missing exit transition'};}transition.pause();
 const fade=[0,80,160].map(time=>{transition.currentTime=time;return Number(getComputedStyle(track).opacity);});
 values.push({width,count:animations.length,name:style.animationName,timing:style.animationTimingFunction,frames,fade,duration:style.animationDuration,opacityOnly:animation.effect.getKeyframes().every(frame=>!('transform' in frame)&&!('left' in frame)&&!('width' in frame)&&'opacity' in frame),height:style.height,exitTransition:getComputedStyle(track).transitionDuration});track.remove();
}
return {values};
"#,
    )?;
    for value in sample["values"].as_array().ok_or("beam samples")? {
        if value["reduced"] == true {
            if value["stopped"] != true {
                return Err(format!("reduced motion beam: {sample}"));
            }
        } else {
            let frames = value["frames"].as_array().ok_or("glow frames")?;
            let fade = value["fade"].as_array().ok_or("glow fade")?;
            if value["count"] != 1
                || value["name"] != "beam-breathe"
                || value["timing"] != "ease-in-out"
                || value["duration"] != "1.2s"
                || value["opacityOnly"] != true
                || value["height"] != "1.5px"
                || value["exitTransition"] != "0.16s"
                || frames.iter().any(|frame| {
                    frame["left"] != frames[0]["left"]
                        || (frame["width"].as_f64().unwrap_or(0.0)
                            - value["width"].as_f64().unwrap_or(0.0))
                        .abs()
                            > 0.5
                })
                || frames[0]["opacity"].as_f64() >= frames[1]["opacity"].as_f64()
                || frames[1]["opacity"].as_f64() >= frames[2]["opacity"].as_f64()
                || fade[0].as_f64() != Some(1.0)
                || fade[1]
                    .as_f64()
                    .is_none_or(|opacity| opacity <= 0.0 || opacity >= 1.0)
                || fade[2].as_f64() != Some(0.0)
            {
                return Err(format!("glow continuity or fade: {sample}"));
            }
        }
    }
    samples.push(sample);
    Ok(())
}

fn verify_clipboard_action_feedback(
    app: &AppHandle,
    samples: &mut Vec<serde_json::Value>,
) -> Result<(), String> {
    let record = nami::clipboard::ClipboardRecord {
        id: "action-feedback".into(),
        kind: "text".into(),
        text: "Synthetic copy feedback".into(),
        date: "2099-01-01T00:00:00Z".into(),
        is_favorite: false,
        is_pinned: false,
        source_app: None,
        image_path: None,
        fingerprint: "action-feedback".into(),
    };
    app.state::<AppState>()
        .database
        .lock()
        .unwrap()
        .as_mut()
        .ok_or("test database")?
        .clipboard_insert(&record)?;
    crate::clipboard_runtime::changed(app)?;
    records_mode(app, "clipboard")?;
    wait_records(
        app,
        "return {ready:!!document.querySelector('[data-record-id=action-feedback]')};",
    )?;
    samples.push(wait_records(app, r#"
const search=document.querySelector('.clipboard-panel input[type=search]'), row=document.querySelector('[data-record-id=action-feedback][data-keyboard-current]');
if(!row)return {ready:false};
const button=row.querySelector('.history-row-select');
const probe=row.cloneNode(false);delete probe.dataset.keyboardCurrent;probe.dataset.recordTarget='true';row.after(probe);
const hoverBackground=getComputedStyle(probe).backgroundColor;probe.remove();
const sameHighlight=()=>getComputedStyle(row).backgroundColor===hoverBackground&&getComputedStyle(row).boxShadow==='none';
if(!window.__clipboardActionFocusProbe){
  if(document.activeElement!==search)return {ready:false};
  if(hoverBackground==='rgba(0, 0, 0, 0)'||row.hasAttribute('data-record-target')||getComputedStyle(row).backgroundColor!=='rgba(0, 0, 0, 0)')throw Error('search focus must not highlight the initial keyboard target');
  window.__clipboardActionFocusProbe=true;
  button.focus();
}
if(document.activeElement!==button||getComputedStyle(button).outlineStyle!=='none')throw Error('record button must receive focus without a frame');
if(!sameHighlight())return {ready:false};
delete window.__clipboardActionFocusProbe;
search.focus({preventScroll:true});
if(!search.dispatchEvent(new KeyboardEvent('keydown',{key:' ',code:'Space',bubbles:true,cancelable:true})))throw Error('search cannot type a space');
return {ready:true,keyboardTarget:row.dataset.recordId};
"#)?);
    on_main(app, |app| {
        app.state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .history_pinned = false;
        apply_translation(app)
    })?;
    samples.push(wait_records(
        app,
        r#"
const row=document.querySelector('[data-record-id=action-feedback]');
return {ready:!!row?.querySelector('[data-action=copy] button:not(:disabled)'),standaloneCopy:true};
"#,
    )?);
    evaluate_view_dom(
        app,
        "clipboard",
        r#"
const button=document.querySelector('[data-record-id=action-feedback] [data-action=copy] button');button.click();return true;
"#,
    )?;
    samples.push(wait_records(app, r#"
const panel=document.querySelector('.clipboard-panel'),button=panel.querySelector('[data-record-id=action-feedback] [data-action=copy] button');
return {ready:!!button?.querySelector('.copy-success')&&!panel.closest('[hidden]'),copiedWhileOpen:true};
"#)?);
    samples.push(wait_records(app, r#"
const rows=[...document.querySelectorAll('.clipboard-panel .history-row')].filter(row=>row.querySelector('.history-row-select')?.getAttribute('aria-label')==='Clipboard: Synthetic copy feedback');
return {ready:rows.length===2&&rows.some(row=>row.dataset.recordId==='action-feedback'),distinctRows:rows.map(row=>row.dataset.recordId)};
"#)?);
    verify_record_beam_motion(app, samples)?;
    for (label, active) in [("Favorite", true), ("Remove favorite", false)] {
        evaluate_view_dom(app, "clipboard", &format!("window.__star=document.querySelector('[data-record-id=action-feedback] [data-action=favorite] .history-symbol');document.querySelector('[data-record-id=action-feedback] [aria-label=\"{label}\"]').click();return true;"))?;
        let favorite = wait_records(
            app,
            &format!(
                r#"const row=document.querySelector('[data-record-id=action-feedback]'),button=row.querySelector('[data-action=favorite] button'),icon=button.querySelector('.history-symbol');return {{ready:button.getAttribute('aria-pressed')==='{active}',same:icon===window.__star,symbol:icon.dataset.symbol,beams:document.querySelectorAll('[role=progressbar]').length}};"#
            ),
        )?;
        if favorite["same"] != true || favorite["symbol"] != "star" || favorite["beams"] != 0 {
            return Err(format!("favorite feedback: {favorite}"));
        }
        samples.push(favorite);
        if active {
            evaluate_view_dom(app,"clipboard","window.__footerStar=document.querySelector('.clipboard-panel .history-favorites .history-symbol');document.querySelector('.clipboard-panel .history-favorites').click();return true;")?;
            let footer = wait_records(
                app,
                r#"const b=document.querySelector('.clipboard-panel .history-favorites'),i=b.querySelector('.history-symbol'),color=getComputedStyle(i).color,warning=matchMedia('(prefers-color-scheme: dark)').matches?'rgb(216, 150, 20)':'rgb(250, 173, 20)';return {ready:b.getAttribute('aria-pressed')==='true'&&color===warning,same:i===window.__footerStar,symbol:i.dataset.symbol,color};"#,
            )?;
            if footer["same"] != true || footer["symbol"] != "star" {
                return Err(format!("footer favorite: {footer}"));
            }
            samples.push(footer);
            wait_records(
                app,
                "return {ready:!!document.querySelector('[data-record-id=action-feedback]')};",
            )?;
            evaluate_view_dom(app,"clipboard","document.querySelector('[data-record-id=action-feedback]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:40,clientY:100}));return true;")?;
            let menu = wait_records(
                app,
                r#"const b=[...document.querySelectorAll('[role=menuitem]')].find(b=>b.querySelector('.record-menu-label')?.textContent==='Unfavorite'),i=b?.querySelector('.history-symbol'),color=i&&getComputedStyle(i).color,warning=matchMedia('(prefers-color-scheme: dark)').matches?'rgb(216, 150, 20)':'rgb(250, 173, 20)';return {ready:!!i&&color===warning,symbol:i?.dataset.symbol,color};"#,
            )?;
            if menu["symbol"] != "star" {
                return Err(format!("menu favorite: {menu}"));
            }
            samples.push(menu);
            evaluate_view_dom(app,"clipboard","document.querySelector('[role=menu]').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));document.querySelector('.clipboard-panel .history-favorites').click();return true;")?;
            wait_records(app,"return {ready:!document.querySelector('[role=menu]')&&!!document.querySelector('[data-record-id=action-feedback]')&&document.querySelector('.clipboard-panel .history-favorites').getAttribute('aria-pressed')==='false'};")?;
        }
    }
    for active in [true, false] {
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
const row=document.querySelector('[data-record-id=action-feedback] .history-row-select');
row.focus();
row.dispatchEvent(new KeyboardEvent('keydown',{code:'KeyF',key:'ƒ',altKey:true,bubbles:true,cancelable:true}));
return true;
"#,
        )?;
        samples.push(wait_records(app, &format!(r#"
const button=document.querySelector('[data-record-id=action-feedback] [data-action=favorite] button');
return {{ready:button?.getAttribute('aria-pressed')==='{active}',keyboardFavorite:{active}}};
"#))?);
    }
    verify_direct_delete(app, "clipboard", samples)?;
    Ok(())
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
                        || sample[state]["timeDuration"] != "0.15s"
                    {
                        return Err(format!("clipboard action transition: {sample}"));
                    }
                }
                samples.push(sample);
                verify_clipboard_action_feedback(app, &mut samples)?;
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
if(!card){const note=[...document.querySelectorAll('.clipboard-context-menu button')].find(button=>button.querySelector('.record-menu-label')?.textContent==='Note');if(note)note.click();else row.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:20,clientY:60}));return {ready:false};}
const input=card.querySelector('input'),content=card.querySelector('textarea');if(!content||input.disabled)return {ready:false};
const labels=[...card.querySelectorAll('.clipboard-editor-label')],style=getComputedStyle(content);
return {ready:true,headings:card.querySelectorAll('h1,h2,h3').length,hints:card.querySelectorAll('.clipboard-editor-hint').length,labels:labels.map(label=>label.textContent),keys:[...card.querySelectorAll('.clipboard-editor-actions kbd')].map(key=>key.textContent),weights:labels.map(label=>getComputedStyle(label).fontWeight),fontStyle:style.fontStyle,contentWeight:style.fontWeight,labelStyle:getComputedStyle(labels[1]).fontStyle,value:content.value,disabled:content.disabled,readonly:content.readOnly,color:style.color,editableColor:getComputedStyle(input).color,resize:style.resize,alignment:labels.map((label,index)=>{const a=label.getBoundingClientRect(),b=[input,content][index].getBoundingClientRect();return {left:Math.abs(a.left-b.left),right:Math.abs(a.right-b.right)};})};
"#,
            )?;
            if sample["ready"] == true {
                if sample["headings"] != 0
                    || sample["hints"] != 0
                    || sample["labels"] != serde_json::json!(["Title", "Content"])
                    || sample["keys"] != serde_json::json!(["Esc", "↵"])
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
                for (title, key, expected) in [
                    ("Saved with Enter", "Enter", "Saved with Enter"),
                    ("Cancelled with Esc", "Escape", "Saved with Enter"),
                ] {
                    evaluate_view_dom(
                        app,
                        "clipboard",
                        r#"document.querySelector('[data-record-id="editor-layout"] [data-action="note"] button').click();return true;"#,
                    )?;
                    wait_records(app, "const input=document.querySelector('.clipboard-editor input');return {ready:!!input&&!input.disabled};")?;
                    let title = serde_json::to_string(title).map_err(|_| "fixture")?;
                    evaluate_view_dom(
                        app,
                        "clipboard",
                        &format!(
                            r#"
const input=document.querySelector('.clipboard-editor input');
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,{title});
input.dispatchEvent(new Event('input',{{bubbles:true}}));
input.dispatchEvent(new KeyboardEvent('keydown',{{key:'{key}',bubbles:true,cancelable:true}}));return true;
"#
                        ),
                    )?;
                    samples.push(wait_records(app, &format!(r#"
const row=document.querySelector('[data-record-id="editor-layout"]');
return {{ready:!document.querySelector('.clipboard-editor')&&row?.textContent.includes('{expected}')&&!row?.textContent.includes('Cancelled with Esc'),key:'{key}'}};
"#))?);
                }
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
        Ok(())
    })?;
    wait_motion(app)?;
    let selector = if mode == "clipboard" {
        ".clipboard-panel"
    } else {
        "#translation-history"
    };
    wait_records(app, &format!("const p=document.querySelector('{selector}');return {{ready:!!p&&p.getClientRects().length>0&&!p.closest('[hidden]')&&!!p.querySelector('.history-row:not([hidden])')&&!!p.querySelector('.history-search input')}};"))?;
    // Focus the mounted child after WK startup, which can otherwise replace its native responder.
    on_main(app, |app| {
        let window = app.get_window("translation").ok_or("translation window")?;
        window.set_focus().map_err(|_| "window_focus")?;
        activate_application();
        app.get_webview("clipboard")
            .ok_or("records child")?
            .set_focus()
            .map_err(|_| "records focus")?;
        Ok(())
    })?;
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
const probe=row.cloneNode(true);probe.classList.remove('selected','context-target');probe.removeAttribute('data-record-target');p.append(probe);const ordinaryBackground=getComputedStyle(probe).backgroundColor;probe.classList.add('selected');const detailBackground=getComputedStyle(probe).backgroundColor;probe.setAttribute('data-record-target','true');const targetBackground=getComputedStyle(probe).backgroundColor;probe.remove();
const label=row.querySelector('.history-languages'),labelStyle=getComputedStyle(label);
return {ready:true,kind:p.id==='translation-history'?'translation':'clipboard',panel:nodeId(p),width:p.getBoundingClientRect().width,
rows:[...p.querySelectorAll('.history-row:not([hidden])')].map(r=>({id:r.dataset.recordId,node:nodeId(r),selected:r.classList.contains('selected'),pinned:r.classList.contains('pinned')})),
search:{...box(search),border:getComputedStyle(search).borderTopWidth,radius:getComputedStyle(search).borderRadius,shadow:getComputedStyle(search).boxShadow,font:getComputedStyle(input).fontSize,caret:getComputedStyle(input).caretColor},query:input.value,
toolbar:[...p.querySelectorAll('.history-toolbar > button')].map(b=>({...box(b),...icon(b),pressed:b.getAttribute('aria-pressed'),expanded:b.getAttribute('aria-expanded'),disabled:b.disabled})),
actions:[...row.querySelectorAll('.history-row-actions button:not([aria-hidden="true"])')].map(icon),actionDuration:getComputedStyle(row.querySelector('.history-row-actions')).transitionDuration,
labelColor:labelStyle.color,labelFill:labelStyle.webkitTextFillColor,targetBackground,detailBackground,ordinaryBackground,
timeDuration:getComputedStyle(row.querySelector('time')).transitionDuration,previewFont:getComputedStyle(row.querySelector('.history-preview')).fontSize,
origin:{...box(row.querySelector('.history-origin')),fit:getComputedStyle(row.querySelector('.history-origin')).objectFit,radius:getComputedStyle(row.querySelector('.history-origin')).borderRadius},
footer:{...box(footer),star:icon(footer.querySelector('.history-favorites')),keyboard:icon(footer.querySelector('.history-shortcuts')),count:footer.querySelector('.history-record-count').textContent},
scrollbar:scroller?{...box(scroller),visible:scroller.classList.contains('visible'),transition:getComputedStyle(scroller.firstElementChild).transitionDuration}:null,
mask:getComputedStyle(p.querySelector('.history-list')).webkitMaskImage,error:document.querySelector('.global-error')?.textContent};
"#;

fn verify_direct_delete(
    app: &AppHandle,
    mode: &str,
    samples: &mut Vec<serde_json::Value>,
) -> Result<(), String> {
    let selector = if mode == "clipboard" {
        ".clipboard-panel"
    } else {
        "#translation-history"
    };
    let before = evaluate_view_dom(
        app,
        "clipboard",
        &format!(
            r#"const p=document.querySelector('{selector}'),b=p.querySelector('.history-items .history-delete:not(:disabled)');if(!b)throw Error('missing delete');const red=document.createElement('span');red.style.color='var(--history-red)';b.parentElement.append(red);const expected=getComputedStyle(red).color;red.remove();const color=getComputedStyle(b).color;window.__deleteRecord=b.closest('[data-record-id]').dataset.recordId;b.click();return {{color,expected}};"#
        ),
    )?;
    if before["color"] != before["expected"] {
        return Err(format!(
            "{mode} delete is not red before clicking: {before}"
        ));
    }
    samples.push(before);
    samples.push(wait_records(app, r#"return {ready:!document.querySelector(`[data-record-id="${window.__deleteRecord}"]`),deleted:window.__deleteRecord};"#)?);
    Ok(())
}

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
            || sample["timeDuration"] != "0.15s"
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
    let mut case = run_record_action_timing(app, "translation", "translation action timing");
    if !case.passed {
        return case;
    }
    let result = (|| {
        on_main(app, |app| {
            app.state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .translation_visible = false;
            apply_translation(app)
        })?;
        wait_motion(app)?;
        wait_records(app, "return {ready:document.querySelector('#translation-history .records-details').getAttribute('aria-expanded')==='false'};")?;
        let controls = {
            let state = app.state::<AppState>();
            let mut options = state.clipboard_options.lock().unwrap().clone();
            options.record_shortcuts.set(
                nami::shortcuts::ClipboardRecordAction::Preview,
                Some(nami::shortcuts::KeyBinding::new("Space", &[])),
                None,
                &[],
            )?;
            options.controls()
        };
        app.get_webview("clipboard")
            .ok_or("records child")?
            .emit("clipboard_controls_changed", controls)
            .map_err(|_| "event_emit")?;
        let setup = r#"
const panel=document.querySelector('#translation-history'),row=panel.querySelector('.history-items .history-row-select');
row.focus();row.blur();
window.__detailKeys=[];
window.addEventListener('keydown',e=>{if(e.code==='Space')window.__detailKeys.push({trusted:e.isTrusted,handled:e.defaultPrevented,body:e.target===document.body});});
return document.activeElement===document.body;
"#;
        if evaluate_view_dom(app, "clipboard", setup)? != true {
            return Err("could not leave focus on the records body".into());
        }
        for (trigger, open, close) in [
            ("space", "", ""),
            ("toolbar", "document.querySelector('#translation-history .records-details').click();return true;", "document.querySelector('#translation-history .records-details').click();return true;"),
            ("row", "window.__clickedRow=document.querySelector('#translation-history .history-items .history-row-select');window.__clickedRow.click();return true;", "window.__clickedRow.click();window.__clickedRow.dispatchEvent(new MouseEvent('dblclick',{bubbles:true,detail:2}));return true;"),
        ] {
            if trigger == "space" {
                post_records_key(app, 49, 0, " ", " ", false)?;
            } else {
                evaluate_view_dom(app, "clipboard", open)?;
            }
            wait_records(app, "return {ready:document.querySelector('#translation-history .records-details').getAttribute('aria-expanded')==='true'};")?;
            wait_motion(app)?;
            let pre_motion_events = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let observed = pre_motion_events.clone();
            let owned_app = app.clone();
            let close_listener = app.listen_any("history_changed", move |_| {
                let state = owned_app.state::<AppState>();
                let state = state.windows.lock().unwrap();
                if !state.translation_visible
                    && state.translation_extent >= state.translation_width - 0.5
                {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
            });
            let clicked_at = std::time::Instant::now();
            if trigger == "space" {
                post_records_key(app, 49, 0, " ", " ", false)?;
            } else {
                evaluate_view_dom(app, "clipboard", close)?;
            }
            let mut first_move_ms = None;
            for _ in 0..100 {
                let moved = on_main(app, |app| {
                    let state = app.state::<AppState>();
                    let state = state.windows.lock().unwrap();
                    Ok(state.translation_extent < state.translation_width - 0.5)
                })?;
                if moved {
                    first_move_ms = Some(clicked_at.elapsed().as_secs_f64() * 1000.0);
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            app.unlisten(close_listener);
            let pre_motion_events = pre_motion_events.load(Ordering::SeqCst);
            case.samples.push(serde_json::json!({"trigger":trigger,"closeFirstMoveMs":first_move_ms,"historySnapshotsBeforeMotion":pre_motion_events}));
            if pre_motion_events != 0 || first_move_ms.is_none_or(|elapsed| elapsed > 75.0) {
                return Err(format!("{trigger} close waited before moving: historySnapshots={pre_motion_events}, firstMoveMs={first_move_ms:?}"));
            }
            let closed = wait_records(app, "return {ready:document.querySelector('#translation-history .records-details').getAttribute('aria-expanded')==='false',beam:!!document.querySelector('[role=progressbar]')};")?;
            if closed["beam"] != false {
                return Err(format!("{trigger} close showed activity"));
            }
            wait_motion(app)?;
        }
        case.samples.push(wait_records(app, "const keys=window.__detailKeys;return {ready:keys.length===2&&keys.every(k=>k.trusted&&k.handled&&k.body),keys};")?);
        app.get_webview("clipboard")
            .ok_or("records child")?
            .emit(
                "clipboard_controls_changed",
                app.state::<AppState>()
                    .clipboard_options
                    .lock()
                    .unwrap()
                    .controls(),
            )
            .map_err(|_| "event_emit")?;
        verify_direct_delete(app, "translation", &mut case.samples)?;
        on_main(app, |app| {
            app.state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .translation_visible = true;
            apply_translation(app)
        })?;
        wait_motion(app)?;
        let original_preferences = app
            .state::<AppState>()
            .translation_preferences
            .lock()
            .unwrap()
            .clone();
        let mixed = crate::model::HistoryRecord {
            title: String::new(),
            id: "translation-auto-metadata".into(),
            text: "Synthetic mixed language input".into(),
            source_language: "mul".into(),
            translation: Some(crate::model::TranslationMetadata {
                requested_source: "auto".into(),
                engine: "edge".into(),
                target_engines: Vec::new(),
                strategy_version: crate::translation::STRATEGY_VERSION,
                detections: vec![
                    crate::model::LanguageDetection {
                        raw_language: "zh-Hans".into(),
                        language: "zh-CN".into(),
                        score: Some(1.0),
                    },
                    crate::model::LanguageDetection {
                        raw_language: "en".into(),
                        language: "en".into(),
                        score: Some(1.0),
                    },
                ],
            }),
            target_languages: vec!["en".into()],
            translations: vec!["Synthetic mixed language output".into()],
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
            .insert(&mixed, "edge", || true)?;
        crate::history_runtime::history_add(app, mixed.clone())?;
        let metadata_result = (|| {
            for engine in ["edge", "google"] {
                evaluate_dom(
                    app,
                    &format!(
                        r#"window.__metadataSetup={{done:false}};(async()=>{{await window.__TAURI_INTERNALS__.invoke('history_sample_select',{{sampleId:null,retranslate:false,clearDetail:true}});await window.__TAURI_INTERNALS__.invoke('translation_preferences_set',{{engine:{engine:?},sourceLanguage:'auto',primaryLanguage:'en',secondaryLanguage:'none'}});await window.__TAURI_INTERNALS__.invoke('history_sample_select',{{sampleId:'translation-auto-metadata',retranslate:false}});}})().then(()=>window.__metadataSetup={{done:true}},error=>window.__metadataSetup={{done:true,error:String(error)}});return true;"#
                    ),
                )?;
                let restored = wait_dom(
                    app,
                    r#"const source=document.querySelector('.source-bar > .language-picker'),bar=document.querySelector('.source-bar');return {ready:window.__metadataSetup?.done&&!window.__metadataSetup.error&&document.querySelector('.source-input')?.value==='Synthetic mixed language input'&&source?.textContent==='Multiple languages'&&bar?.dataset.detectedLanguage==='mul',source:source?.textContent,detected:bar?.dataset.detectedLanguage,description:source?.getAttribute('aria-description'),result:document.querySelector('.result-column')?.textContent,error:window.__metadataSetup?.error,status:document.querySelector('.translation-status')?.textContent};"#,
                )?;
                if restored["description"] != "Detected: Multiple languages"
                    || restored["result"] != "Synthetic mixed language output"
                    || restored["status"].is_string()
                {
                    return Err(format!(
                        "{engine} auto metadata history restore: {restored}"
                    ));
                }
                let mut native = serde_json::Value::Null;
                for _ in 0..40 {
                    native = on_main(app, native_chrome_sample)?;
                    if native["buttons"][0]["title"] == "Multiple languages" {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                if native["buttons"][0]["title"] != "Multiple languages" {
                    return Err(format!("{engine} native chrome lost auto mode: {native}"));
                }
                case.samples.push(serde_json::json!({"engine":engine,"restored":restored,"nativeSource":native["buttons"][0]}));
                app.get_webview("translation")
                    .ok_or("translation view")?
                    .emit("translation_chrome_action", "source")
                    .map_err(|_| "source menu event")?;
                let source_catalog = wait_dom(
                    app,
                    r#"const popover=document.querySelector('.language-popover'),options=[...popover?.querySelectorAll('[role=option]')??[]];return {ready:options.length>100,count:options.length,esperanto:options.some(option=>option.textContent==='Esperanto'),swahili:options.some(option=>option.textContent==='Swahili'),selected:options.filter(option=>option.getAttribute('aria-selected')==='true').map(option=>option.textContent)};"#,
                )?;
                let expected_source = crate::translation::languages(engine)
                    .iter()
                    .filter(|language| language.source)
                    .count()
                    + 1;
                if source_catalog["count"].as_u64() != Some(expected_source as u64)
                    || source_catalog["esperanto"] != (engine == "google")
                    || source_catalog["swahili"] != true
                    || source_catalog["selected"] != serde_json::json!(["Auto-Detect"])
                {
                    return Err(format!(
                        "{engine} source language catalog differs: {source_catalog}"
                    ));
                }
                case.samples
                    .push(serde_json::json!({"engine":engine,"sourceCatalog":source_catalog}));
                evaluate_dom(app, "document.querySelector('.language-search input').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));return true;")?;
                wait_dom(
                    app,
                    "return {ready:!document.querySelector('.language-popover')};",
                )?;
                evaluate_dom(app, "document.querySelector('.translation-footer .language-slot .language-picker').click();return true;")?;
                let target_catalog = wait_dom(
                    app,
                    r#"const options=[...document.querySelectorAll('.language-popover [role=option]')];return {ready:options.length>100,count:options.length,esperanto:options.some(option=>option.textContent==='Esperanto'),selected:options.filter(option=>option.getAttribute('aria-selected')==='true').map(option=>option.textContent)};"#,
                )?;
                let expected_target = crate::translation::languages(engine)
                    .iter()
                    .filter(|language| language.target)
                    .count();
                if target_catalog["count"].as_u64() != Some(expected_target as u64)
                    || target_catalog["esperanto"] != (engine == "google")
                    || target_catalog["selected"] != serde_json::json!(["English"])
                {
                    return Err(format!(
                        "{engine} target language catalog differs: {target_catalog}"
                    ));
                }
                case.samples
                    .push(serde_json::json!({"engine":engine,"targetCatalog":target_catalog}));
                evaluate_dom(app, "document.querySelector('.language-search input').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));return true;")?;
                wait_dom(
                    app,
                    "return {ready:!document.querySelector('.language-popover')};",
                )?;
            }
            Ok::<_, String>(())
        })();
        let mut long_name = mixed;
        long_name.id = "translation-long-detected-name".into();
        long_name.source_language = "nhe".into();
        long_name.translation.as_mut().unwrap().engine = "google".into();
        app.state::<AppState>()
            .database
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("isolated database")?
            .insert(&long_name, "google", || true)?;
        crate::history_runtime::history_add(app, long_name)?;
        evaluate_dom(app, "window.__TAURI_INTERNALS__.invoke('history_sample_select',{sampleId:'translation-long-detected-name',retranslate:false});return true;")?;
        let full_name = wait_dom(app, "const b=document.querySelector('.source-bar > .language-picker'),s=b?.querySelector('span');return {ready:b?.textContent==='Nahuatl (Eastern Huasteca)',label:b?.textContent,width:b?.getBoundingClientRect().width,full:!!s&&s.scrollWidth<=s.clientWidth};")?;
        let mut native_name = serde_json::Value::Null;
        for _ in 0..40 {
            native_name = on_main(app, native_chrome_sample)?;
            if native_name["buttons"][0]["title"] == "Nahuatl (Eastern Huasteca)" {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if full_name["full"] != true
            || native_name["buttons"][0]["title"] != "Nahuatl (Eastern Huasteca)"
            || native_name["buttons"][0]["width"].as_f64().unwrap_or(0.0) + 1.0
                < full_name["width"].as_f64().unwrap_or(1000.0)
        {
            return Err(format!(
                "detected language name truncated: {full_name} / {native_name}"
            ));
        }
        case.samples.push(serde_json::json!({"fullDetectedName":full_name,"nativeSource":native_name["buttons"][0]}));
        let restored_preferences = serde_json::json!({"engine":original_preferences.engine,"sourceLanguage":original_preferences.source_language,"primaryLanguage":original_preferences.primary_language,"secondaryLanguage":original_preferences.secondary_language});
        evaluate_dom(
            app,
            &format!(
                r#"window.__metadataCleanup={{done:false}};(async()=>{{await window.__TAURI_INTERNALS__.invoke('history_sample_select',{{sampleId:null,retranslate:false,clearDetail:true}});await window.__TAURI_INTERNALS__.invoke('translation_preferences_set',{restored_preferences});await window.__TAURI_INTERNALS__.invoke('history_record_update',{{recordId:'translation-auto-metadata',action:'delete'}});await window.__TAURI_INTERNALS__.invoke('history_record_update',{{recordId:'translation-long-detected-name',action:'delete'}});await window.__TAURI_INTERNALS__.invoke('window_hide',{{region:'translation',keepHistory:true}});}})().then(()=>window.__metadataCleanup={{done:true}},error=>window.__metadataCleanup={{done:true,error:String(error)}});return true;"#
            ),
        )?;
        let cleanup = wait_dom(
            app,
            "return {ready:window.__metadataCleanup?.done,...window.__metadataCleanup};",
        )?;
        if cleanup["error"].is_string() {
            return Err(format!("metadata scenario cleanup failed: {cleanup}"));
        }
        metadata_result?;
        Ok::<_, String>(())
    })();
    case.passed = result.is_ok();
    case.error = result.err();
    case
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
    record_case(
        &mut cases,
        "detail open and close use matching native timing",
        |samples| {
            on_main(app, |app| {
                reset(app, 360.0, 700.0)?;
                let state = app.state::<AppState>();
                state.windows.lock().unwrap().translation_visible = false;
                layout_translation(app, false)?;
                let window = app.get_window("translation").ok_or("window")?;
                window.show().map_err(|_| "window_show")?;
                Ok(())
            })?;
            for visible in [true, false, true, false] {
                on_main(app, move |app| {
                    app.state::<AppState>()
                        .windows
                        .lock()
                        .unwrap()
                        .translation_visible = visible;
                    apply_translation(app)
                })?;
                std::thread::sleep(Duration::from_millis(320));
                let frame = on_main(app, move |app| {
                    let state = app.state::<AppState>();
                    let state = state.windows.lock().unwrap();
                    if state.motion_anchor.is_some() {
                        return Err("detail motion exceeded 280ms plus two frame allowance".into());
                    }
                    equal(
                        state.translation_extent,
                        if visible { 700.0 } else { 0.0 },
                        "detail final extent",
                    )?;
                    drop(state);
                    geometry(app)
                })?;
                equal(
                    frame.window_width,
                    if visible { 1060.0 } else { 360.0 },
                    "detail final native width",
                )?;
                samples.push(frame);
            }
            Ok(())
        },
    );
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
            evaluate_view_dom(
                app,
                "clipboard",
                r#"window.__pinOrder={done:false};(async()=>{try{const p=document.querySelector('.record-view:not([hidden]) .history-panel'),command=p.id==='translation-history'?'history_record_update':'clipboard_record_update',rows=[...p.querySelectorAll('.history-items .history-row:not([hidden])')],ids=rows.slice(-2).map(r=>r.dataset.recordId),dates=rows.slice(-2).map(r=>r.querySelector('time').dateTime);for(const recordId of ids)await window.__TAURI_INTERNALS__.invoke(command,{recordId,action:'pin'});window.__pinOrder={done:true,ids,dates,command};}catch(error){window.__pinOrder={done:true,error:String(error)};}})();return true;"#,
            )?;
            let order = wait_records(
                app,
                r#"const test=window.__pinOrder,p=document.querySelector('.record-view:not([hidden]) .history-panel'),rows=[...p.querySelectorAll('.history-pinned .history-row:not([hidden])')],ids=rows.map(r=>r.dataset.recordId);return {ready:test?.done&&ids[0]===test.ids?.[1]&&ids[1]===test.ids?.[0],ids:ids.slice(0,2),dates:test.ids?.map(id=>p.querySelector(`[data-record-id="${id}"] time`).dateTime),originalDates:test.dates,error:test.error};"#,
            )?;
            if order["dates"] != order["originalDates"] {
                return Err(format!("pin changed display dates: {order}"));
            }
            samples.push(order);
            evaluate_view_dom(
                app,
                "clipboard",
                r#"window.__pinOrderRestored=false;(async()=>{for(const recordId of window.__pinOrder.ids)await window.__TAURI_INTERNALS__.invoke(window.__pinOrder.command,{recordId,action:'pin'});window.__pinOrderRestored=true;})();return true;"#,
            )?;
            wait_records(app,"return {ready:window.__pinOrderRestored&&document.querySelectorAll('.record-view:not([hidden]) .history-pinned .history-row:not([hidden])').length===2};")?;
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
            title: String::new(),
            id: "unified-translation".into(),
            text: "Synthetic shared record input".into(),
            source_language: "en".into(),
            translation: None,
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
        evaluate_view_dom(
            app,
            "clipboard",
            r#"window.__invalidClearDetail={done:false};window.__TAURI_INTERNALS__.invoke('history_sample_select',{sampleId:'unified-translation',retranslate:false,clearDetail:true}).then(()=>window.__invalidClearDetail={done:true,rejected:false},error=>window.__invalidClearDetail={done:true,rejected:error==='invalid_argument',error:String(error)});return true;"#,
        )?;
        let invalid_clear = wait_records(
            app,
            "return {ready:window.__invalidClearDetail?.done,...window.__invalidClearDetail};",
        )?;
        if invalid_clear["rejected"] != true {
            return Err(format!(
                "clearDetail accepted a nonempty sampleId: {invalid_clear}"
            ));
        }
        samples.push(invalid_clear);
        for key in [
            "search",
            "actionDuration",
            "timeDuration",
            "previewFont",
            "labelColor",
            "labelFill",
            "targetBackground",
            "detailBackground",
            "mask",
        ] {
            if clipboard[key] != translation[key] {
                return Err(format!(
                    "record {key} differs: clipboard={}, translation={}",
                    clipboard[key], translation[key]
                ));
            }
        }
        if clipboard["targetBackground"] == clipboard["ordinaryBackground"]
            || clipboard["detailBackground"] != clipboard["ordinaryBackground"]
        {
            return Err("only the interaction target should have a row background".into());
        }
        let clipboard_actions = clipboard["actions"]
            .as_array()
            .ok_or("clipboard actions")?
            .iter()
            .filter(|action| {
                !["languages", "ocr"].contains(&action["symbol"].as_str().unwrap_or_default())
            })
            .collect::<Vec<_>>();
        let translation_actions = translation["actions"]
            .as_array()
            .ok_or("translation actions")?;
        if clipboard_actions.len() != translation_actions.len()
            || !["doc.on.doc", "note"].into_iter().all(|symbol| {
                translation_actions
                    .iter()
                    .any(|action| action["symbol"] == symbol)
            })
        {
            return Err(format!(
                "shared record actions differ: {clipboard} / {translation}"
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
                r#"const menu=document.querySelector('.record-context-menu');if(!menu)return {ready:false};
const style=getComputedStyle(menu),label=b=>b.querySelector('.record-menu-label').textContent;
const nodes=[...menu.querySelectorAll('[role=menuitem],[role=separator]')],separator=menu.querySelector('[role=separator]');
return {ready:true,contextTarget:!!document.querySelector('.record-view:not([hidden]) .history-row.context-target'),labels:nodes.map(b=>b.getAttribute('role')==='separator'?'/':label(b)),keys:nodes.filter(b=>b.getAttribute('role')==='menuitem').map(b=>b.querySelector('kbd')?.textContent),width:menu.getBoundingClientRect().width,divider:separator&&[getComputedStyle(separator).height,getComputedStyle(separator).backgroundImage],background:style.backgroundColor,blur:style.webkitBackdropFilter,radius:style.borderRadius,shadow:style.boxShadow,items:[...menu.querySelectorAll('button')].filter(b=>['Favorite','Pin','Delete'].includes(label(b))).map(b=>{const s=getComputedStyle(b),i=b.querySelector('.history-symbol');return {label:label(b),color:s.color,height:s.height,opacity:s.opacity,symbol:i.dataset.symbol,mask:getComputedStyle(i).webkitMaskImage,width:getComputedStyle(i).width};})};"#,
            )?;
            if menu["contextTarget"] != true {
                return Err(format!(
                    "{mode}: right-click target is not highlighted: {menu}"
                ));
            }
            if mode == "clipboard"
                && menu["labels"]
                    != serde_json::json!([
                        "Paste",
                        "Paste as Plain Text",
                        "Copy",
                        "/",
                        "Favorite",
                        "Pin",
                        "Note",
                        "/",
                        "Delete"
                    ])
            {
                return Err(format!("clipboard text menu order: {menu}"));
            }
            if mode == "translation"
                && (menu["labels"]
                    != serde_json::json!([
                        "Copy source text",
                        "Copy Primary Text",
                        "/",
                        "Translate",
                        "/",
                        "Favorite",
                        "Pin",
                        "Note",
                        "/",
                        "Delete"
                    ])
                    || menu["keys"]
                        != serde_json::json!(["⇧⌘C", "⌘1", "⌘↵", "⌥F", "⌥P", "⇧⌘N", "⌘⌫"]))
            {
                return Err(format!("single-result translation menu: {menu}"));
            }
            samples.push(
                serde_json::json!({"mode":mode,"menuLabels":menu["labels"],"contextTarget":true}),
            );
            let mut common = menu;
            common.as_object_mut().unwrap().remove("labels");
            common.as_object_mut().unwrap().remove("keys");
            menus.push(common);
            evaluate_view_dom(app,"clipboard","document.querySelector('.record-context-menu').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));return true;")?;
            wait_records(app, "return {ready:!document.querySelector('.record-context-menu')&&!document.querySelector('.record-view:not([hidden]) .history-row.context-target')};")?;
        }
        if menus[0] != menus[1] {
            return Err(format!("record menus differ: {menus:?}"));
        }
        samples.push(serde_json::json!({"sharedMenus":menus}));
        let before_copy = on_main(app, |_| Ok(crate::platform::clipboard_version()))?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
const row=document.querySelector('#translation-history [data-record-id=unified-translation]');
row.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:100,clientY:120}));return true;
"#,
        )?;
        wait_records(app, "return {ready:!!document.querySelector('#translation-history .context-target[data-record-id=unified-translation]')&&!!document.querySelector('[role=menu]')};")?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
[...document.querySelectorAll('[role=menu] .record-menu-label')].find(item=>item.textContent==='Copy Primary Text').closest('button').click();return true;
"#,
        )?;
        let mut copied_primary = false;
        for _ in 0..80 {
            copied_primary = on_main(app, move |_| {
                Ok(crate::platform::clipboard_version() != before_copy
                    && crate::platform::read_clipboard(|_| false)?.is_some_and(|capture| matches!(capture.payload, crate::platform::ClipboardPayload::Text(text) if text == "Synthetic shared output")))
            })?;
            if copied_primary {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if !copied_primary {
            return Err("translation menu did not copy the saved primary text".into());
        }
        samples.push(serde_json::json!({"contextPrimaryCopied":true}));
        for mode in ["clipboard", "translation"] {
            records_mode(app, mode)?;
            for width in [280.0, 320.0, 454.0, 380.0] {
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
                let footer = wait_records(
                    app,
                    r#"
const p=document.querySelector('.record-view:not([hidden]) .history-panel'),bounds=p.getBoundingClientRect(),footer=p.querySelector('.history-footer');
const filterButtons=[...footer.querySelectorAll('.history-filters button')].map(b=>b.getBoundingClientRect()),filters={left:Math.min(...filterButtons.map(b=>b.left)),right:Math.max(...filterButtons.map(b=>b.right))},count=footer.querySelector('.history-record-count').getBoundingClientRect(),keyboard=footer.querySelector('.history-shortcuts').getBoundingClientRect();
const controls=[...footer.querySelectorAll('button')].map(button=>({label:button.getAttribute('aria-label'),rect:button.getBoundingClientRect()}));
const overlap=(a,b)=>Math.min(a.right,b.right)-Math.max(a.left,b.left)>.5&&Math.min(a.bottom,b.bottom)-Math.max(a.top,b.top)>.5;
return {ready:true,width:bounds.width,centerOffset:(filters.left+filters.right)/2-(bounds.left+bounds.width/2),overlaps:controls.flatMap((a,i)=>controls.slice(i+1).filter(b=>overlap(a.rect,b.rect)).map(b=>[a.label,b.label])),countOverlap:controls.some(a=>overlap(a.rect,count)),within:controls.every(a=>a.rect.left>=bounds.left&&a.rect.right<=bounds.right),keyboardAtEdge:keyboard.right<=bounds.right};
"#,
                )?;
                if footer["centerOffset"].as_f64().unwrap_or(99.).abs() > 0.5
                    || footer["overlaps"]
                        .as_array()
                        .is_none_or(|values| !values.is_empty())
                    || footer["countOverlap"] != false
                    || footer["within"] != true
                {
                    return Err(format!(
                        "{mode}: footer loses centering or overlaps at {width}px: {footer}"
                    ));
                }
                samples.push(footer);
            }
        }
        for (name, args) in [
            ("records_view_set", serde_json::json!({"view":"clipboard"})),
            (
                "window_hide",
                serde_json::json!({"region":"translation","keepHistory":true}),
            ),
        ] {
            if name == "window_hide" {
                on_main(app, |app| {
                    app.state::<AppState>()
                        .windows
                        .lock()
                        .unwrap()
                        .translation_visible = true;
                    apply_translation(app)
                })?;
                wait_motion(app)?;
            }
            let token_before = app.state::<AppState>().operation.load(Ordering::SeqCst);
            evaluate_view_dom(app, "clipboard", &format!("window.__explicitNavigation={{done:false}};window.__TAURI_INTERNALS__.invoke({name:?},{args}).then(()=>window.__explicitNavigation={{done:true}},error=>window.__explicitNavigation={{done:true,error:String(error)}});return true;"))?;
            let navigation = wait_records(
                app,
                "return {ready:window.__explicitNavigation?.done,...window.__explicitNavigation};",
            )?;
            let token_after = app.state::<AppState>().operation.load(Ordering::SeqCst);
            if navigation["error"].is_string() || token_after <= token_before {
                return Err(format!("explicit {name} retained stale paste token {token_before}->{token_after}: {navigation}"));
            }
            samples.push(serde_json::json!({"navigation":name,"tokenBefore":token_before,"tokenAfter":token_after}));
        }
        records_mode(app, "clipboard")?;
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
            r#"
const input=document.querySelector('#translation-history .history-search input');
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'synthetic-no-matching-record-71f39');
input.dispatchEvent(new Event('input',{bubbles:true}));return true;
"#,
        )?;
        let no_matches = wait_records(app, "const p=document.querySelector('#translation-history');return {ready:p.querySelectorAll('.history-row:not([hidden])').length===0&&!!p.querySelector('[role=status]'),empty:p.querySelector('[role=status]')?.textContent};")?;
        let cleared = wait_dom(app, "const source=document.querySelector('.source-input')?.value,results=[...document.querySelectorAll('.result-column')].map(e=>e.textContent);return {ready:source===''&&results.every(value=>value===''),source,results};")?;
        samples.extend([no_matches, cleared]);
        evaluate_view_dom(app, "clipboard", "document.querySelector('#translation-history .history-search button').click();return true;")?;
        wait_records(
            app,
            r#"return {ready:!!document.querySelector('#translation-history [data-record-id=unified-translation]')&&document.querySelector('#translation-history .history-search input').value===''};"#,
        )?;
        evaluate_view_dom(app, "clipboard", "document.querySelector('#translation-history [data-record-id=unified-translation] .history-row-select').click();return true;")?;
        samples.push(wait_dom(app, "const source=document.querySelector('.source-input')?.value,result=document.querySelector('.result-column')?.textContent;return {ready:source==='Synthetic shared record input'&&result==='Synthetic shared output',source,result,restoredAfterEmptySearch:true};")?);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('[data-record-id="unified-translation"] .history-row-select').click();return true;"#,
        )?;
        wait_records(app,"return {ready:document.querySelector('#translation-history .records-details').getAttribute('aria-expanded')==='false'};")?;
        wait_motion(app)?;
        wait_records(
            app,
            r#"return {ready:!document.querySelector('#translation-history .history-row.selected')};"#,
        )?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
const row=document.querySelector('#translation-history [data-record-id=unified-translation]');row.querySelector('[data-action=note] button').click();return true;
"#,
        )?;
        wait_records(
            app,
            r#"
const card=document.querySelector('.clipboard-editor');if(!card||card.querySelector('input').disabled)return {ready:false};
const content=card.querySelector('textarea');if(!content.readOnly||content.value!=='Synthetic shared record input')throw Error('translation note content must be read-only');
return {ready:true};
"#,
        )?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
const title=document.querySelector('.clipboard-editor input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(title,'Native title only');title.dispatchEvent(new Event('input',{bubbles:true}));
title.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}));return true;
"#,
        )?;
        samples.push(wait_records(app, r#"
const row=document.querySelector('#translation-history [data-record-id=unified-translation]');return {ready:row?.querySelector('.history-preview')?.textContent==='Native title only'&&!document.querySelector('.clipboard-editor'),translationNoteSaved:true};
"#)?);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('#translation-history input[type=search]').dispatchEvent(new KeyboardEvent('keydown',{key:'Tab',code:'Tab',bubbles:true,cancelable:true}));return true;"#,
        )?;
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
const note=[...document.querySelectorAll('.clipboard-context-menu button')].find(button=>button.querySelector('.record-menu-label')?.textContent==='Note');
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
        for action in ["Cancel window", "Cancel", "Discard"] {
            let presentation = app
                .state::<AppState>()
                .selection_revision
                .fetch_add(1, Ordering::SeqCst)
                + 1;
            on_main(app, move |app| {
                *app.state::<AppState>().target.lock().unwrap() =
                    Some(crate::platform::Target::test_identity());
                if action == "Cancel window" {
                    crate::windows::show(app, "clipboard")
                } else {
                    crate::selection::queue_input(
                        app,
                        presentation,
                        None,
                        "Synthetic pending selected text".into(),
                        None,
                    )
                }
            })?;
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
            if app.state::<AppState>().target.lock().unwrap().is_none() {
                return Err("pending navigation replaced the previous paste target".into());
            }
            if !guarded {
                return Err("navigation switched view before confirmation".into());
            }
            let decision = if action.starts_with("Cancel") {
                "Cancel"
            } else {
                action
            };
            evaluate_view_dom(app,"clipboard",&format!("const dialog=document.querySelector('[role=alertdialog]');const button=[...dialog.querySelectorAll('button')].find(b=>b.textContent==='{}');if(!button)throw Error('confirmation action missing');button.click();return true;",decision))?;
            if decision == "Cancel" {
                let mut retained = false;
                for _ in 0..100 {
                    retained=evaluate_view_dom(app,"clipboard","return !document.querySelector('[role=alertdialog]')&&document.querySelector('.clipboard-editor input')?.value==='Unsaved synthetic note';")?==true;
                    if retained {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                if app.state::<AppState>().target.lock().unwrap().is_none() {
                    return Err("Cancel changed the previous paste target".into());
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
                if app.state::<AppState>().target.lock().unwrap().is_some() {
                    return Err("Discard did not commit the pending paste target".into());
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

fn post_records_key(
    app: &AppHandle,
    code: u16,
    flags: usize,
    characters: &'static str,
    unmodified: &'static str,
    repeat: bool,
) -> Result<(), String> {
    post_window_key(
        app,
        "translation",
        code,
        flags,
        characters,
        unmodified,
        repeat,
    )
}

fn post_window_key(
    app: &AppHandle,
    label: &'static str,
    code: u16,
    flags: usize,
    characters: &'static str,
    unmodified: &'static str,
    repeat: bool,
) -> Result<(), String> {
    on_main(app, move |app| {
        let window = app.get_window(label).ok_or("records host missing")?;
        let native = window
            .ns_window()
            .map_err(|_| "records native host")?
            .cast::<AnyObject>();
        unsafe {
            let application: *mut AnyObject =
                objc2::msg_send![objc2::class!(NSApplication), sharedApplication];
            let number: isize = objc2::msg_send![native, windowNumber];
            let is_key: bool = objc2::msg_send![native, isKeyWindow];
            if !is_key {
                return Err("records window is not key".into());
            }
            let character = objc2_foundation::NSString::from_str(characters);
            let unmodified = objc2_foundation::NSString::from_str(unmodified);
            for kind in [10usize, 11] {
                let event: *mut AnyObject = objc2::msg_send![objc2::class!(NSEvent), keyEventWithType: kind, location: objc2_foundation::NSPoint::new(0.0, 0.0), modifierFlags: flags, timestamp: 0.0f64, windowNumber: number, context: std::ptr::null::<AnyObject>(), characters: &*character, charactersIgnoringModifiers: &*unmodified, isARepeat: repeat && kind == 10, keyCode: code];
                if event.is_null() {
                    return Err("cannot create native key event".into());
                }
                let _: () = objc2::msg_send![application, postEvent: event, atStart: false];
            }
        }
        Ok(())
    })
}

#[repr(C)]
struct TestHotKeyId {
    signature: u32,
    id: u32,
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: TestHotKeyId,
        target: *mut std::ffi::c_void,
        options: u32,
        reference: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn UnregisterEventHotKey(reference: *mut std::ffi::c_void) -> i32;
    fn GetApplicationEventTarget() -> *mut std::ffi::c_void;
    fn CreateEvent(
        allocator: *const std::ffi::c_void,
        class: u32,
        kind: u32,
        time: f64,
        attributes: u32,
        event: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn SetEventParameter(
        event: *mut std::ffi::c_void,
        name: u32,
        kind: u32,
        size: u32,
        data: *const std::ffi::c_void,
    ) -> i32;
    fn SendEventToEventTarget(event: *mut std::ffi::c_void, target: *mut std::ffi::c_void) -> i32;
    fn ReleaseEvent(event: *mut std::ffi::c_void);
}

fn summon_with_clipboard_hotkey(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    on_main(app, |app| {
        let shortcut = crate::binding_shortcut(&nami::shortcuts::clipboard_binding())?;
        app.global_shortcut()
            .register(shortcut)
            .map_err(|error| error.to_string())?;
        send_registered_hotkey(app, nami::shortcuts::clipboard_binding(), 5)?;
        *app.state::<AppState>().target.lock().unwrap() = None;
        Ok(())
    })
}

fn send_registered_hotkey(
    app: &AppHandle,
    binding: crate::shortcuts::KeyBinding,
    kind: u32,
) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let shortcut = crate::binding_shortcut(&binding)?;
    if !app.global_shortcut().is_registered(shortcut) {
        return Err("global hotkey not registered".into());
    }
    // Deliver at the registered Carbon handler without posting to the global input stream.
    let hotkey = [u32::from_be_bytes(*b"htrs"), shortcut.id()];
    let mut event = std::ptr::null_mut();
    unsafe {
        let created = CreateEvent(
            std::ptr::null(),
            u32::from_be_bytes(*b"keyb"),
            kind,
            0.0,
            0,
            &mut event,
        );
        if created != 0 || event.is_null() {
            return Err(format!("create clipboard global hotkey event: {created}"));
        }
        let param = SetEventParameter(
            event,
            u32::from_be_bytes(*b"----"),
            u32::from_be_bytes(*b"hkid"),
            8,
            hotkey.as_ptr().cast(),
        );
        let result = if param == 0 {
            SendEventToEventTarget(event, GetApplicationEventTarget())
        } else {
            param
        };
        ReleaseEvent(event);
        if result != 0 {
            return Err(format!("dispatch global hotkey event: {result}"));
        }
    }
    Ok(())
}

fn set_test_paste_format(app: &AppHandle, plain: bool) -> Result<(), String> {
    on_main(app, |app| {
        create(app, "settings")?;
        Ok(())
    })?;
    wait_view_dom(
        app,
        "settings",
        "return {ready:!!window.__TAURI_INTERNALS__};",
    )?;
    evaluate_view_dom(
        app,
        "settings",
        &format!(
            r#"
window.__pasteFormat=null;
const invoke=window.__TAURI_INTERNALS__.invoke;
invoke('clipboard_behavior_set',{{pasteAsPlainText:{plain}}}).then(async()=>{{
  const options=await invoke('clipboard_options_get');
  return options.pasteAsPlainText;
}}).then(value=>window.__pasteFormat={{value}},error=>window.__pasteFormat={{error:String(error)}});
return true;
"#
        ),
    )?;
    let result = wait_view_dom(
        app,
        "settings",
        "return {ready:!!window.__pasteFormat,...window.__pasteFormat};",
    )?;
    if result["value"] != plain {
        return Err(format!("could not save paste format: {result}"));
    }
    let state = app.state::<AppState>();
    let database = state.database.lock().unwrap();
    if crate::clipboard_preferences::ClipboardOptions::load(
        database.as_ref().ok_or("missing_test_database")?,
    )?
    .paste_as_plain_text
        != plain
    {
        return Err("paste format was not restored from the database".into());
    }
    Ok(())
}

fn run_clipboard_quick_paste(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| -> Result<(), String> {
        let state = app.state::<AppState>();
        *state.target.lock().unwrap() = None;
        let now = chrono::Utc::now().to_rfc3339();
        let formatting = nami::clipboard::ClipboardTextFormatting {
            html: Some(b"<b>Synthetic p1-favorite</b>".to_vec()),
            rtf: Some(br"{\rtf1\b Synthetic p1-favorite}".to_vec()),
        };
        let plain = "Synthetic p1-favorite";
        let path = std::env::temp_dir()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        {
            let mut database = state.database.lock().unwrap();
            let db = database.as_mut().ok_or("isolated test database")?;
            for (id, pinned, favorite, bundle) in [
                ("p1-favorite", false, true, "test.browser"),
                ("p1-pinned", true, true, "test.browser"),
                ("p1-other", false, false, "test.other"),
            ] {
                db.clipboard_insert_capture(
                    &nami::clipboard::ClipboardRecord {
                        id: id.into(),
                        kind: if id == "p1-other" { "files" } else { "text" }.into(),
                        text: if id == "p1-other" {
                            serde_json::json!([path]).to_string()
                        } else {
                            format!("Synthetic {id}")
                        },
                        date: now.clone(),
                        is_favorite: favorite,
                        is_pinned: pinned,
                        image_path: None,
                        fingerprint: id.into(),
                        source_app: Some(serde_json::json!({"bundleId": bundle, "name": bundle})),
                    },
                    None,
                    (id == "p1-favorite").then_some(&formatting),
                )?;
            }
        }
        records_mode(app, "clipboard")?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
const panel=document.querySelector('.clipboard-panel');
panel.querySelector('.history-favorites').click();
panel.querySelector('input[type=search]').focus();
return true;
"#,
        )?;
        samples.push(wait_records(app, r#"
const panel=document.querySelector('.clipboard-panel'), rows=[...panel.querySelectorAll('.history-row:not([hidden])')];
return {ready:rows.length===2&&rows.some(row=>row.dataset.recordId==='p1-pinned'&&!row.querySelector('[aria-keyshortcuts]'))&&rows.some(row=>row.dataset.recordId==='p1-favorite'&&row.querySelector('[aria-keyshortcuts="Meta+1"]')), action:'Favorites group real IPC', rows:rows.map(row=>row.dataset.recordId)};
"#)?);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
const heading=document.querySelector('[data-record-id="p1-favorite"] .history-row-heading');
window.__quickPasteIconWidth=heading.querySelector('.history-origin').getBoundingClientRect().width;
document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Meta',code:'MetaLeft',metaKey:true,bubbles:true}));
return true;
"#,
        )?;
        samples.push(wait_records(app, r#"
const hint=document.querySelector('.clipboard-quick-paste-hint'), hints=document.querySelectorAll('.clipboard-quick-paste-hint');
return {ready:hints.length===1&&hint.textContent==='1'&&hint.closest('[data-record-id]').dataset.recordId==='p1-favorite'&&hint.getBoundingClientRect().width===window.__quickPasteIconWidth, action:'Command badge identifies the correct row without changing width'};
"#)?);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
document.activeElement.dispatchEvent(new KeyboardEvent('keyup',{key:'Meta',code:'MetaLeft',bubbles:true}));return true;
"#,
        )?;
        samples.push(wait_records(app, r#"return {ready:!document.querySelector('.clipboard-quick-paste-hint'),action:'Command release restores source icon'};"#)?);
        for plain_mode in [false, true] {
            if plain_mode {
                set_test_paste_format(app, true)?;
            }
            let expected = if plain_mode {
                crate::platform::ClipboardPayload::Text(plain.into())
            } else {
                crate::platform::ClipboardPayload::RichText {
                    text: plain.into(),
                    formatting: formatting.clone(),
                }
            };
            for entry in ["number", "menu", "click"] {
                let version = on_main(app, |_| Ok(crate::platform::clipboard_version()))?;
                match entry {
                    "number" => {
                        evaluate_view_dom(app, "clipboard", "document.querySelector('.clipboard-panel input[type=search]').focus();return true;")?;
                        post_records_key(app, 18, 1 << 20, "1", "1", false)?;
                    }
                    "menu" => {
                        evaluate_view_dom(
                            app,
                            "clipboard",
                            r#"document.querySelector('[data-record-id="p1-favorite"]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true}));return true;"#,
                        )?;
                        wait_records(
                            app,
                            r#"
const items=[...document.querySelectorAll('[role=menuitem]')];
const paste=items.find(item=>item.textContent==='Paste');
if(!paste)return {ready:false};
if(items.some(item=>item.textContent==='Paste as Plain Text'))throw Error('duplicate paste entry');
paste.click();return {ready:true};
"#,
                        )?;
                    }
                    _ => {
                        evaluate_view_dom(
                            app,
                            "clipboard",
                            r#"document.querySelector('[data-record-id="p1-favorite"] .history-row-select').click();return true;"#,
                        )?;
                    }
                }
                let mut actual = None;
                for _ in 0..40 {
                    actual = on_main(app, move |_| {
                        Ok(crate::platform::read_clipboard(|_| false)?
                            .filter(|capture| capture.version != version)
                            .map(|capture| capture.payload))
                    })?;
                    if actual.is_some() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                if actual.as_ref() != Some(&expected) {
                    return Err(format!("{entry} did not write the expected format (plain={plain_mode}): {actual:?}"));
                }
                samples.push(wait_records(app, &format!(r#"return {{ready:document.querySelector('.clipboard-panel .record-feedback')?.textContent.includes('Press ⌘V to paste manually'),entry:{entry:?},plain:{plain_mode}}};"#))?);
            }
            on_main(app, move |_| {
                crate::platform::test_paste_into_editor(plain, !plain_mode)
            })?;
            samples.push(serde_json::json!({"nativeEditorReceived":true,"plain":plain_mode}));
        }
        for (id, action, expected, status) in [
            (
                "p1-favorite",
                "copy",
                crate::platform::ClipboardPayload::RichText {
                    text: plain.into(),
                    formatting,
                },
                "copied",
            ),
            (
                "p1-other",
                "paste",
                crate::platform::ClipboardPayload::Files(vec![path]),
                "failed",
            ),
        ] {
            evaluate_view_dom(
                app,
                "clipboard",
                &format!(
                    r#"
window.__pasteFormatUse=null;
window.__TAURI_INTERNALS__.invoke('clipboard_record_use',{{recordId:{id:?},action:{action:?}}})
  .then(result=>window.__pasteFormatUse=result,error=>window.__pasteFormatUse={{error:String(error)}});
return true;
"#
                ),
            )?;
            let result = wait_records(
                app,
                "return {ready:!!window.__pasteFormatUse,...window.__pasteFormatUse};",
            )?;
            let actual = on_main(app, |_| {
                Ok(crate::platform::read_clipboard(|_| false)?.map(|capture| capture.payload))
            })?;
            if result["status"] != status
                || result["copyStatus"] != "current"
                || actual != Some(expected)
            {
                return Err(format!(
                    "plain-paste preference affected {id} / {action}: {result}, {actual:?}"
                ));
            }
            samples.push(
                serde_json::json!({"record":id,"action":action,"originalFormatPreserved":true}),
            );
        }
        Ok(())
    })();
    DomCase {
        name: "scoped numbered clipboard paste",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_clipboard_translate_shortcut(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        let record = nami::clipboard::ClipboardRecord {
            id: "translate-hotkey".into(),
            kind: "text".into(),
            text: "Synthetic clipboard Command-T".into(),
            date: "2099-01-01T00:00:00Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
            image_path: None,
            fingerprint: "translate-hotkey".into(),
        };
        app.state::<AppState>()
            .database
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("isolated test database")?
            .clipboard_insert(&record)?;
        on_main(app, |app| {
            crate::sync_global_shortcuts(app);
            show(app, "clipboard")
        })?;
        wait_records(
            app,
            r#"const row=document.querySelector('[data-keyboard-current]');return {ready:document.hasFocus()&&row?.dataset.recordId==='translate-hotkey',focused:document.hasFocus(),record:row?.dataset.recordId};"#,
        )?;
        let command_t = || {
            on_main(app, |app| {
                for event in [5, 6] {
                    send_registered_hotkey(
                        app,
                        nami::shortcuts::KeyBinding::new("KeyT", &["meta"]),
                        event,
                    )?;
                }
                Ok(())
            })
        };
        command_t()?;
        samples.push(wait_dom(app, r#"
const input=document.querySelector('.source-input');return {ready:input?.value==='Synthetic clipboard Command-T',action:'Command-T translated the clipboard record'};
"#)?);
        samples.push(wait_records(app, r#"
const row=document.querySelector('#translation-history .history-items .history-row');
const button=row?.querySelector('.history-row-select');
return {ready:document.hasFocus()&&document.activeElement===button&&button?.getAttribute('aria-pressed')==='true'&&row?.hasAttribute('data-keyboard-current')&&button.textContent.includes('Synthetic clipboard Command-T'),action:'New translation record selected and focused',record:row?.dataset.recordId,focused:document.hasFocus(),activeId:document.activeElement?.closest('[data-record-id]')?.dataset.recordId,activeTag:document.activeElement?.tagName,selected:button?.getAttribute('aria-pressed'),current:row?.hasAttribute('data-keyboard-current')};
"#)?);
        samples.push(wait_dom(app, r#"
return {ready:document.querySelector('.source-input')?.value==='Synthetic clipboard Command-T'&&document.body.textContent.includes('Synthetic translation en'),action:'Focused row matches the completed translation'};
"#)?);
        // Verify the global fallback without capturing selection from another app.
        on_main(app, |app| {
            app.state::<AppState>()
                .translation_preferences
                .lock()
                .unwrap()
                .selection_enabled = false;
            app.get_webview("translation")
                .ok_or("translation webview")?
                .set_focus()
                .map_err(|_| "translation focus")?;
            Ok(())
        })?;
        wait_dom(app, "return {ready:document.hasFocus()};")?;
        command_t()?;
        samples.push(wait_dom(app, r#"
const input=document.querySelector('.source-input');return {ready:input?.value==='',action:'Command-T with right pane focused opened blank translation'};
"#)?);
        Ok::<(), String>(())
    })();
    DomCase {
        name: "clipboard Command-T focus routing",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_clipboard_shortcuts_smoke(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let mut owned_image = None;
    let result = (|| {
        const CMD: usize = 1 << 20;
        const OPT: usize = 1 << 19;
        const SHIFT: usize = 1 << 17;
        let press = |code, flags, chars, unmodified| {
            post_records_key(app, code, flags, chars, unmodified, false)
        };
        on_main(app, |_| crate::platform::test_clipboard_history())?;
        samples.push(
            serde_json::json!({"privateClipboardExcluded":true,"independentNamedPasteboard":true}),
        );
        let check = |action: &str, predicate: &str| {
            wait_records(
                app,
                &format!(
                    r#"
const panel=document.querySelector('.clipboard-panel'),search=panel?.querySelector('input[type=search]'),row=panel?.querySelector('[data-keyboard-current]'),
    marked=(name)=>row?.querySelector(`[data-action=${{name}}] button`)?.getAttribute('aria-pressed')==='true',
    focused=document.activeElement===search,keys=window.__smokeKeys,count=panel?.querySelector('.history-record-count')?.textContent;
return {{ready:({predicate}),action:{action:?},record:row?.dataset.recordId,focused,activeTag:document.activeElement?.tagName,activeRow:document.activeElement?.closest('[data-record-id]')?.dataset.recordId,editor:!!document.querySelector('.clipboard-editor'),keys:keys?.slice(-2),count}};
"#
                ),
            )
        };
        {
            let state = app.state::<AppState>();
            let mut db = state.database.lock().unwrap();
            let db = db.as_mut().ok_or("isolated test database")?;
            for (id, text) in [
                ("smoke-keep", "Synthetic shortcut smoke keep"),
                ("smoke-normal", "Synthetic shortcut smoke normal"),
                ("smoke-first", "Synthetic shortcut smoke first"),
                ("smoke-second", "Synthetic shortcut smoke second"),
            ] {
                db.clipboard_insert(&nami::clipboard::ClipboardRecord {
                    id: id.into(),
                    kind: "text".into(),
                    text: text.into(),
                    date: "2099-01-01T00:00:00Z".into(),
                    is_favorite: false,
                    is_pinned: false,
                    source_app: None,
                    image_path: None,
                    fingerprint: id.into(),
                })?;
            }
            state.clipboard_options.lock().unwrap().activation =
                crate::clipboard_preferences::Activation::Copy;
        }
        summon_with_clipboard_hotkey(app)?;
        samples.push(check("Open Clipboard History", "focused&&document.hasFocus()&&row?.dataset.recordId==='smoke-second'&&count==='4 records'")?);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
window.__smokeKeys=[];
document.addEventListener('keydown',event=>window.__smokeKeys.push({key:event.key,code:event.code,trusted:event.isTrusted,repeat:event.repeat}),true);
document.addEventListener('copy',event=>window.__smokeKeys.push({key:'Copy',trusted:event.isTrusted}),true);
return true;
"#,
        )?;
        press(40, CMD, "k", "k")?;
        samples.push(check("Show Shortcuts", "panel?.querySelector('.history-shortcuts-panel')?.textContent.includes('Focus Input')&&panel.querySelector('.history-shortcuts-panel').textContent.includes('↵')&&!panel.querySelector('.history-shortcuts-panel').textContent.includes('Open Translation')&&keys.at(-1)?.trusted")?);
        press(3, CMD, "f", "f")?;
        samples.push(check("Focus Input", "focused&&!panel?.querySelector('.history-shortcuts-panel')&&keys.at(-1)?.code==='KeyF'&&keys.at(-1)?.trusted")?);
        for pinned in [true, false] {
            press(35, CMD | SHIFT, "P", "P")?;
            samples.push(check("Pin / Unpin Window", &format!("panel?.querySelector('.history-pin')?.getAttribute('aria-pressed')==='{pinned}'&&keys.at(-1)?.trusted"))?);
        }
        press(40, CMD, "k", "k")?;
        check(
            "Keep the running record view open",
            "!!panel?.querySelector('.history-shortcuts-panel')&&!focused",
        )?;
        on_main(app, |app| {
            create(app, "settings")?;
            Ok(())
        })?;
        wait_view_dom(
            app,
            "settings",
            "return {ready:!!document.querySelector('.settings-sidebar')};",
        )?;
        evaluate_view_dom(
            app,
            "settings",
            r#"
window.__focusShortcutSaved=false;
window.__TAURI_INTERNALS__.invoke('translation_shortcut_set', {action:'focusSearch',binding:{code:'KeyF',modifiers:['meta','shift']}}).then(
  ()=>{window.__focusShortcutSaved=true;},error=>{window.__focusShortcutSaved=String(error);}
);
return true;
"#,
        )?;
        wait_view_dom(
            app,
            "settings",
            r#"
if(typeof window.__focusShortcutSaved==='string')throw Error(window.__focusShortcutSaved);
return {ready:window.__focusShortcutSaved===true};
"#,
        )?;
        check("Receive changed hotkey in the open record view", "panel?.querySelector('.history-shortcuts-panel')?.textContent.includes('Focus Input⇧ ⌘ F')")?;
        press(3, CMD | SHIFT, "F", "F")?;
        samples.push(check("Focus Input after remapping", "focused&&!panel?.querySelector('.history-shortcuts-panel')&&keys.at(-1)?.code==='KeyF'&&keys.at(-1)?.trusted")?);
        press(125, 0, "\u{f701}", "\u{f701}")?;
        samples.push(check("Down focuses the record list", "document.activeElement===row?.querySelector('.history-row-select')&&row?.dataset.recordId==='smoke-second'&&keys.at(-1)?.trusted")?);
        press(125, 0, "\u{f701}", "\u{f701}")?;
        samples.push(check("Down", "document.activeElement===row?.querySelector('.history-row-select')&&row?.dataset.recordId==='smoke-first'&&keys.at(-1)?.trusted")?);
        press(126, 0, "\u{f700}", "\u{f700}")?;
        samples.push(check("Up", "document.activeElement===row?.querySelector('.history-row-select')&&row?.dataset.recordId==='smoke-second'&&keys.at(-1)?.trusted")?);
        evaluate_view_dom(
            app,
            "settings",
            r#"
window.__navigationSaved=false;
window.__TAURI_INTERNALS__.invoke('clipboard_record_shortcut_set',{action:'navigateDown',binding:{code:'F6',modifiers:[]}}).then(()=>window.__navigationSaved=true,error=>window.__navigationSaved=String(error));
return true;
"#,
        )?;
        wait_view_dom(app,"settings", "if(typeof window.__navigationSaved==='string')throw Error(window.__navigationSaved);return {ready:window.__navigationSaved===true};")?;
        press(97, 0, "\u{f709}", "\u{f709}")?;
        samples.push(check("Navigate down after remapping to F6", "document.activeElement===row?.querySelector('.history-row-select')&&row?.dataset.recordId==='smoke-first'&&keys.at(-1)?.code==='F6'&&keys.at(-1)?.trusted")?);
        press(126, 0, "\u{f700}", "\u{f700}")?;
        check(
            "Return to first record",
            "document.activeElement===row?.querySelector('.history-row-select')&&row?.dataset.recordId==='smoke-second'&&keys.at(-1)?.trusted",
        )?;
        press(124, 0, "\u{f703}", "\u{f703}")?;
        samples.push(check("Next category", "panel?.querySelector('.history-favorites')?.getAttribute('aria-pressed')==='true'&&keys.at(-1)?.trusted")?);
        press(123, 0, "\u{f702}", "\u{f702}")?;
        check("Previous category", "panel?.querySelector('.history-favorites')?.getAttribute('aria-pressed')==='false'&&keys.at(-1)?.trusted")?;
        press(48, 0, "\t", "\t")?;
        samples.push(check("Tab to translation records", "document.querySelector('.records-workspace')?.dataset.recordView==='translation'&&document.activeElement===document.querySelector('#translation-history input[type=search]')&&keys.at(-1)?.trusted")?);
        press(48, 0, "\t", "\t")?;
        check("Tab to clipboard records", "focused&&document.querySelector('.records-workspace')?.dataset.recordView==='clipboard'&&keys.at(-1)?.trusted")?;
        press(45, CMD | SHIFT, "N", "N")?;
        samples.push(check("Note", "keys.at(-1)?.trusted&&document.querySelector('.clipboard-editor textarea')?.value==='Synthetic shortcut smoke second'")?);
        press(53, 0, "\u{1b}", "\u{1b}")?;
        check(
            "Close note",
            "!document.querySelector('.clipboard-editor')&&document.activeElement===row?.querySelector('.history-row-select')",
        )?;
        press(3, CMD | SHIFT, "F", "F")?;
        check(
            "Return to search",
            "focused&&keys.at(-1)?.code==='KeyF'&&keys.at(-1)?.trusted",
        )?;
        press(16, CMD, "y", "y")?;
        samples.push(check("Preview", "panel?.querySelector('.records-details')?.getAttribute('aria-expanded')==='true'&&keys.at(-1)?.trusted")?);
        press(16, CMD, "y", "y")?;
        check("Close Preview", "focused&&panel?.querySelector('.records-details')?.getAttribute('aria-expanded')==='false'")?;
        let activate = |action, copy_flags: Option<usize>| -> Result<serde_json::Value, String> {
            let before = on_main(app, |_| {
                let version = crate::platform::write_text("Synthetic smoke sentinel")
                    .map_err(|(code, _)| code.to_owned())?;
                Ok(version)
            })?;
            if let Some(flags) = copy_flags {
                let character = if flags & SHIFT != 0 { "C" } else { "c" };
                press(8, flags, character, character)?;
            } else {
                press(36, 0, "\r", "\r")?;
            }
            let mut outcome = None;
            for _ in 0..60 {
                let result = on_main(app, |_| {
                    let payload =
                        crate::platform::read_clipboard(|_| false)?.map(|capture| capture.payload);
                    Ok((crate::platform::clipboard_version(), payload))
                })?;
                if result.0 != before
                    && result.1
                        == Some(crate::platform::ClipboardPayload::Text(
                            "Synthetic shortcut smoke second".into(),
                        ))
                {
                    outcome = Some(result.0);
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if outcome.is_none() {
                let input = evaluate_view_dom(app, "clipboard", "const i=document.activeElement;return {focused:document.hasFocus(),target:i?.tagName,selected:i?.selectionStart!==i?.selectionEnd,docSelection:document.getSelection()?.isCollapsed,keys:window.__smokeKeys.slice(-4)};")?;
                return Err(format!(
                    "{action} did not write the intended synthetic record: {input}"
                ));
            }
            check(action, "(keys.at(-1)?.trusted||keys.at(-2)?.key==='Copy'&&keys.at(-2)?.trusted)&&panel?.getClientRects().length>0&&row?.dataset.recordId==='smoke-second'")
        };
        samples.push(activate("Return (Copy Only)", None)?);
        samples.push(activate("Copy record", Some(CMD | SHIFT))?);
        check(
            "Copy feedback appears",
            "!!row?.querySelector('.record-copied')",
        )?;
        let version = on_main(app, |_| Ok(crate::platform::clipboard_version()))?;
        press(8, CMD, "c", "c")?;
        check(
            "System text copy preserved",
            "keys.at(-1)?.key==='Copy'&&keys.at(-1)?.trusted",
        )?;
        press(36, OPT, "\r", "\r")?;
        samples.push(check(
            "Dedicated Paste remains disabled",
            "keys.at(-1)?.code==='Enter'&&keys.at(-1)?.trusted",
        )?);
        if on_main(app, |_| Ok(crate::platform::clipboard_version()))? != version {
            return Err("retired shortcut used a record".into());
        }
        evaluate_view_dom(
            app,
            "settings",
            r#"
window.__copyShortcutSaved=false;
(async()=>{const invoke=window.__TAURI_INTERNALS__.invoke;await invoke('clipboard_record_shortcut_set',{action:'copyRecord',binding:{code:'KeyC',modifiers:['meta']}});await invoke('clipboard_record_shortcut_set',{action:'focusRecords',binding:{code:'F8',modifiers:[]}});})().then(()=>window.__copyShortcutSaved=true,error=>window.__copyShortcutSaved=String(error));return true;
"#,
        )?;
        wait_view_dom(app,"settings", "if(typeof window.__copyShortcutSaved==='string')throw Error(window.__copyShortcutSaved);return {ready:window.__copyShortcutSaved===true};")?;
        press(100, 0, "\u{f70b}", "\u{f70b}")?;
        check(
            "Focus record with custom F8 for Command-C",
            "document.activeElement===row?.querySelector('.history-row-select')&&!row?.querySelector('.record-copied')",
        )?;
        samples.push(activate("Command-C copies focused record", Some(CMD))?);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
const input=document.querySelector('.clipboard-panel input[type=search]');
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'Synthetic');input.dispatchEvent(new Event('input',{bubbles:true}));input.focus();input.setSelectionRange(0,4);
window.__nativeSearchCopy=null;
window.addEventListener('copy',event=>{if(event.target!==input)return;window.__nativeSearchCopy={text:input.value.slice(input.selectionStart,input.selectionEnd),trusted:event.isTrusted,handled:event.defaultPrevented};event.preventDefault();},{once:true});
return true;
"#,
        )?;
        press(8, CMD, "c", "c")?;
        samples.push(check("Command-C preserves search selection", "window.__nativeSearchCopy?.text==='Synt'&&window.__nativeSearchCopy?.trusted&&!window.__nativeSearchCopy?.handled")?);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
const input=document.querySelector('.clipboard-panel input[type=search]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'');input.dispatchEvent(new Event('input',{bubbles:true}));return true;
"#,
        )?;
        check(
            "Clear synthetic search",
            "focused&&search.value===''&&row?.dataset.recordId==='smoke-second'",
        )?;
        // Without a previous application, exercise Paste up to its explicit target check.
        let (send, receive) = mpsc::sync_channel(1);
        let view = app.get_webview("clipboard").ok_or("clipboard view")?;
        let listener = view.listen("clipboard_use_failed", move |event| {
            let _ = send.try_send(event.payload().to_owned());
        });
        *app.state::<AppState>().target.lock().unwrap() = None;
        let state = app.state::<AppState>();
        state.clipboard_options.lock().unwrap().activation =
            crate::clipboard_preferences::Activation::Paste;
        let controls = state.clipboard_options.lock().unwrap().controls();
        view.emit("clipboard_controls_changed", controls)
            .map_err(|error| error.to_string())?;
        samples.push(activate("Return (Paste into Previous App)", None)?);
        let failure = receive
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Paste did not attempt the previous-app path")?;
        view.unlisten(listener);
        let failure: serde_json::Value =
            serde_json::from_str(&failure).map_err(|error| error.to_string())?;
        if failure["code"] != "target_unavailable" || failure["copyStatus"] != "current" {
            return Err(format!(
                "Paste did not reach the expected safety boundary: {failure}"
            ));
        }
        press(3, OPT, "ƒ", "f")?;
        samples.push(check(
            "Favorite",
            "marked('favorite')&&keys.at(-1)?.trusted",
        )?);
        let search_for = |query: &str| {
            evaluate_view_dom(
                app,
                "clipboard",
                &format!(
                    r#"
const search=document.querySelector('.clipboard-panel input[type=search]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(search,{query:?});search.dispatchEvent(new Event('input',{{bubbles:true}}));return true;
"#
                ),
            )
        };
        search_for("smoke first")?;
        check(
            "Find record to pin",
            "focused&&row?.dataset.recordId==='smoke-first'&&count==='1 record'",
        )?;
        press(35, OPT, "π", "p")?;
        samples.push(check("Pin", "marked('pin')&&keys.at(-1)?.trusted")?);
        for (kind, label) in [
            ("normal", "regular"),
            ("second", "favorite"),
            ("first", "pinned"),
        ] {
            let query = format!("smoke {kind}");
            let id = format!("smoke-{kind}");
            search_for(&query)?;
            let marked = match kind {
                "second" => "&&marked('favorite')",
                "first" => "&&marked('pin')",
                _ => "",
            };
            let (before, after) = if kind == "second" {
                ("4 records", "3 records")
            } else {
                ("1 record", "0 records")
            };
            check(
                "Search loaded",
                &format!("focused&&row?.dataset.recordId==={id:?}&&count==={before:?}{marked}"),
            )?;
            if kind == "normal" {
                post_records_key(app, 51, CMD, "\u{7f}", "\u{7f}", true)?;
                check(
                    "Delete repeat ignored",
                    "keys.at(-1)?.repeat&&count==='1 record'",
                )?;
            }
            press(51, CMD, "\u{7f}", "\u{7f}")?;
            samples.push(check(
                &format!("Delete directly: {label}"),
                &format!(
                    "focused&&count==={after:?}&&search?.value==={query:?}&&keys.at(-1)?.trusted"
                ),
            )?);
            let state = app.state::<AppState>();
            let database = state.database.lock().unwrap();
            let db = database.as_ref().ok_or("isolated test database")?;
            if db.clipboard_record(&id).is_ok() || db.clipboard_record("smoke-keep").is_err() {
                return Err(format!("Delete changed the wrong stored records: {id}"));
            }
        }
        press(53, 0, "\u{1b}", "\u{1b}")?;
        samples.push(check(
            "Escape hides history",
            "!panel?.getClientRects().length||panel.closest('[inert]')!==null",
        )?);
        let image = on_main(app, |_| clipboard_text_fixture(true))?;
        let root = crate::clipboard_runtime::image_root(app)?;
        let (path, created) = nami::clipboard::store_image_with_status(&root, &image, "png")?;
        if created {
            owned_image = Some(root.join(&path));
        }
        app.state::<AppState>()
            .database
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("isolated test database")?
            .clipboard_insert(&nami::clipboard::ClipboardRecord {
                id: "smoke-ocr".into(),
                kind: "image".into(),
                text: "Synthetic shortcut image".into(),
                date: "2099-01-02T00:00:00Z".into(),
                is_favorite: false,
                is_pinned: false,
                source_app: None,
                image_path: Some(path),
                fingerprint: "smoke-ocr".into(),
            })?;
        // Prepare the real Vision result before checking interactive shortcut readiness.
        if !tauri::async_runtime::block_on(crate::clipboard_text::has_text(
            app,
            "smoke-ocr".into(),
        ))? {
            return Err("Vision found no text in the synthetic shortcut image".into());
        }
        crate::clipboard_runtime::changed(app)?;
        on_main(app, |app| resume_navigation(app, "clipboard"))?;
        check(
            "OCR image ready with privacy descriptions",
            "focused&&document.hasFocus()&&row?.dataset.recordId==='smoke-ocr'
             &&row.querySelector('[data-action=ocr] button')?.getAttribute('aria-description')==='Recognizes text on this Mac without sending the image or text.'
             &&row.querySelector('[data-action=translate] button')?.getAttribute('aria-description')==='Sends text to the selected translation service.'",
        )?;
        press(31, CMD | SHIFT, "O", "O")?;
        let ocr = check(
            "Extract Text",
            "focused&&count==='6 records'&&keys.at(-1)?.code==='KeyO'&&keys.at(-1)?.trusted",
        )?;
        let text = on_main(app, |_| {
            let capture = crate::platform::read_clipboard(|_| false)?
                .ok_or("OCR did not populate the isolated pasteboard")?;
            match capture.payload {
                crate::platform::ClipboardPayload::Text(text)
                    if text.to_uppercase().contains("HELLO NAMI") =>
                {
                    Ok(text)
                }
                _ => Err("OCR copied unexpected content".into()),
            }
        })?;
        {
            let state = app.state::<AppState>();
            let database = state.database.lock().unwrap();
            let page = database
                .as_ref()
                .ok_or("isolated test database")?
                .clipboard_page("HELLO NAMI", false, None, 10)?;
            if page.records.is_empty() {
                return Err("OCR did not save the recognized text".into());
            }
        }
        samples.push(ocr);
        search_for("smoke keep")?;
        check(
            "Translate text ready",
            "focused&&row?.dataset.recordId==='smoke-keep'&&count==='1 record'",
        )?;
        press(17, CMD, "t", "t")?;
        samples.push(wait_dom(app, r#"
const input=document.querySelector('.source-input');return {ready:input?.value==='Synthetic shortcut smoke keep',action:'Translate'};
"#)?);
        if on_main(app, |_| {
            Ok(crate::platform::read_clipboard(|_| false)?.map(|capture| capture.payload))
        })? != Some(crate::platform::ClipboardPayload::Text(text))
        {
            return Err("Translate unexpectedly changed the isolated pasteboard".into());
        }
        evaluate_view_dom(
            app,
            "settings",
            r#"
window.__listFocusSaved=false;
window.__TAURI_INTERNALS__.invoke('records_open_focus_set',{focus:'list'}).then(()=>window.__listFocusSaved=true,error=>window.__listFocusSaved=String(error));
return true;
"#,
        )?;
        wait_view_dom(app,"settings", "if(typeof window.__listFocusSaved==='string')throw Error(window.__listFocusSaved);return {ready:window.__listFocusSaved===true};")?;
        {
            let db = app.state::<AppState>();
            let db = db.database.lock().unwrap();
            let persisted = db
                .as_ref()
                .ok_or("isolated test database")?
                .preference("records_open_focus")?;
            if persisted.as_deref() != Some(r#""list""#) {
                return Err("opening focus was not saved".into());
            }
        }
        on_main(app, |app| resume_navigation(app, "clipboard"))?;
        samples.push(check("Open with record list focus", "document.hasFocus()&&!!row&&document.activeElement===row.querySelector('.history-row-select')&&!focused")?);

        Ok(())
    })();
    if let Some(path) = owned_image {
        let _ = std::fs::remove_file(path);
    }
    DomCase {
        name: "clipboard shortcut smoke through macOS and WebView",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_clipboard_menu_paste(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        on_main(app, |_| crate::platform::test_rich_text_capture_limit())?;
        let rich = crate::platform::ClipboardPayload::RichText {
            text: "Synthetic rich clipboard smoke".into(),
            formatting: nami::clipboard::ClipboardTextFormatting {
                html: Some(b"<b>Synthetic rich clipboard smoke</b>".to_vec()),
                rtf: Some(br"{\rtf1\b Synthetic rich clipboard smoke}".to_vec()),
            },
        };
        let capture = on_main(app, {
            let rich = rich.clone();
            move |app| {
                let version = crate::platform::write_clipboard(&rich).map_err(|(code, _)| code)?;
                app.state::<AppState>()
                    .clipboard_version
                    .store(version, Ordering::SeqCst);
                crate::platform::read_clipboard(|_| false)?
                    .ok_or_else(|| "missing synthetic rich clipboard capture".into())
            }
        })?;
        if capture.payload != rich {
            return Err("HTML/RTF did not round-trip AppKit capture".into());
        }
        let root = crate::clipboard_runtime::image_root(app)?;
        let rich_id = {
            let state = app.state::<AppState>();
            let mut database = state.database.lock().unwrap();
            let database = database.as_mut().ok_or("isolated test database")?;
            crate::clipboard_runtime::persist_capture(database, &root, capture)?;
            database
                .clipboard_page("Synthetic rich clipboard smoke", false, None, 10)?
                .records
                .into_iter()
                .next()
                .ok_or("rich capture not searchable as plaintext")?
                .id
        };
        records_mode(app, "clipboard")?;
        *app.state::<AppState>().target.lock().unwrap() = None;
        let paste_record = |id: &str, action: &str| {
            evaluate_view_dom(
                app,
                "clipboard",
                &format!(
                    r#"
window.__smokePaste=null;
window.__TAURI_INTERNALS__.invoke('clipboard_record_use',{{recordId:{id:?},action:{action:?}}})
  .then(result=>window.__smokePaste=result,error=>window.__smokePaste={{error:String(error)}});
return true;
"#
                ),
            )?;
            wait_records(
                app,
                "return {ready:!!window.__smokePaste,...window.__smokePaste};",
            )
        };
        let check_paste = |id: &str, action: &str, expected| {
            let result = paste_record(id, action)?;
            if result["code"] != "target_unavailable" || result["copyStatus"] != "current" {
                return Err(format!("{action} safety boundary: {result}"));
            }
            let actual = on_main(app, |_| {
                Ok(crate::platform::read_clipboard(|_| false)?.map(|capture| capture.payload))
            })?;
            if actual != Some(expected) {
                return Err(format!("isolated pasteboard did not match {action}"));
            }
            Ok(())
        };
        set_test_paste_format(app, false)?;
        for (action, plain) in [("pastePlain", true), ("paste", false), ("paste", true)] {
            if action == "paste" && plain {
                set_test_paste_format(app, true)?;
            }
            let expected = if plain {
                crate::platform::ClipboardPayload::Text("Synthetic rich clipboard smoke".into())
            } else {
                rich.clone()
            };
            if action == "paste" && plain {
                check_paste(&rich_id, action, expected)?;
            } else {
                wait_records(
                    app,
                    &format!(
                        r#"
const row=document.querySelector('.clipboard-panel [data-record-id='+CSS.escape({rich_id:?})+']');
if(!row||!document.hasFocus())return {{ready:false}};
row.dispatchEvent(new MouseEvent('contextmenu',{{bubbles:true,cancelable:true,clientX:100,clientY:100}}));
return {{ready:true}};
"#
                    ),
                )?;
                wait_records(app, "return {ready:!!document.activeElement?.closest('.record-context-menu-keys')};")?;
                let version = on_main(app, |_| Ok(crate::platform::clipboard_version()))?;
                post_records_key(app, 36, if plain { 1 << 20 } else { 0 }, "\r", "\r", false)?;
                samples.push(wait_records(
                    app,
                    r#"
const menuOpen=!!document.querySelector('.record-context-menu');
const message=document.querySelector('.clipboard-panel .record-feedback')?.textContent;
return {ready:!menuOpen&&message==='Content copied. Press ⌘V to paste manually.',message,menuOpen};
"#,
                )?);
                let matched = on_main(app, move |_| {
                    Ok(crate::platform::clipboard_version() != version
                        && crate::platform::read_clipboard(|_| false)?
                            .is_some_and(|capture| capture.payload == expected))
                })?;
                if !matched {
                    return Err(format!(
                        "menu shortcut did not write the expected {action} payload"
                    ));
                }
            }
            on_main(app, move |_| {
                crate::platform::test_paste_into_editor("Synthetic rich clipboard smoke", !plain)
            })?;
            samples.push(serde_json::json!({"pasteAction":action,"plainPaste":plain,"noTarget":true,"bytesMatch":true,"nativeEditorReceived":true}));
        }
        set_test_paste_format(app, false)?;
        samples.push(serde_json::json!({"oversizedHtmlFallsBack":true}));
        let fixture =
            std::env::temp_dir().join(format!("nami-open-smoke-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&fixture).map_err(|error| error.to_string())?;
        let first = fixture.join("first.txt");
        let second = fixture.join("second file.txt");
        let open_targets = (|| {
            for path in [&first, &second] {
                std::fs::write(path, "Synthetic clipboard open smoke")
                    .map_err(|error| error.to_string())?;
            }
            {
                let state = app.state::<AppState>();
                let mut database = state.database.lock().unwrap();
                let database = database.as_mut().ok_or("isolated test database")?;
                for (id, kind, text, title) in [
                    (
                        "smoke-open-url",
                        "text",
                        format!("www.example.test/{}", "a".repeat(2200)),
                        "Open smoke web",
                    ),
                    (
                        "smoke-open-mail",
                        "text",
                        "test+tag@example.test".into(),
                        "Open smoke mail",
                    ),
                    (
                        "smoke-open-plain",
                        "text",
                        format!(
                            "https://example.test/{}\nNot a standalone link",
                            "a".repeat(2200)
                        ),
                        "Open smoke plain",
                    ),
                    (
                        "smoke-open-files",
                        "files",
                        serde_json::to_string(&[&first, &second]).unwrap(),
                        "Open smoke group",
                    ),
                    (
                        "smoke-open-file",
                        "files",
                        serde_json::to_string(&[&first]).unwrap(),
                        "Open smoke single",
                    ),
                ] {
                    database.clipboard_insert(&nami::clipboard::ClipboardRecord {
                        id: id.into(),
                        kind: kind.into(),
                        text,
                        date: "2099-01-01T00:00:00Z".into(),
                        is_favorite: false,
                        is_pinned: false,
                        source_app: None,
                        image_path: None,
                        fingerprint: id.into(),
                    })?;
                    database.clipboard_edit(id, title, None)?;
                }
            }
            for (id, query, paths) in [
                ("smoke-open-file", "Open smoke single", vec![&first]),
                (
                    "smoke-open-files",
                    "Open smoke group",
                    vec![&first, &second],
                ),
            ] {
                let paths: Vec<_> = paths
                    .iter()
                    .map(|path| path.to_str().unwrap().to_owned())
                    .collect();
                check_paste(
                    id,
                    "pastePaths",
                    crate::platform::ClipboardPayload::Text(paths.join("\n")),
                )?;
                check_paste(id, "paste", crate::platform::ClipboardPayload::Files(paths))?;
                evaluate_view_dom(
                    app,
                    "clipboard",
                    &format!(
                        r#"
const search=document.querySelector('.clipboard-panel input[type=search]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(search,{query:?});search.dispatchEvent(new Event('input',{{bubbles:true}}));return true;
"#
                    ),
                )?;
                wait_records(app, &format!("return {{ready:document.querySelector('.clipboard-panel [data-keyboard-current]')?.dataset.recordId==={id:?}&&document.querySelector('.clipboard-panel .history-record-count')?.textContent==='1 record'}};"))?;
                evaluate_view_dom(
                    app,
                    "clipboard",
                    &format!(
                        r#"
const row=document.querySelector('[data-record-id='+{id:?}+']');row.dispatchEvent(new MouseEvent('contextmenu',{{bubbles:true,cancelable:true,clientX:40,clientY:100}}));return true;
"#
                    ),
                )?;
                samples.push(wait_records(app, r#"
const menu=document.querySelector('.clipboard-context-menu');if(!menu)return {ready:false};
const items=[...menu.querySelectorAll('[role=menuitem],[role=separator]')].map(el=>el.getAttribute('role')==='separator'?'/':el.querySelector('.record-menu-label').textContent);
const expected=['Paste','Paste as Path','Copy','/','Show in Finder','/','Favorite','Pin','Note','/','Delete'];
if(JSON.stringify(items)!==JSON.stringify(expected))throw Error('file menu order: '+items);
return {ready:true,fileMenu:items,pathPaste:true,ordinaryFilePaste:true};
"#)?);
                if id == "smoke-open-files" {
                    evaluate_view_dom(app, "clipboard", "[...document.querySelectorAll('.clipboard-context-menu .record-menu-label')].find(el=>el.textContent==='Show in Finder').closest('button').click();return true;")?;
                } else {
                    evaluate_view_dom(app, "clipboard", "document.body.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}));return true;")?;
                }
            }
            let before = on_main(app, |_| Ok(crate::platform::clipboard_version()))?;
            if paste_record(&rich_id, "pastePaths")?["error"] != "invalid_argument"
                || paste_record("smoke-open-files", "pastePlain")?["error"] != "invalid_argument"
                || on_main(app, |_| Ok(crate::platform::clipboard_version()))? != before
            {
                return Err(
                    "wrong-kind paste must be rejected without changing the pasteboard".into(),
                );
            }
            samples.push(serde_json::json!({"wrongKindPasteRejected":true}));
            evaluate_view_dom(
                app,
                "clipboard",
                r#"
const invoke=window.__TAURI_INTERNALS__.invoke;
window.__openTypes=null;Promise.all(['web','mail','plain'].map(id=>invoke('clipboard_open_target',{recordId:'smoke-open-'+(id==='web'?'url':id),execute:false}))).then(value=>window.__openTypes=value,error=>window.__openTypes=String(error));return true;
"#,
            )?;
            samples.push(wait_records(app, r#"return {ready:JSON.stringify(window.__openTypes)===JSON.stringify(['link','email',null]),openTypes:window.__openTypes};"#)?);
            let choose = r#"
const buttons=[...document.querySelectorAll('.clipboard-detail button.clipboard-file')];return {ready:buttons.length===2&&buttons[1].getAttribute('aria-label')==='Show in Finder: second file.txt'&&!buttons[1].disabled,action:'Choose either file in details'};
"#;
            samples.push(wait_dom(app, choose)?);
            // Removing the second file after details load catches routing to the wrong index.
            std::fs::remove_file(&second).map_err(|error| error.to_string())?;
            evaluate_view_dom(app, "translation", "document.querySelectorAll('.clipboard-detail button.clipboard-file')[1].click();return true;")?;
            samples.push(wait_dom(app, r#"
const pane=document.querySelector('.clipboard-detail');return {ready:pane?.textContent.includes('The original file was moved or deleted and cannot be located'),action:'Revalidate the selected file at execution time'};
"#)?);
            evaluate_view_dom(
                app,
                "translation",
                r#"
window.__finderDispatch='pending';window.__TAURI_INTERNALS__.invoke('clipboard_open_target',{recordId:'smoke-open-files',fileIndex:0,execute:true}).then(value=>window.__finderDispatch=value,error=>window.__finderDispatch=String(error));return true;
"#,
            )?;
            samples.push(wait_dom(app, r#"return {ready:window.__finderDispatch==='file',action:'Hand off the remaining synthetic file to Finder',result:window.__finderDispatch};"#)?);
            Ok::<(), String>(())
        })();
        let _ = std::fs::remove_dir_all(fixture);
        open_targets?;

        Ok(())
    })();
    DomCase {
        name: "clipboard menu formats and file reveal",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_record_display_shortcuts(app: &AppHandle) -> DomCase {
    let mut samples = Vec::new();
    let result = (|| {
        const CMD: usize = 1 << 20;
        let press = |label, code, chars| {
            on_main(app, move |app| {
                let window = app.get_window(label).ok_or("shortcut window")?;
                let native = window
                    .ns_window()
                    .map_err(|_| "shortcut window")?
                    .cast::<AnyObject>();
                activate_application();
                unsafe {
                    let _: () = objc2::msg_send![native, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
                }
                Ok(())
            })?;
            post_window_key(app, label, code, CMD, chars, chars, false)
        };
        let wait_visible = |label: &'static str, expected| {
            for _ in 0..50 {
                let visible = on_main(app, move |app| {
                    app.get_window(label)
                        .ok_or("shortcut window")?
                        .is_visible()
                        .map_err(|error| error.to_string())
                })?;
                if visible == expected {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(format!(
                "Shortcut did not set {label} visibility to {expected}"
            ))
        };
        on_main(app, |app| {
            let state = app.state::<AppState>();
            let mut regions = state.windows.lock().unwrap();
            regions.history_pinned = true;
            regions.translation_pinned = true;
            drop(regions);
            create(app, "settings")?;
            Ok(())
        })?;
        let source =
            "First preview line\nSecond preview line\nThird preview line\nFourth preview line";
        {
            let state = app.state::<AppState>();
            let mut database = state.database.lock().unwrap();
            let db = database.as_mut().ok_or("isolated database")?;
            db.clipboard_insert(&nami::clipboard::ClipboardRecord {
                id: "smoke-preview-text".into(),
                kind: "text".into(),
                text: source.into(),
                date: "2099-01-01T00:00:00Z".into(),
                is_favorite: false,
                is_pinned: false,
                source_app: None,
                image_path: None,
                fingerprint: "smoke-preview-text".into(),
            })?;
        }
        let translation = nami::model::HistoryRecord {
            id: "smoke-preview-source".into(),
            title: String::new(),
            text: source.into(),
            source_language: "en".into(),
            translation: None,
            target_languages: vec!["fr".into()],
            translations: vec!["Synthetic translation".into()],
            date: "2099-01-01T00:00:00Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
        };
        on_main(app, move |app| history_add(app, translation))?;
        crate::clipboard_runtime::changed(app)?;
        let measure = |id: &str, lines| {
            format!(
                r#"
const node=document.querySelector('[data-record-id={id}] .history-text-preview');
const clamp=node&&getComputedStyle(node).webkitLineClamp,height=node?.getBoundingClientRect().height;
return {{ready:clamp==='{lines}'&&height>0&&node.scrollHeight>height,action:'Text preview height',id:{id:?},clamp,height}};
"#
            )
        };
        on_main(app, |app| resume_navigation(app, "clipboard"))?;
        let baseline = wait_records(app, &measure("smoke-preview-text", 3))?;
        samples.push(baseline.clone());
        press("translation", 43, ",")?;
        wait_visible("settings", true)?;
        samples.push(serde_json::json!({"action":"Command-comma from Clipboard"}));
        wait_view_dom(
            app,
            "settings",
            "return {ready:!!document.querySelector('.settings-content')};",
        )?;
        evaluate_view_dom(
            app,
            "settings",
            r#"
window.__previewPreferences='pending';
(async()=>{
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const before=(await invoke('app_snapshot')).clipboardControls.previewLines;
  if(before.clipboard!==3||before.translation!==3)throw Error('incorrect default preview lines');
  for(const lines of [0,4]){
    const error=await invoke('records_preview_lines_set',{view:'clipboard',lines}).then(()=>null,String);
    if(error!=='invalid_argument')throw Error('out-of-range preview lines were accepted');
  }
  await invoke('records_preview_lines_set',{view:'clipboard',lines:1});
  await invoke('records_preview_lines_set',{view:'translation',lines:2});
})().then(()=>window.__previewPreferences='saved',error=>window.__previewPreferences=String(error));return true;
"#,
        )?;
        samples.push(wait_view_dom(app,"settings",r#"return {ready:window.__previewPreferences==='saved',action:'Save distinct preview preferences and reject 0/4',status:window.__previewPreferences};"#)?);
        {
            let state = app.state::<AppState>();
            let database = state.database.lock().unwrap();
            let reloaded = crate::clipboard_preferences::ClipboardOptions::load(
                database.as_ref().ok_or("isolated database")?,
            )?;
            if serde_json::to_value(reloaded.controls()).map_err(|error| error.to_string())?
                ["previewLines"]
                != serde_json::json!({"clipboard":1,"translation":2})
            {
                return Err("Preview preferences did not survive reload".into());
            }
        }
        press("settings", 13, "w")?;
        wait_visible("settings", false)?;
        on_main(app, |app| resume_navigation(app, "clipboard"))?;
        let compact = wait_records(app, &measure("smoke-preview-text", 1))?;
        if compact["height"].as_f64().unwrap_or_default()
            >= baseline["height"].as_f64().unwrap_or_default()
        {
            return Err("Reducing the clipboard line limit did not shrink the preview".into());
        }
        samples.push(compact);
        on_main(app, |app| set_record_view(app, "translation"))?;
        samples.push(wait_records(app, &measure("smoke-preview-source", 2))?);

        for target in ["clipboard", "translation"] {
            on_main(app, move |app| resume_navigation(app, target))?;
            wait_visible("translation", true)?;
            if target == "translation" {
                press("translation", 43, ",")?;
                wait_visible("settings", true)?;
                samples.push(serde_json::json!({"action":"Command-comma from Translation"}));
                press("settings", 13, "w")?;
                wait_visible("settings", false)?;
                on_main(app, |app| resume_navigation(app, "translation"))?;
            }
            press("translation", 13, "w")?;
            wait_visible("translation", false)?;
            samples.push(serde_json::json!({"action":"Command-W", "target":target, "hidden":true}));
        }

        Ok(())
    })();
    DomCase {
        name: "record previews and macOS window shortcuts",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
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
        samples.push(wait_records(app, r#"
const p=document.querySelector('.clipboard-panel'),input=p.querySelector('.history-search input');
const rows=[...p.querySelectorAll('.history-row:not([hidden])')];
if(rows.length<2||document.activeElement!==input)return {ready:false};
const [first,second]=rows, bg=row=>getComputedStyle(row).backgroundColor, base=bg(first);
if(rows.some(row=>bg(row)!==base)||p.querySelector('[data-record-target]'))throw Error('search opening has a highlighted record');
second.dispatchEvent(new MouseEvent('mousemove',{clientX:18,clientY:60,bubbles:true}));
return {ready:true,phase:'search opening before pointer move'};
"#)?);
        samples.push(wait_records(app, r#"
const p=document.querySelector('.clipboard-panel'),rows=[...p.querySelectorAll('.history-row:not([hidden])')];
const [first,second]=rows, bg=row=>getComputedStyle(row).backgroundColor;
return {ready:document.activeElement===p.querySelector('.history-search input')&&p.querySelector('[data-record-target]')===second&&rows.filter(row=>bg(row)!==bg(first)).length===1,phase:'one mouse target without stealing search focus'};
"#)?);
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

struct CrossAppFixture {
    app: AppHandle,
    bundle: std::path::PathBuf,
    report: std::path::PathBuf,
    next_line: std::cell::Cell<usize>,
    pid: u32,
}

impl Drop for CrossAppFixture {
    fn drop(&mut self) {
        let bundle = self.bundle.clone();
        let _ = on_main(&self.app, move |_| {
            crate::platform::test_close_fixture(&bundle);
            Ok(())
        });
    }
}

impl CrossAppFixture {
    fn start(app: &AppHandle, executable: &std::path::Path, mode: &str) -> Result<Self, String> {
        let bundle = executable
            .ancestors()
            .nth(3)
            .ok_or("missing fixture bundle")?
            .to_owned();
        let report = bundle.with_file_name(format!("fixture-{mode}-{}.log", uuid::Uuid::new_v4()));
        std::fs::File::create_new(&report).map_err(|error| error.to_string())?;
        let mut fixture = Self {
            app: app.clone(),
            bundle,
            report,
            next_line: std::cell::Cell::new(0),
            pid: 0,
        };
        // LaunchServices assigns the launch identity that production target checks require.
        let status = std::process::Command::new("/usr/bin/open")
            .args(["-n", "--stdout"])
            .arg(&fixture.report)
            .arg(&fixture.bundle)
            .args(["--args", "--smoke", mode])
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err("LaunchServices could not open the synthetic target".into());
        }
        fixture.pid = fixture
            .wait_event()?
            .strip_prefix("READY ")
            .and_then(|pid| pid.parse().ok())
            .filter(|pid| *pid > 0)
            .ok_or("invalid synthetic READY marker")?;
        Ok(fixture)
    }

    fn next_event(&self) -> Result<Option<String>, String> {
        let data = std::fs::read_to_string(&self.report).map_err(|error| error.to_string())?;
        let line = data
            .split_inclusive('\n')
            .nth(self.next_line.get())
            .and_then(|line| line.strip_suffix('\n'));
        if line.is_some() {
            self.next_line.set(self.next_line.get() + 1);
        }
        Ok(line.map(str::to_owned))
    }

    fn wait_event(&self) -> Result<String, String> {
        for _ in 0..200 {
            if let Some(event) = self.next_event()? {
                return Ok(event);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Err("timed out waiting for synthetic target response".into())
    }

    fn expect(&self, expected: &str) -> Result<(), String> {
        if self.wait_event()? == expected {
            Ok(())
        } else {
            Err(format!("synthetic target did not report {expected}"))
        }
    }

    fn focused(&self, app: &AppHandle) -> Result<crate::platform::Target, String> {
        let pid = self.pid;
        let bundle = self.bundle.clone();
        let target = on_main(app, move |_| {
            crate::platform::test_activate_fixture(pid, &bundle).map_err(str::to_owned)
        })?;
        for _ in 0..100 {
            if on_main(app, move |_| Ok(crate::platform::target_focused(target)))? {
                return Ok(target);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Err("the synthetic target never became frontmost; no keys sent".into())
    }
}

fn run_cross_app(app: &AppHandle) -> DomCase {
    const TEXT: &str = "NAMI synthetic selection";
    const MARKER: &str = "NAMI synthetic clipboard baseline";
    let mut samples = Vec::new();
    let result = (|| {
        let mode = std::env::var("NAMI_UI_TEST_CROSS_APP").unwrap_or_default();
        let expected_identity = if mode == "denied-fresh" {
            "dev.local.nami.ui-tests.denial"
        } else {
            "dev.local.nami.ui-tests"
        };
        if app.config().identifier != expected_identity {
            return Err("cross-app smoke needs the matching isolated Nami identity".into());
        }
        if !matches!(
            mode.as_str(),
            "selection" | "full" | "denied" | "denied-fresh"
        ) {
            return Err("set NAMI_UI_TEST_CROSS_APP=selection, full (dedicated test session only), denied, or denied-fresh".into());
        }
        let denied = matches!(mode.as_str(), "denied" | "denied-fresh");
        let path =
            std::env::var_os("NAMI_UI_TEST_FIXTURE").ok_or("missing synthetic target bundle")?;
        let path = std::path::Path::new(&path);
        if !path.ends_with("Nami Cross-App Fixture.app/Contents/MacOS/cross-app-fixture") {
            return Err("invalid synthetic target bundle path".into());
        }
        let trusted = on_main(app, |_| Ok(crate::platform::accessibility()))?;
        if trusted == denied {
            return Err("isolated Nami's current Accessibility permission does not match the requested mode; permission is changed only by the user".into());
        }
        let buffer = |text: &'static str| {
            on_main(app, move |app| {
                let version = crate::platform::write_text(text).map_err(|(code, _)| code)?;
                app.state::<AppState>()
                    .clipboard_version
                    .store(version, Ordering::SeqCst);
                Ok(version)
            })
        };
        let is_buffer = |text: &'static str| {
            on_main(app, move |_| {
                Ok(crate::platform::test_selection_buffer_text().as_deref() == Some(text))
            })
        };
        let mut previous = None;
        let fixture_modes: &[&str] = if trusted {
            &["ax", "manual-ax", "copy", "copy-delayed", "empty"]
        } else {
            &["ax"]
        };
        let expect_event = |fixture: &CrossAppFixture, expected: &str| -> Result<(), String> {
            for _ in 0..120 {
                if let Some(event) = fixture.next_event()? {
                    if event == expected {
                        return Ok(());
                    }
                    if event == "AX ENABLED" {
                        continue;
                    }
                    return Err(format!("unexpected synthetic target event: {event}"));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(format!("timed out waiting for {expected}"))
        };
        let expect_no_copy = |fixture: &CrossAppFixture| -> Result<(), String> {
            for _ in 0..12 {
                if let Some(event) = fixture.next_event()? {
                    if event.starts_with("COPY ") || event.starts_with("PASTE ") {
                        return Err(format!("unexpected synthetic clipboard action: {event}"));
                    }
                    if event != "AX ENABLED" {
                        return Err(format!("unexpected synthetic target event: {event}"));
                    }
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(())
        };
        for &fixture_mode in fixture_modes {
            if app.get_window("translation").is_some() {
                on_main(app, |app| crate::windows::hide(app, "translation"))?;
                wait_motion(app)?;
            }
            let version = buffer(MARKER)?;
            crate::platform::test_set_last_foreign(None);
            let fixture = CrossAppFixture::start(app, path, fixture_mode)?;
            let target = fixture.focused(app)?;
            let mut tracked = false;
            for _ in 0..40 {
                if crate::platform::test_last_foreign().map(|target| target.pid())
                    == Some(fixture.pid as i32)
                {
                    tracked = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            if !tracked {
                return Err("the frontmost activation tracker missed the synthetic target".into());
            }
            // Regression: a previous shortcut target must not outrank the
            // application that is frontmost when this shortcut fires. The
            // stale entry is a live process, so it survives liveness checks.
            if let Some(stale) = crate::platform::test_self_target() {
                crate::platform::test_set_last_foreign(Some(stale));
                let captured = [
                    crate::platform::capture_target(),
                    crate::platform::capture_selection_target(),
                ];
                crate::platform::test_set_last_foreign(None);
                if captured
                    .into_iter()
                    .any(|target| target.map(|target| target.pid()) != Some(fixture.pid as i32))
                {
                    return Err(format!(
                        "target resolution ignored the frontmost synthetic {fixture_mode} target"
                    ));
                }
            }
            let expected = if trusted && fixture_mode != "empty" {
                TEXT
            } else {
                ""
            };
            if trusted {
                let direct = crate::platform::selected_text(Some(target));
                let matches = match fixture_mode {
                    "ax" => direct.as_deref() == Ok(TEXT),
                    "empty" => direct.as_deref() == Ok(""),
                    _ => {
                        direct.as_deref() == Ok("")
                            || matches!(
                                direct,
                                Err("selection_unavailable" | "selection_focus_unavailable")
                            )
                    }
                };
                if !matches {
                    return Err(format!(
                        "unexpected AX status from synthetic {fixture_mode} target: {:?}",
                        direct.as_ref().map(|_| "text")
                    ));
                }
            }
            let start = std::time::Instant::now();
            on_main(app, move |app| {
                crate::selection::test_summon_from(app, Some(target), true)
            })?;
            if matches!(fixture_mode, "copy" | "copy-delayed") {
                expect_event(&fixture, &format!("COPY {TEXT}")).map_err(|error| {
                    let running = app
                        .state::<AppState>()
                        .selection_busy
                        .load(Ordering::SeqCst);
                    format!("{error}; selection still running: {running}")
                })?;
            } else if fixture_mode == "manual-ax" {
                expect_event(&fixture, "AX ENABLED")?;
            }
            wait_dom(
                app,
                &format!(
                    r#"
const input=document.querySelector('.source-input'),alert=document.querySelector('.translation-status [role=alert]');
return {{ready:input?.value==={expected:?}&&!input.closest('[inert]')&&({trusted}||alert?.textContent.includes('Accessibility')),hasInput:!!input,sourceMatches:input?.value==={expected:?},inert:!!input?.closest('[inert]'),alert:alert?.textContent}};
"#
                ),
            )?;
            let elapsed = start.elapsed().as_millis();
            if !matches!(fixture_mode, "copy" | "copy-delayed") {
                expect_no_copy(&fixture)?;
            }
            if !is_buffer(MARKER)? {
                return Err(
                    "selection changed the isolated baseline instead of preserving it".into(),
                );
            }
            if (!matches!(fixture_mode, "copy" | "copy-delayed") && trusted)
                && on_main(app, |_| Ok(crate::platform::clipboard_version()))? != version
            {
                return Err(
                    "selection changed the isolated baseline instead of preserving it".into(),
                );
            }
            samples.push(serde_json::json!({"process":fixture_mode,"axAuthorized":trusted,"clipboardPreserved":true,"copyReceived":matches!(fixture_mode, "copy" | "copy-delayed"),"elapsedMs":elapsed}));
            previous = Some(target);
        }
        if trusted {
            const EXCLUDED: &str = "dev.local.nami.cross-app-fixture";
            let fixture = CrossAppFixture::start(app, path, "copy")?;
            let target = fixture.focused(app)?;
            on_main(app, move |_| {
                Ok(crate::platform::test_copy_from_fixture(target)?)
            })?;
            fixture.expect(&format!("COPY {TEXT}"))?;
            on_main(app, |app| {
                crate::windows::resume_navigation(app, "clipboard")
            })?;
            let host = crate::platform::test_self_target().ok_or("isolated host identity")?;
            for _ in 0..50 {
                if on_main(app, move |_| Ok(crate::platform::target_focused(host)))? {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            on_main(app, move |_| {
                if !crate::platform::target_focused(host) {
                    return Err("isolated host did not take focus from excluded fixture".into());
                }
                if crate::platform::read_clipboard(|bundle| bundle == EXCLUDED)?.is_some() {
                    return Err(
                        "unmarked copy from the excluded fixture was accepted after switching apps"
                            .into(),
                    );
                }
                let allowed =
                    crate::platform::read_clipboard(|_| false)?.ok_or("missing control copy")?;
                if allowed.source_app.is_some()
                    || allowed.payload != crate::platform::ClipboardPayload::Text(TEXT.into())
                {
                    return Err("ambiguous control copy was lost or assigned a source".into());
                }
                crate::platform::write_text(MARKER).map_err(|(code, _)| code)?;
                let fresh = crate::platform::read_clipboard(|bundle| bundle == EXCLUDED)?;
                if fresh.map(|capture| capture.payload)
                    != Some(crate::platform::ClipboardPayload::Text(MARKER.into()))
                {
                    return Err("old exclusion suppressed the next clipboard version".into());
                }
                Ok(())
            })?;
            samples.push(serde_json::json!({"excludedCopyAfterFocusSwitchRejected":true,"ambiguousAllowedCopyUnlabelled":true,"freshCopyAllowed":true}));
        }
        let previous = previous.ok_or("no synthetic target ran")?;
        for _ in 0..100 {
            if !on_main(app, move |_| Ok(crate::platform::target_alive(previous)))? {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if on_main(app, move |_| Ok(crate::platform::target_alive(previous)))? {
            return Err("synthetic target did not terminate".into());
        }
        {
            let state = app.state::<AppState>();
            state
                .database
                .lock()
                .unwrap()
                .as_mut()
                .ok_or("test database")?
                .clipboard_insert(&nami::clipboard::ClipboardRecord {
                    id: "cross-app-paste".into(),
                    kind: "text".into(),
                    text: TEXT.into(),
                    date: "2099-01-01T00:00:00Z".into(),
                    is_favorite: false,
                    is_pinned: false,
                    source_app: None,
                    image_path: None,
                    fingerprint: "cross-app-paste".into(),
                })?;
        }
        crate::clipboard_runtime::changed(app)?;
        records_mode(app, "clipboard")?;
        let paste = || -> Result<serde_json::Value, String> {
            evaluate_view_dom(
                app,
                "clipboard",
                r#"
window.__fixturePaste=null;
window.__TAURI_INTERNALS__.invoke('clipboard_record_use',{recordId:'cross-app-paste',action:'paste'})
    .then(value=>window.__fixturePaste=value,error=>window.__fixturePaste={error:String(error)});
return true;
"#,
            )?;
            wait_records(
                app,
                "return {ready:!!window.__fixturePaste,...window.__fixturePaste};",
            )
        };
        let fixture = CrossAppFixture::start(app, path, "paste")?;
        let target = fixture.focused(app)?;
        if trusted {
            // A stale version guarantees this preflight can never dispatch a key, even if focus races.
            let old_version = buffer(MARKER)?;
            let current = buffer("NAMI synthetic replacement copy")?;
            if current == old_version {
                return Err("isolated clipboard version did not advance".into());
            }
            let changed = on_main(app, move |_| {
                Ok(crate::platform::paste_keys_down(target, old_version))
            })?;
            if changed != Err("clipboard_changed") || !is_buffer("NAMI synthetic replacement copy")?
            {
                return Err("paste preflight accepted an overwritten clipboard".into());
            }
            on_main(app, |app| {
                crate::windows::resume_navigation(app, "clipboard")
            })?;
            for _ in 0..50 {
                if on_main(app, move |_| Ok(!crate::platform::target_focused(target)))? {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let lost = on_main(app, move |_| {
                if !crate::platform::target_alive(target) {
                    return Err("live focus test target terminated".into());
                }
                Ok(crate::platform::paste_keys_down(target, old_version))
            })?;
            if lost != Err("target_unavailable") || fixture.next_event()?.is_some() {
                return Err(
                    "paste preflight did not reject a live target after focus moved".into(),
                );
            }
            samples.push(serde_json::json!({"changedVersionRejected":true,"liveUnfocusedTargetRejected":true}));
        }
        if denied || mode == "full" {
            *app.state::<AppState>().target.lock().unwrap() = Some(target);
            let result = paste()?;
            if mode == "full" {
                fixture.expect(&format!("PASTE [{TEXT}]"))?;
                if result["status"] != "dispatched" {
                    return Err(format!(
                        "paste was received but host reported failure: {result}"
                    ));
                }
                samples.push(serde_json::json!({"actualExternalPasteReceived":true}));
            } else {
                if result["code"] != "permission_required" || fixture.next_event()?.is_some() {
                    return Err(format!(
                        "permission denial did not protect target: {result}"
                    ));
                }
                samples
                    .push(serde_json::json!({"pastePermissionDenied":true,"noPasteReceived":true,"freshIdentity":mode=="denied-fresh"}));
            }
        }
        if trusted {
            *app.state::<AppState>().target.lock().unwrap() = Some(previous);
            let result = paste()?;
            if result["code"] != "target_unavailable"
                || result["copyStatus"] != "current"
                || fixture.next_event()?.is_some()
            {
                return Err(format!("dead target protection failed: {result}"));
            }
            wait_records(app, "return {ready:document.querySelector('.record-feedback')?.textContent.includes('Press ⌘V to paste manually')};")?;
            samples.push(
                serde_json::json!({"exitedPasteTargetRejected":true,"manualPasteHintVisible":true}),
            );
        }
        Ok(())
    })();
    DomCase {
        name: "cross-process selection and clipboard safety",
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
            on_main(app, move |app| {
                activate_application();
                if index == 1 {
                    crate::selection::show_blank(app)
                } else {
                    crate::selection::test_empty_summon(app, true)
                }
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
            if index == 0 {
                sample["isolatedAccessibilityGranted"] =
                    on_main(app, |_| Ok(crate::platform::accessibility()))?.into();
            }
            if let Some(handoff) = handoff {
                sample["hideHandoff"] = handoff;
            }
            samples.push(sample);
        }
        let state = app.state::<AppState>();
        let before = state.translation_revision.load(Ordering::SeqCst);
        on_main(app, |app| crate::selection::test_empty_summon(app, false))?;
        for _ in 0..100 {
            if !state.selection_busy.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if state.selection_busy.load(Ordering::SeqCst)
            || state.translation_revision.load(Ordering::SeqCst) != before
            || state.selection.lock().unwrap().is_some()
        {
            return Err("selection-only shortcut without text interrupted a translation".into());
        }
        samples.push(serde_json::json!({"phase":"selection-only-empty-preserves-translation"}));
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
    clipboard_text_image(640.0, 160.0, text.then_some("HELLO NAMI"), None)
}

fn clipboard_text_image(
    width: f64,
    height: f64,
    text: Option<&str>,
    stamp: Option<&str>,
) -> Result<Vec<u8>, String> {
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{ns_string, NSData, NSPoint, NSRect, NSSize, NSString};
    unsafe {
        let image: *mut AnyObject = msg_send![class!(NSImage), alloc];
        let image: *mut AnyObject = msg_send![image, initWithSize: NSSize::new(width,height)];
        let image = Retained::from_raw(image).ok_or("synthetic image allocation")?;
        let _: () = msg_send![&*image, lockFocus];
        let white: Retained<AnyObject> = msg_send![class!(NSColor), whiteColor];
        let _: () = msg_send![&*white, setFill];
        let _: () = msg_send![class!(NSBezierPath), fillRect: NSRect::new(NSPoint::new(0.0,0.0),NSSize::new(width,height))];
        if let Some(text) = text {
            let attributes: Retained<AnyObject> =
                msg_send![class!(NSMutableDictionary), dictionary];
            let font: Retained<AnyObject> = msg_send![class!(NSFont), boldSystemFontOfSize: 48f64];
            let color: Retained<AnyObject> = msg_send![class!(NSColor), blackColor];
            let _: () = msg_send![&*attributes, setObject: &*font, forKey: ns_string!("NSFont")];
            let _: () = msg_send![&*attributes, setObject: &*color, forKey: ns_string!("NSColor")];
            let label = NSString::from_str(text);
            let _: () = msg_send![&*label, drawAtPoint: NSPoint::new(48.0,48.0), withAttributes: &*attributes];
            if let Some(stamp) = stamp {
                let stamp = NSString::from_str(stamp);
                let _: () = msg_send![&*stamp, drawAtPoint: NSPoint::new(48.0,125.0), withAttributes: &*attributes];
            }
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

// The probe uses the same named pasteboard and production capture loop as the OCR case.
fn run_clipboard_capture_burst(app: &AppHandle) -> DomCase {
    use std::time::Instant;

    unsafe extern "C" {
        fn clock() -> std::ffi::c_ulong;
    }
    let mut samples = Vec::new();
    let result = (|| -> Result<(), String> {
        if app.config().identifier != "dev.local.nami.ui-tests" {
            return Err("capture burst requires the isolated native host".into());
        }
        let baseline = std::env::var("NAMI_UI_TEST_CAPTURE_BASELINE").as_deref() == Ok("1");
        let state = app.state::<AppState>();
        *state.database.lock().unwrap() = Some(crate::database::HistoryDatabase::open(
            std::path::Path::new(":memory:"),
        )?);
        on_main(app, move |app| {
            if baseline {
                crate::clipboard_runtime::start_with_poll_interval(app, Duration::from_millis(350));
            } else {
                crate::clipboard_runtime::start(app);
            }
            app.state::<AppState>()
                .clipboard_enabled
                .store(true, Ordering::SeqCst);
            Ok(())
        })?;
        std::thread::sleep(Duration::from_millis(500));
        let started = Instant::now();
        let cpu_started = unsafe { clock() };
        std::thread::sleep(Duration::from_secs(4));
        let cpu_ms = unsafe { clock() }.saturating_sub(cpu_started) as f64 / 1000.0;
        samples.push(serde_json::json!({"phase":"idle", "baseline":baseline, "wallMs":started.elapsed().as_millis(), "processCpuMs":cpu_ms}));

        let nonce = uuid::Uuid::new_v4();
        let mut observed = std::collections::HashMap::new();
        let observe =
            |observed: &mut std::collections::HashMap<String, Instant>| -> Result<(), String> {
                let database = state.database.lock().unwrap();
                let page = database
                    .as_ref()
                    .ok_or("test database")?
                    .clipboard_page("", false, None, 100)?;
                for record in page.records {
                    observed.entry(record.text).or_insert_with(Instant::now);
                }
                Ok(())
            };
        for spacing in [70, 220, 450] {
            let mut writes = Vec::new();
            for index in 0..10 {
                let text = format!(
                    "Synthetic capture {nonce} {spacing} {} {index}",
                    if index % 2 == 0 { "alpha" } else { "beta" }
                );
                let payload = text.clone();
                let started = on_main(app, move |_| {
                    crate::platform::write_text(&payload).map_err(|(code, _)| code)?;
                    Ok(Instant::now())
                })?;
                writes.push((text, started));
                let dwell = if index == 9 { 650 } else { spacing };
                while started.elapsed() < Duration::from_millis(dwell) {
                    observe(&mut observed)?;
                    std::thread::sleep(Duration::from_millis(8));
                }
            }
            let latencies: Vec<_> = writes
                .iter()
                .map(|(text, started)| {
                    observed
                        .get(text)
                        .map(|time| time.duration_since(*started).as_millis())
                })
                .collect();
            let captured = latencies.iter().flatten().count();
            let gaps = writes
                .windows(2)
                .map(|pair| pair[1].1.duration_since(pair[0].1).as_millis())
                .collect::<Vec<_>>();
            samples.push(serde_json::json!({"phase":"capture-to-queryable", "requestedSpacingMs":spacing,"actualSpacingMs":gaps,"captured":captured,"written":writes.len(),"latencyMs":latencies}));
            if latencies.last().is_none_or(Option::is_none) || (spacing == 450 && captured < 9) {
                return Err(
                    "sustained capture missed its final settled write or lost spaced copies".into(),
                );
            }
        }
        Ok(())
    })();
    app.state::<AppState>()
        .clipboard_enabled
        .store(false, Ordering::SeqCst);
    DomCase {
        name: "clipboard burst sampling and idle cost",
        passed: result.is_ok(),
        error: result.err(),
        samples,
    }
}

fn run_clipboard_image_index(app: &AppHandle) -> DomCase {
    use std::time::Instant;

    use crate::clipboard_text::VISION_CALLS;
    let mut samples = Vec::new();
    let mut media = Vec::new();
    let directory = std::env::temp_dir().join(format!("nami-index-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let state = app.state::<AppState>();
        std::fs::create_dir(&directory).map_err(|error| error.to_string())?;
        let path = directory.join("history.sqlite3");
        let database = crate::database::HistoryDatabase::open(&path)?;
        *state.database.lock().unwrap() = Some(database);
        let baseline = std::env::var("NAMI_UI_TEST_CAPTURE_BASELINE").as_deref() == Ok("1");
        on_main(app, move |app| {
            if baseline {
                crate::clipboard_runtime::start_with_poll_interval(app, Duration::from_millis(350));
            } else {
                crate::clipboard_runtime::start(app);
            }
            app.state::<AppState>()
                .clipboard_enabled
                .store(true, Ordering::SeqCst);
            Ok(())
        })?;
        let nonce = directory
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut expected_count = 0;
        let mut pending = Vec::new();
        let mut capture = |payload| -> Result<_, String> {
            let started = Instant::now();
            let version = on_main(app, move |_| {
                crate::platform::write_clipboard(&payload).map_err(|(code, _)| code.into())
            })?;
            expected_count += 1;
            loop {
                let observed = {
                    let locked = state.database.lock().unwrap();
                    let db = locked.as_ref().ok_or("test database")?;
                    let page = db.clipboard_page("", false, None, 1)?;
                    if page.total_records == expected_count
                        && state.clipboard_version.load(Ordering::SeqCst) == version
                    {
                        let summary = page.records.first().ok_or("missing captured image")?;
                        Some(db.clipboard_record(&summary.id)?)
                    } else {
                        None
                    }
                };
                if let Some(record) = observed {
                    media.push(record.image_path.clone().ok_or("missing image path")?);
                    return Ok((record, started, version));
                }
                if started.elapsed() > Duration::from_secs(5) {
                    return Err("capture timed out".into());
                }
                std::thread::sleep(Duration::from_millis(8));
            }
        };
        let fixture = |label: String, stamp: String| {
            on_main(app, move |_| {
                clipboard_text_image(1920.0, 1080.0, Some(&label), Some(&stamp))
            })
        };
        let mut last_version = 0;
        let inputs = (0..8)
            .map(|index| {
                let label = format!("Nami Error E247 海浪 Capture {index}");
                fixture(label, format!("{nonce} {index}")).map(|bytes| {
                    crate::platform::ClipboardPayload::Image {
                        bytes,
                        extension: "png".into(),
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        for payload in inputs {
            let (record, started, version) = capture(payload)?;
            last_version = version;
            let capture_ms = started.elapsed().as_secs_f64() * 1000.0;
            samples.push(serde_json::json!({"phase":"capture", "ms":capture_ms}));
            // Observe each job concurrently with the next capture, not after the whole burst.
            let app = app.clone();
            pending.push(std::thread::spawn(move || -> Result<_, String> {
                loop {
                    let lock = app.state::<AppState>();
                    let db = lock.database.lock().unwrap();
                    if let Some(text) = db
                        .as_ref()
                        .ok_or("test database")?
                        .clipboard_ocr_text(&record.id)?
                    {
                        return Ok((record.id, text, started.elapsed().as_secs_f64() * 1000.0));
                    }
                    drop(db);
                    if started.elapsed() > Duration::from_secs(10) {
                        return Err("automatic OCR timed out".into());
                    }
                    std::thread::sleep(Duration::from_millis(8));
                }
            }));
        }
        let mut ids = Vec::new();
        let mut chinese_hits = 0;
        for pending in pending {
            let (id, text, ms) = pending.join().map_err(|_| "OCR observer panicked")??;
            if !text.contains("E247") || !text.contains('\n') {
                return Err("automatic OCR did not preserve fixture text".into());
            }
            chinese_hits += usize::from(text.contains("海浪"));
            ids.push(id);
            samples.push(serde_json::json!({"phase":"capture-to-searchable", "ms":ms}));
        }
        if chinese_hits != ids.len() {
            return Err(format!(
                "Vision missed the mixed Chinese fixture: {chinese_hits}/{}",
                ids.len()
            ));
        }
        let photo = directory.join("Fixture E247 海浪.png");
        let photo_bytes = fixture("Nami Error E247 海浪 Image File".into(), nonce.clone())?;
        let dimensions = crate::platform::image_dimensions(&photo_bytes)?;
        std::fs::write(&photo, &photo_bytes).map_err(|error| error.to_string())?;
        let (file, _, _) = capture(crate::platform::ClipboardPayload::Files(vec![photo
            .to_string_lossy()
            .into_owned()]))?;
        let blank = on_main(app, |_| clipboard_text_fixture(false))?;
        let (blank, _, version) = capture(crate::platform::ClipboardPayload::Image {
            bytes: blank,
            extension: "png".into(),
        })?;
        if tauri::async_runtime::block_on(crate::clipboard_text::has_text(app, blank.id.clone()))? {
            return Err("blank image probe unexpectedly found text".into());
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let original = loop {
            {
                let lock = state.database.lock().unwrap();
                let db = lock.as_ref().ok_or("test database")?;
                if let (Some(text), Some(blank_text)) = (
                    db.clipboard_ocr_text(&file.id)?,
                    db.clipboard_ocr_text(&blank.id)?,
                ) {
                    if !blank_text.is_empty() || !text.contains("E247") {
                        return Err("unexpected OCR result".into());
                    }
                    break text;
                }
            }
            if Instant::now() >= deadline {
                return Err("file or empty OCR did not complete".into());
            }
            std::thread::sleep(Duration::from_millis(8));
        };
        if on_main(app, |_| Ok(crate::platform::clipboard_version()))? != version
            || last_version >= version
        {
            return Err("automatic OCR modified the pasteboard".into());
        }
        state.clipboard_enabled.store(false, Ordering::SeqCst);
        {
            let mut lock = state.database.lock().unwrap();
            drop(lock.take());
            let db = crate::database::HistoryDatabase::open(&path)?;
            let all = db.clipboard_page("", false, None, 100)?;
            if all.total_records != 10 || all.records.iter().any(|record| record.kind != "image") {
                return Err(
                    "automatic OCR inserted a derived text record or lost a capture".into(),
                );
            }
            for (term, count) in [("fixture e247 海浪.png", 1), ("e247", 9)] {
                let result = db.clipboard_page(term, false, None, 100)?;
                if result.total_records != count
                    || result
                        .records
                        .iter()
                        .filter(|row| row.id == file.id)
                        .count()
                        != 1
                {
                    return Err(format!(
                        "search did not return the original image exactly once: {term}"
                    ));
                }
            }
            for (term, min_count) in [
                ("E247", 9),
                ("海浪", 9),
                ("海", 9),
                ("fixture e247 海浪.png", 1),
            ] {
                let timings: Vec<f64> = (0..50)
                    .map(|_| {
                        let start = Instant::now();
                        let page = db.clipboard_page(term, false, None, 100)?;
                        if page.total_records < min_count {
                            return Err("search missed fixture".to_owned());
                        }
                        Ok(start.elapsed().as_secs_f64() * 1000.0)
                    })
                    .collect::<Result<_, _>>()?;
                samples.push(
                    serde_json::json!({"phase":"database-search", "term":term, "ms":timings}),
                );
            }
            *lock = Some(db);
        }
        tauri::async_runtime::block_on(async {
            *state.image_text.lock().await = Default::default();
        });
        let calls = VISION_CALLS.load(Ordering::SeqCst);
        if calls != 10 {
            return Err(format!(
                "expected one Vision call per new image, got {calls}"
            ));
        }
        let started = Instant::now();
        let results = tauri::async_runtime::block_on(async {
            tokio::join!(
                crate::clipboard_text::has_text(app, file.id.clone()),
                crate::clipboard_text::has_text(app, blank.id.clone())
            )
        });
        if results != (Ok(true), Ok(false)) || VISION_CALLS.load(Ordering::SeqCst) != calls {
            return Err("reopened OCR results started new Vision work".into());
        }
        samples.push(serde_json::json!({"phase":"cold-cache-reuse-two-records","ms":started.elapsed().as_secs_f64()*1000.0, "visionCalls":calls,"additionalVisionCalls":0,"chineseHits":chinese_hits,"chineseSamples":8,"dimensions":dimensions}));
        // Explicit extraction takes the persisted original (case and newlines), not the folded index.
        on_main(app, |app| {
            let state = app.state::<AppState>();
            state.windows.lock().unwrap().history_visible = true;
            Ok(())
        })?;
        tauri::async_runtime::block_on(crate::clipboard_text::process(
            app,
            file.id.clone(),
            "ocr".into(),
        ))?;
        let copied = on_main(app, |_| Ok(crate::platform::read_clipboard(|_| false)?))?;
        if !copied.is_some_and(|capture| {
            capture.payload == crate::platform::ClipboardPayload::Text(original)
        }) || VISION_CALLS.load(Ordering::SeqCst) != calls
        {
            return Err("manual extraction did not reuse the exact persisted original".into());
        }
        samples.push(serde_json::json!({"phase":"explicit-extraction-after-reopen","additionalVisionCalls":0}));
        let derived_text_id = state
            .database
            .lock()
            .unwrap()
            .as_ref()
            .ok_or("test database")?
            .clipboard_page_filtered("", false, Some("text"), None, 1)?
            .records
            .into_iter()
            .next()
            .ok_or("missing derived text fixture")?
            .id;
        if tauri::async_runtime::block_on(crate::clipboard_text::process(
            app,
            derived_text_id,
            "index".into(),
        )) != Err("invalid_argument".into())
        {
            return Err("single-image indexing accepted a text record".into());
        }
        let root = crate::clipboard_runtime::image_root(app)?;
        let retry_bytes = fixture(
            "Nami Error E247 海浪 Retry".into(),
            format!("{nonce} retry"),
        )?;
        let retry_image = nami::clipboard::store_image(&root, &retry_bytes, "png")?;
        media.push(retry_image.clone());
        let retry = state
            .database
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("test database")?
            .clipboard_insert(&nami::clipboard::ClipboardRecord {
                id: format!("synthetic-retry-{nonce}"),
                kind: "image".into(),
                text: String::new(),
                date: "2026-11-20T00:00:00Z".into(),
                is_favorite: false,
                is_pinned: false,
                source_app: None,
                image_path: Some(retry_image),
                fingerprint: nami::clipboard::fingerprint("image", &retry_bytes),
            })?;
        let image = root.join(
            retry
                .image_path
                .clone()
                .ok_or("missing retry fixture image")?,
        );
        let saved = directory.join("retry-image.png");
        // Exercise an unindexed historical image through the real IPC.
        on_main(app, |app| show(app, "translation"))?;
        wait_dom(app, "return {ready:!!window.__TAURI_INTERNALS__?.invoke};")?;
        let index = |key: &str| -> Result<serde_json::Value, String> {
            evaluate_dom(
                app,
                &format!(
                    r#"
window.__indexProbe=null;
(async()=>{{
  const invoke=window.__TAURI_INTERNALS__.invoke, recordId={id:?};
  try {{
    const before=(await invoke('clipboard_record_detail',{{recordId}})).imageIndexStatus;
    let error;
    try {{await invoke('clipboard_record_process',{{recordId,action:'index'}});}} catch(e) {{error=String(e);}}
    const after=(await invoke('clipboard_record_detail',{{recordId}})).imageIndexStatus;
    window.__indexProbe={{key:{key:?},before,after,error}};
  }} catch(e) {{window.__indexProbe={{key:{key:?},probeError:String(e)}};}}
}})();return true;
"#,
                    id = retry.id
                ),
            )?;
            wait_dom(
                app,
                &format!(
                    "return {{ready:window.__indexProbe?.key==={key:?},...window.__indexProbe}};"
                ),
            )
        };
        let before = on_main(app, |_| Ok(crate::platform::clipboard_version()))?;
        std::fs::rename(&image, &saved).map_err(|error| error.to_string())?;
        let missing = index("missing");
        std::fs::rename(&saved, &image).map_err(|error| error.to_string())?;
        let missing = missing?;
        if missing["before"] != "unindexed"
            || missing["after"] != "failed"
            || missing["error"] != "clipboard_image_missing"
            || VISION_CALLS.load(Ordering::SeqCst) != calls
        {
            return Err(format!("missing image lacked retriable status: {missing}"));
        }
        let completed = index("restored")?;
        {
            let db = state.database.lock().unwrap();
            let db = db.as_ref().ok_or("test database")?;
            if completed["before"] != "failed"
                || completed["after"] != "indexed"
                || !db
                    .clipboard_page("E247", false, None, 100)?
                    .records
                    .iter()
                    .any(|record| record.id == retry.id)
                || db.clipboard_page("", false, None, 100)?.total_records != 12
                || on_main(app, |_| Ok(crate::platform::clipboard_version()))? != before
                || VISION_CALLS.load(Ordering::SeqCst) != calls + 1
            {
                return Err(format!("single-image retry changed the clipboard or lost its searchable original: {completed}"));
            }
            samples.push(serde_json::json!({"phase":"single-image-retry-through-ipc","retryVisionCalls":1,"clipboardPreserved":true,"noDerivedRecord":true}));
        }
        let recopy_bytes = fixture(
            "Nami Error E247 海浪 Recopy".into(),
            format!("{nonce} recopy"),
        )?;
        let recopy_image = nami::clipboard::store_image(&root, &recopy_bytes, "png")?;
        media.push(recopy_image.clone());
        let recopy = state
            .database
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("test database")?
            .clipboard_insert(&nami::clipboard::ClipboardRecord {
                id: format!("synthetic-recopy-{nonce}"),
                kind: "image".into(),
                text: String::new(),
                date: "2026-11-20T00:00:00Z".into(),
                is_favorite: false,
                is_pinned: false,
                source_app: None,
                image_path: Some(recopy_image),
                fingerprint: nami::clipboard::fingerprint("image", &recopy_bytes),
            })?;
        on_main(app, |app| {
            let state = app.state::<AppState>();
            state
                .clipboard_version
                .store(crate::platform::clipboard_version(), Ordering::SeqCst);
            state.clipboard_enabled.store(true, Ordering::SeqCst);
            Ok(())
        })?;
        let (retry, retry_text) = {
            let database = state.database.lock().unwrap();
            let database = database.as_ref().ok_or("test database")?;
            (
                database.clipboard_record(&retry.id)?,
                database
                    .clipboard_ocr_text(&retry.id)?
                    .ok_or("missing retry OCR text")?,
            )
        };
        for (record, expected, additional_calls) in [
            (recopy, "E247", 1),
            (blank, "", 0),
            (retry, retry_text.as_str(), 0),
        ] {
            let path = record.image_path.as_deref().ok_or("missing recopy image")?;
            let bytes = nami::clipboard::read_image(&root, path)?;
            let calls = VISION_CALLS.load(Ordering::SeqCst);
            let version = on_main(app, move |_| {
                crate::platform::write_clipboard(&crate::platform::ClipboardPayload::Image {
                    bytes,
                    extension: "png".into(),
                })
                .map_err(|(code, _)| code.into())
            })?;
            let started = Instant::now();
            loop {
                if state.clipboard_version.load(Ordering::SeqCst) == version {
                    let lock = state.database.lock().unwrap();
                    let db = lock.as_ref().ok_or("test database")?;
                    if db.clipboard_page("", false, None, 1)?.total_records != 13 {
                        return Err("recopy created a duplicate or derived text record".into());
                    }
                    if let Some(text) = db.clipboard_ocr_text(&record.id)? {
                        let matches = if additional_calls == 0 {
                            text == expected
                        } else {
                            text.contains(expected)
                        };
                        if !matches
                            || VISION_CALLS.load(Ordering::SeqCst) != calls + additional_calls
                        {
                            return Err(
                                "recopy did not preserve or index the original OCR result".into()
                            );
                        }
                        break;
                    }
                }
                if started.elapsed() > Duration::from_secs(10) {
                    return Err("recopied image was not indexed".into());
                }
                std::thread::sleep(Duration::from_millis(8));
            }
        }
        samples.push(serde_json::json!({"phase":"recopy-image-index","additionalVisionCalls":1,"noDuplicates":true,"preservedEmptyAndIndexed":true}));
        Ok(())
    })();
    let state = app.state::<AppState>();
    state.clipboard_enabled.store(false, Ordering::SeqCst);
    drop(state.database.lock().unwrap().take());
    if let Ok(root) = crate::clipboard_runtime::image_root(app) {
        for path in media {
            let _ = nami::clipboard::remove_image(&root, &path);
        }
    }
    let _ = std::fs::remove_dir_all(directory);
    DomCase {
        name: "clipboard local image indexing and performance",
        passed: result.is_ok(),
        error: result.err(),
        samples,
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
        evaluate_dom(app, "document.querySelector('.clipboard-detail [data-action=copy] button').click();return true;")?;
        samples.push(wait_dom(app, "const detail=document.querySelector('.clipboard-detail');return {ready:!!detail?.querySelector('[data-action=copy] .copy-success')&&!!detail.getClientRects().length,detailCopied:true};")?);

        for (action, pressed) in [("favorite", "Remove favorite"), ("pin", "Unpin record")] {
            evaluate_dom(app, &format!("document.querySelector('.clipboard-detail [data-action={action}] button').click();return true;"))?;
            samples.push(wait_dom(app, &format!(r#"
const detail=document.querySelector('.clipboard-detail'), mark=detail?.querySelector('[data-action={action}] button'), button=detail?.querySelector('[data-action=delete] button');
if(mark?.getAttribute('aria-label')!=='{pressed}')return {{ready:false}};
if(!button||button.disabled)throw Error('marked records must allow explicit deletion');
return {{ready:true,deleteAvailableWith:{action:?}}};
"#))?);
            evaluate_dom(app, &format!("document.querySelector('.clipboard-detail [data-action={action}] button').click();return true;"))?;
            wait_dom(app, &format!("return {{ready:document.querySelector('.clipboard-detail [data-action={action}] button')?.getAttribute('aria-pressed')==='false'}};"))?;
        }
        let native = on_main(app, native_chrome_sample)?;
        if native["buttons"].as_array().is_none_or(|buttons| {
            buttons.len() != 1 || buttons.iter().any(|button| button["hidden"] != true)
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
            "document.querySelector('.clipboard-detail [role=scrollbar]').focus(); return true;",
        )?;
        samples.push(wait_dom(app, "const bar=document.querySelector('.clipboard-detail [role=scrollbar]');return {ready:document.activeElement===bar&&bar.classList.contains('visible'),thumb:getComputedStyle(bar.firstElementChild).opacity};")?);
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
        let before_entry = app
            .state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .clipboard_detail_id
            .clone();
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('.clipboard-panel [data-record-id="detail-second"]').dispatchEvent(new MouseEvent('mouseover',{bubbles:true}));return true;"#,
        )?;
        wait_records(app, "return {ready:true};")?;
        if app
            .state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .clipboard_detail_id
            != before_entry
        {
            return Err("stationary pointer changed clipboard details".into());
        }
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('.clipboard-panel [data-record-id="detail-second"]').dispatchEvent(new MouseEvent('mousemove',{bubbles:true}));return true;"#,
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
        let id = app
            .state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .clipboard_detail_id
            .clone()
            .ok_or("missing detail target")?;
        evaluate_dom(
            app,
            r#"const detail=document.querySelector('.clipboard-detail'),button=detail.querySelector('[data-action=delete] button');
const probe=document.createElement('span');probe.style.color='var(--history-red)';detail.append(probe);
const expected=getComputedStyle(probe).color;probe.remove();if(button.disabled||getComputedStyle(button).color!==expected)throw Error('detail delete must be enabled and red');button.click();return true;"#,
        )?;
        for _ in 0..120 {
            if app
                .state::<AppState>()
                .database
                .lock()
                .unwrap()
                .as_ref()
                .ok_or("database")?
                .clipboard_detail(&id)
                .is_err()
            {
                samples.push(serde_json::json!({"detailDeletedOnFirstClick": true}));
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err("detail record was not deleted on the first click".into())
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
        let blank = database.clipboard_record("kind-image-blank")?;
        database.clipboard_index_image(&blank.id, &blank.fingerprint, "")?;
        let before_browsing = crate::clipboard_text::VISION_CALLS.load(Ordering::SeqCst);
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
return {{ready:Math.abs(p.getBoundingClientRect().width-{width})<.1,width:p.getBoundingClientRect().width,count:box(f.querySelector('.history-record-count')),filters:box(filters),keyboard:box(f.querySelector('.history-shortcuts')),centerOffset:(Math.min(...[...filters.querySelectorAll('button')].map(b=>b.getBoundingClientRect().left))+Math.max(...[...filters.querySelectorAll('button')].map(b=>b.getBoundingClientRect().right)))/2-(p.getBoundingClientRect().left+p.getBoundingClientRect().width/2),icons:[...filters.querySelectorAll('.history-symbol')].map(box)}};
"#
                ),
            )?;
            if sample["centerOffset"].as_f64().unwrap().abs() > 0.5
                || sample["count"]["right"].as_f64().unwrap()
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
            (
                "text",
                vec!["copy", "translate", "note", "favorite", "pin", "delete"],
            ),
            (
                "image",
                vec![
                    "copy",
                    "translate",
                    "ocr",
                    "note",
                    "favorite",
                    "pin",
                    "delete",
                ],
            ),
            (
                "blank",
                vec![
                    "copy",
                    "translate",
                    "ocr",
                    "note",
                    "favorite",
                    "pin",
                    "delete",
                ],
            ),
            ("files", vec!["copy", "note", "favorite", "pin", "delete"]),
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
        for (id, action_visible) in [("kind-image-text", true), ("kind-image-blank", false)] {
            evaluate_view_dom(
                app,
                "clipboard",
                &format!(
                    r#"
const button=document.querySelector('.clipboard-panel .records-details');
const invoke=window.__TAURI_INTERNALS__.invoke,recordId={id:?};
if(button.getAttribute('aria-expanded')==='true')invoke('clipboard_details_select',{{recordId,browsing:true}});
else invoke('records_details_toggle',{{recordId}});
return true;
"#
                ),
            )?;
            let detail = wait_dom(
                app,
                &format!(
                    r#"
const pane=document.querySelector('.clipboard-detail'),image=!!pane?.querySelector('.clipboard-detail-content .clipboard-image-preview');
const index=!!pane?.querySelector('.clipboard-image-index-action button:not(:disabled)'),ocr=!!pane?.querySelector('[data-action=ocr]:not([aria-hidden=true]) button:not(:disabled)');
const status=['Image text not indexed','No text recognized','Image text is searchable'].find(text=>pane?.textContent.includes(text));
return {{ready:image&&!status&&index==={action_visible}&&ocr==={action_visible},image,status,index,ocr}};
"#
                ),
            )?;
            samples.push(detail);
        }
        evaluate_view_dom(
            app,
            "clipboard",
            "document.querySelector('.clipboard-panel .records-details').click();return true;",
        )?;
        if crate::clipboard_text::VISION_CALLS.load(Ordering::SeqCst) != before_browsing {
            return Err("opening the clipboard list or details invoked Vision".into());
        }

        let version = app
            .state::<AppState>()
            .clipboard_version
            .load(Ordering::SeqCst);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"window.__kindIpc={done:false};(async()=>{try { const invoke=window.__TAURI_INTERNALS__.invoke;const blank=await invoke('clipboard_record_has_text',{recordId:'kind-image-blank'}),image=await invoke('clipboard_record_has_text',{recordId:'kind-image-text'});let blankOcrRejected=false,textOcrRejected=false;try{await invoke('clipboard_record_process',{recordId:'kind-image-blank',action:'ocr'});}catch(error){blankOcrRejected=error==='ocr_no_text';}try{await invoke('clipboard_record_process',{recordId:'kind-text-favorite',action:'ocr'});}catch(error){textOcrRejected=error==='invalid_argument';}window.__kindIpc={done:true,blank,image,blankOcrRejected,textOcrRejected};}catch(error){window.__kindIpc={done:true,error:String(error)};}})();return true;"#,
        )?;
        let ipc = wait_records(
            app,
            "return {ready:window.__kindIpc?.done,...window.__kindIpc};",
        )?;
        let probe_only = {
            let state = app.state::<AppState>();
            let database = state.database.lock().unwrap();
            let database = database.as_ref().ok_or("test database")?;
            database
                .clipboard_page("HELLO NAMI", false, None, 100)?
                .total_records
                == 0
                && database.clipboard_page("", false, None, 100)?.total_records == 5
        };
        if ipc["blank"] != false
            || ipc["image"] != true
            || ipc["blankOcrRejected"] != true
            || ipc["textOcrRejected"] != true
            || !probe_only
            || app
                .state::<AppState>()
                .clipboard_version
                .load(Ordering::SeqCst)
                != version
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
        on_main(app, |app| {
            app.state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .translation_visible = false;
            apply_translation(app)
        })?;
        wait_motion(app)?;
        wait_dom(app, "return {ready:document.querySelector('.translation-clip')?.getAttribute('aria-hidden')==='true'};")?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('.clipboard-panel [data-record-id="kind-image-text"] [aria-label="Translate"]').click();return true;"#,
        )?;
        samples.push(wait_dom(app, r#"const text=document.querySelector('.source-input')?.value,error=document.querySelector('.translation-status [role=alert]')?.textContent;return {ready:text?.includes('HELLO NAMI')&&error==='translation_network_disabled_in_tests',text,error};"#)?);
        let before_hover = app
            .state::<AppState>()
            .translation_revision
            .load(Ordering::SeqCst);
        evaluate_view_dom(app, "clipboard", "document.querySelector('#translation-history .history-row').dispatchEvent(new MouseEvent('mousemove',{bubbles:true}));return true;")?;
        wait_records(app, "return {ready:true};")?;
        let after_hover = wait_dom(
            app,
            r#"const text=document.querySelector('.source-input')?.value;return {ready:!!text,text};"#,
        )?;
        if !after_hover["text"]
            .as_str()
            .is_some_and(|text| text.contains("HELLO NAMI"))
            || app
                .state::<AppState>()
                .translation_revision
                .load(Ordering::SeqCst)
                != before_hover
        {
            return Err(format!(
                "stationary pointer replaced the image translation: {after_hover}"
            ));
        }
        samples.push(after_hover);
        let selected = on_main(app, |app| {
            let record = app
                .state::<AppState>()
                .history
                .lock()
                .unwrap()
                .snapshot()
                .records[0]
                .clone();
            crate::commands::select_history_sample(app, Some(record.id.clone()), false, false)?;
            Ok::<_, String>(record)
        })?;
        let mut completed = selected.clone();
        completed.id = format!("background-{}", uuid::Uuid::new_v4());
        completed.is_favorite = false;
        completed.is_pinned = false;
        completed.text = "Synthetic background translation completed while browsing".into();
        let completed_id = completed.id.clone();
        let handle = app.clone();
        let (send, receive) = mpsc::channel();
        tauri::async_runtime::spawn(async move {
            let revision = handle
                .state::<AppState>()
                .translation_revision
                .load(Ordering::SeqCst);
            let result = crate::history_runtime::commit_translation(
                &handle,
                completed,
                "google".into(),
                revision,
                None,
            )
            .await;
            let _ = send.send(result.map(|_| ()));
        });
        receive
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "background commit timeout")??;
        if app
            .state::<AppState>()
            .history
            .lock()
            .unwrap()
            .selected_id()
            != Some(selected.id.as_str())
        {
            return Err("background translation replaced the browsed record".into());
        }
        if app
            .state::<AppState>()
            .database
            .lock()
            .unwrap()
            .as_ref()
            .ok_or("test database")?
            .record(&completed_id)?
            .is_none()
        {
            return Err("background translation was not saved".into());
        }
        let handle = app.clone();
        let (send, receive) = mpsc::channel();
        tauri::async_runtime::spawn(async move {
            let result =
                crate::history_runtime::update_record(&handle, completed_id, "delete".into(), None)
                    .await;
            let _ = send.send(result.map(|_| ()));
        });
        receive
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "background cleanup timeout")??;
        records_mode(app, "clipboard")?;
        wait_records(app, "return {ready:!!document.querySelector('.clipboard-panel [data-record-id=\"kind-text-favorite\"] [aria-label=\"Translate\"]')};")?;
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
return {ready:image.includes('Translate')&&image.includes('Extract text')&&image.includes('Copy')&&multiple.includes('Note')&&multiple.includes('Copy')&&!multiple.includes('Translate')&&!multiple.includes('Extract text'),image,multiple};
"#, 1500)?);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"document.querySelector('.clipboard-panel [data-record-id="kind-image-file"] [aria-label="Translate"]').click();return true;"#,
        )?;
        samples.push(wait_dom(app, r#"const text=document.querySelector('.source-input')?.value;return {ready:text?.includes('HELLO NAMI'),text};"#)?);
        records_mode(app, "clipboard")?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
window.__extractFile={done:false};
(async()=>{
  try {
    const invoke=window.__TAURI_INTERNALS__.invoke;
    const search=()=>invoke('clipboard_query',{query:'HELLO NAMI',favoritesOnly:false,kind:'files'});
    const before=await search();
    await invoke('clipboard_record_process',{recordId:'kind-image-file',action:'ocr'});
    const after=await search();
    window.__extractFile={done:true,before:before.page.totalRecords,ids:after.page.records.map(row=>row.id)};
  } catch(error) { window.__extractFile={done:true,error:String(error)}; }
})();
return true;
"#,
        )?;
        let extracted = wait_records(
            app,
            "return {ready:window.__extractFile?.done,...window.__extractFile};",
        )?;
        if extracted["before"] != 0 || extracted["ids"] != serde_json::json!(["kind-image-file"]) {
            return Err(format!(
                "file Translate must not index; explicit Extract text must succeed: {extracted}"
            ));
        }
        samples.push(extracted);
        std::fs::write(&file_image, &blank_png).map_err(|_| "fixture image update")?;
        // Leave and re-enter the clipboard view to refresh file eligibility without recognizing text.
        on_main(app, |app| {
            app.state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .set_record_view(crate::model::RecordView::Translation);
            apply_translation(app)
        })?;
        wait_records(
            app,
            "return {ready:!!document.querySelector('.clipboard-panel')?.closest('[hidden]')};",
        )?;
        records_mode(app, "clipboard")?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"window.__fileOcr={done:false};(async()=>{try{await window.__TAURI_INTERNALS__.invoke('clipboard_record_process',{recordId:'kind-image-file',action:'ocr'});window.__fileOcr={done:true};}catch(error){window.__fileOcr={done:true,error:String(error)};}})();return true;"#,
        )?;
        let updated = wait_records(
            app,
            "return {ready:window.__fileOcr?.done,...window.__fileOcr};",
        )?;
        if updated["error"] != "ocr_no_text" {
            return Err(format!(
                "mutable image file OCR used stale result: {updated}"
            ));
        }
        samples.push(updated);
        samples.push(wait_records(app, r#"const row=document.querySelector('.clipboard-panel [data-record-id="kind-image-file"]');return {ready:!!row&&!!row.querySelector('[aria-label="Translate"]:not([aria-hidden=true])')&&!!row.querySelector('[aria-label="Extract text"]:not([aria-hidden=true])')};"#)?);
        let handle = app.clone();
        let (locked_send, locked_receive) = mpsc::channel();
        let (release_send, release_receive) = tokio::sync::oneshot::channel();
        tauri::async_runtime::spawn(async move {
            let state = handle.state::<AppState>();
            let _lock = state.image_text.lock().await;
            let _ = locked_send.send(());
            let _ = release_receive.await;
        });
        locked_receive
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "OCR worker lock timeout")?;
        on_main(app, |app| {
            app.state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .records_browsing = true;
            emit_state(app)
        })?;
        let handle = app.clone();
        let (finished_send, finished_receive) = mpsc::channel();
        tauri::async_runtime::spawn(async move {
            let result =
                crate::clipboard_text::process(&handle, "kind-image-text".into(), "ocr".into())
                    .await;
            let _ = finished_send.send(result);
        });
        wait_records(app, "return {ready:true};")?;
        for _ in 0..100 {
            if !app
                .state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .records_browsing
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if app
            .state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .records_browsing
        {
            return Err("OCR processing did not protect its result".into());
        }
        evaluate_view_dom(
            app,
            "clipboard",
            r#"window.__ocrBrowse=false;window.__TAURI_INTERNALS__.invoke('clipboard_details_select',{recordId:'kind-text-ordinary',browsing:true}).then(()=>window.__ocrBrowse=true);return true;"#,
        )?;
        wait_records(app, "return {ready:window.__ocrBrowse};")?;
        let _ = release_send.send(());
        finished_receive
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "background OCR timeout")??;
        if app
            .state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .clipboard_detail_id
            .as_deref()
            != Some("kind-text-ordinary")
        {
            return Err("background OCR replaced explicit clipboard navigation".into());
        }
        let saved = app
            .state::<AppState>()
            .database
            .lock()
            .unwrap()
            .as_ref()
            .ok_or("test database")?
            .clipboard_page("HELLO NAMI", false, None, 100)?;
        if saved.total_records != 3
            || !saved
                .records
                .iter()
                .any(|record| record.kind == "text" && record.text.contains("HELLO NAMI"))
            || !saved
                .records
                .iter()
                .any(|record| record.id == "kind-image-text" && record.text == "Needle image")
            || !saved
                .records
                .iter()
                .any(|record| record.id == "kind-image-file")
        {
            return Err("explicit OCR must find both originals without duplicating the extracted text record".into());
        }
        samples.push(serde_json::json!({"backgroundOcrSaved":true,"browsedRecordRetained":true,"originalImageAndFileSearchable":true,"extractedTextDeduplicated":true}));
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
const row=document.querySelector('.clipboard-panel [data-record-id="kind-image-text"]');
row?.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:100,clientY:100}));
return true;
"#,
        )?;
        samples.push(wait_records(app, r#"
const panel=document.querySelector('.record-context-menu');if(!panel)return {ready:false};
const nodes=[...panel.querySelectorAll('[role=menuitem],[role=separator]')];
const width=panel.getBoundingClientRect().width;
const menu=nodes.map(el=>el.getAttribute('role')==='separator'?'/':`${el.querySelector('.record-menu-label')?.textContent.trim()} ${el.querySelector('kbd')?.textContent}`);
const expected=['Copy ⇧⌘C','Paste ↵','Save ⌘S','/','Favorite ⌥F','Pin ⌥P','Note ⇧⌘N','/','Translate ⌘T','Extract Text ⇧⌘O','/','Delete ⌘⌫'];
const pinned=document.querySelector('.clipboard-panel .history-pinned');
if(!pinned)return {ready:false};
const empty=pinned.classList.contains('empty');pinned.classList.remove('empty');
const style=getComputedStyle(pinned,'::after'),gradient=style.backgroundImage,height=style.height;
pinned.classList.toggle('empty',empty);
const separators=nodes.filter(el=>el.getAttribute('role')==='separator').map(el=>{const css=getComputedStyle(el);return [css.backgroundImage,css.height]});
return {ready:width===220&&JSON.stringify(menu)===JSON.stringify(expected)&&height==='1px'&&gradient.startsWith('linear-gradient(to right,')&&separators.length===3&&separators.every(([background,size])=>background===gradient&&size===height),imageMenu:menu,menuWidth:width,dividerGradient:gradient,separators};
"#)?);
        // With no external target, Paste can write only the isolated named pasteboard.
        *app.state::<AppState>().target.lock().unwrap() = None;
        let before_paste = on_main(app, |_| Ok(crate::platform::clipboard_version()))?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
window.__imageMenuKey=null;
window.addEventListener('keydown',e=>{window.__imageMenuKey=[e.code,e.key,e.metaKey,e.shiftKey];},{capture:true,once:true});
return true;
"#,
        )?;
        post_records_key(app, 36, 0, "\r", "\r", false)?;
        samples.push(wait_records(app, r#"
const message=document.querySelector('.clipboard-panel .record-feedback')?.textContent;
const menuOpen=!!document.querySelector('.record-context-menu');
return {ready:message==='Content copied. Press ⌘V to paste manually.'&&!menuOpen,message,menuOpen,key:window.__imageMenuKey,pasteMenuShortcutWithoutExternalTarget:true};
"#)?);
        if on_main(app, |_| Ok(crate::platform::clipboard_version()))? == before_paste {
            return Err("menu Paste did not write the named pasteboard".into());
        }
        records_mode(app, "clipboard")?;
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
const row=document.querySelector('.clipboard-panel [data-record-id="kind-image-text"]');
row?.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:100,clientY:100}));
return true;
"#,
        )?;
        wait_records(app, "return {ready:document.hasFocus()&&!!document.activeElement?.closest('.record-context-menu-keys')};")?;
        let before_save = on_main(app, |_| Ok(crate::platform::clipboard_version()))?;
        post_records_key(app, 1, 1 << 20, "s", "s", false)?;
        let mut cancelled = false;
        for _ in 0..120 {
            cancelled = on_main(app, |app| unsafe {
                use objc2::{class, msg_send, runtime::AnyObject};
                let parent = app.get_window("translation").ok_or("translation window")?;
                let native = parent
                    .ns_window()
                    .map_err(|_| "save sheet parent")?
                    .cast::<AnyObject>();
                let sheet: *mut AnyObject = msg_send![native, attachedSheet];
                if sheet.is_null() {
                    return Ok(false);
                }
                let is_save: bool = msg_send![sheet, isKindOfClass: class!(NSSavePanel)];
                let visible: bool = msg_send![native, isVisible];
                if !is_save || !visible {
                    return Err("system save panel is not attached to visible records".into());
                }
                let name: objc2::rc::Retained<objc2_foundation::NSString> =
                    msg_send![sheet, nameFieldStringValue];
                let _: () = msg_send![sheet, cancel: std::ptr::null::<AnyObject>()];
                let name = name.to_string();
                let stem = name.strip_suffix(".png").unwrap_or(&name);
                let timestamp = stem
                    .strip_prefix("Nami_")
                    .ok_or("missing Nami file prefix")?;
                let format = "%Y-%m-%d_%H-%M-%S";
                let time = chrono::NaiveDateTime::parse_from_str(timestamp, format)
                    .map_err(|_| "invalid save filename timestamp")?;
                if time.format(format).to_string() != timestamp
                    || (chrono::Local::now().naive_local() - time)
                        .num_seconds()
                        .abs()
                        > 15
                {
                    return Err("save filename does not use the local time when opened".into());
                }
                Ok(true)
            })?;
            if cancelled {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if !cancelled || on_main(app, |_| Ok(crate::platform::clipboard_version()))? != before_save
        {
            return Err(
                "image save did not present a cancellable native sheet or changed the clipboard"
                    .into(),
            );
        }
        samples.push(wait_records(app, r#"
const p=document.querySelector('.clipboard-panel');
return {ready:!!p?.getClientRects().length&&!p.querySelector('.record-feedback'),saveCancelledWithoutFeedback:true,defaultSaveNameMatchesLocalTime:true};
"#)?);
        evaluate_view_dom(
            app,
            "clipboard",
            r#"
window.__removedOcrEdit=null;
window.__TAURI_INTERNALS__.invoke('clipboard_record_update',{recordId:'kind-image-text',action:'correct_ocr',text:'synthetic replacement'}).then(()=>window.__removedOcrEdit=false,()=>window.__removedOcrEdit=true);
return true;
"#,
        )?;
        samples.push(wait_records(app, "return {ready:window.__removedOcrEdit===true,removedOcrEditRejected:window.__removedOcrEdit};")?);
        let calls = crate::clipboard_text::VISION_CALLS.load(Ordering::SeqCst);
        tauri::async_runtime::block_on(crate::clipboard_text::process(
            app,
            "kind-image-text".into(),
            "ocr".into(),
        ))?;
        let captured = on_main(app, |_| Ok(crate::platform::read_clipboard(|_| false)?))?;
        if !captured.is_some_and(|capture| {
            capture.payload == crate::platform::ClipboardPayload::Text("HELLO NAMI".into())
        }) {
            return Err("Extract text did not use the cached OCR text".into());
        }
        records_mode(app, "clipboard")?;
        wait_records(
            app,
            r#"
const action=document.querySelector('.clipboard-panel [data-record-id="kind-image-text"] [aria-label="Translate"]:not(:disabled):not([aria-hidden=true])');
action?.click(); return {ready:!!action};
"#,
        )?;
        samples.push(wait_dom(
            app,
            r#"
const source=document.querySelector('.source-input')?.value;
const dispatched=document.querySelector('.translation-status [role=alert]')?.textContent==='translation_network_disabled_in_tests';
return {ready:source==='HELLO NAMI'&&dispatched,cachedTranslationSource:source,translationAttempted:dispatched};
"#,
        )?);
        if crate::clipboard_text::VISION_CALLS.load(Ordering::SeqCst) != calls {
            return Err("copying or translating cached text reran Vision".into());
        }
        samples.push(serde_json::json!({"extractedIndexedText":true,"additionalVisionCalls":0}));
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
                    title: String::new(),
                    id: format!("pagination-{index}"),
                    text: format!("Synthetic page token-{index:04}"),
                    source_language: "en".into(),
                    translation: None,
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
    let identity = app.config().identifier.as_str();
    let fresh_denial = identity == "dev.local.nami.ui-tests.denial"
        && std::env::var("NAMI_UI_TEST_CASE").as_deref() == Ok("cross-app")
        && std::env::var("NAMI_UI_TEST_CROSS_APP").as_deref() == Ok("denied-fresh");
    if identity != "dev.local.nami.ui-tests" && !fresh_denial {
        return Err("native UI tests require an isolated test application identity".into());
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
            "clipboard-shortcuts",
            "clipboard-translate-shortcut",
            "clipboard-menu-paste",
            "clipboard-quick-paste",
            "record-display-shortcuts",
            "clipboard-settings",
            "clipboard-editor",
            "clipboard-media",
            "clipboard-kind-text",
            "clipboard-image-index",
            "clipboard-capture-burst",
            "clipboard-details",
            "translation-actions",
            "translation-controls",
            "translation-partial",
            "translation-settings",
            "unified-records",
            "records-geometry",
            "records-pinned",
            "records-pagination",
            "records-open",
            "selection-open",
            "cross-app",
        ]
        .contains(&case.as_str())
        {
            return Err("unknown native test case".into());
        }
        let app = app.clone();
        std::thread::spawn(move || {
            let result = if case == "selection-open" {
                run_selection_open(&app)
            } else if case == "cross-app" {
                run_cross_app(&app)
            } else if case == "records-open" {
                run_records_open(&app)
            } else if case == "clipboard-quick-paste" {
                run_clipboard_quick_paste(&app)
            } else if case == "clipboard-capture-burst" {
                run_clipboard_capture_burst(&app)
            } else if case == "clipboard-image-index" {
                run_clipboard_image_index(&app)
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
            } else if case == "translation-controls" {
                run_right_control_dom(&app)
            } else if case == "translation-partial" {
                run_translation_partial(&app)
            } else if case == "translation-settings" {
                run_translation_settings(&app)
            } else if case == "translation-actions" {
                run_translation_action_timing(&app)
            } else if case == "clipboard-settings" {
                run_clipboard_capture_setting(&app)
            } else if case == "record-display-shortcuts" {
                run_record_display_shortcuts(&app)
            } else if case == "clipboard-translate-shortcut" {
                run_clipboard_translate_shortcut(&app)
            } else if case == "clipboard-shortcuts" {
                run_clipboard_shortcuts_smoke(&app)
            } else if case == "clipboard-menu-paste" {
                run_clipboard_menu_paste(&app)
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
        dom_cases.push(run_translation_settings(&app));
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
