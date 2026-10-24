use super::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Instant,
};

fn pump_until(description: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    let context = glib::MainContext::default();
    while !done() {
        while context.pending() {
            context.iteration(false);
        }
        assert!(Instant::now() < deadline, "Timed out: {description}");
        thread::sleep(Duration::from_millis(10));
    }
    while context.pending() {
        context.iteration(false);
    }
}

fn screenshot(ui: &Desktop, name: &str) {
    let Ok(directory) = std::env::var("BETTER_WALLPAPER_GTK_SCREENSHOTS") else {
        return;
    };
    let start = Instant::now();
    pump_until("window layout", || {
        start.elapsed() > Duration::from_millis(350)
    });
    let paintable = gtk::WidgetPaintable::new(Some(&ui.window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        ui.window.width() as f64,
        ui.window.height() as f64,
    );
    let node = snapshot.to_node().expect("window has visible content");
    let texture = ui.window.renderer().unwrap().render_texture(&node, None);
    std::fs::create_dir_all(&directory).unwrap();
    texture
        .save_to_png(std::path::Path::new(&directory).join(format!("{name}.png")))
        .unwrap();
}

#[test]
#[ignore = "Requires GTK display and session bus; run under dbus-run-session and xvfb-run"]
fn native_window_connects_edits_saves_filters_and_recovers() {
    crate::i18n::init(Some(
        if std::env::var("BETTER_WALLPAPER_GTK_TEST_LANGUAGE").as_deref() == Ok("zh-CN") {
            crate::i18n::Language::Chinese
        } else {
            crate::i18n::Language::English
        },
    ));
    gtk::init().unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let endpoint =
        crate::client::validate_endpoint(&format!("http://{}", server.server_addr())).unwrap();
    let mut initial =
        serde_json::to_value(better_wallpaper_core::config::AppConfig::default()).unwrap();
    initial["future_setting"] = json!({"keep":true});
    initial["scene"]["properties"] = json!({"old":"discard when switching scenes"});
    let config = Arc::new(Mutex::new(initial));
    let paused = Arc::new(AtomicBool::new(false));
    let offline = Arc::new(AtomicBool::new(false));
    let shutdown = Arc::new(AtomicBool::new(false));
    let config_server = config.clone();
    let paused_server = paused.clone();
    let offline_server = offline.clone();
    let shutdown_server = shutdown.clone();
    let worker = thread::spawn(move || {
        while !shutdown_server.load(Ordering::Relaxed) {
            let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap() else {
                continue;
            };
            if offline_server.load(Ordering::Relaxed) {
                request
                    .respond(
                        tiny_http::Response::from_string("{\"error\":\"Service unavailable\"}")
                            .with_status_code(503),
                    )
                    .unwrap();
                continue;
            }
            let body = match (request.method().as_str(), request.url()) {
                ("GET", "/api/v1/status") => {
                    json!({"api_version":1,"desktop":"niri","backend":"niri","evidence":"Integration test session","candidates":["niri"],"playback":{"running":true,"paused":paused_server.load(Ordering::Relaxed),"cancelled":false},"plasma_instances":[]})
                }
                ("GET", "/api/v1/config") => config_server.lock().unwrap().clone(),
                ("PUT", "/api/v1/config") => {
                    let config: Value = serde_json::from_reader(request.as_reader()).unwrap();
                    *config_server.lock().unwrap() = config.clone();
                    json!({"saved":true,"restart_required":config["general"]["backend"] == "headless","reload_requested":true,"config":config})
                }
                ("POST", "/api/v1/playback/pause") => {
                    paused_server.store(true, Ordering::Relaxed);
                    json!({})
                }
                ("POST", "/api/v1/playback/resume") => {
                    paused_server.store(false, Ordering::Relaxed);
                    json!({})
                }
                ("GET", "/api/v1/library") => {
                    json!({"entries":(0..15).map(|i| json!({"name":format!("Wallpaper {:02}", i+1),"path":format!("/wallpapers/{i}.mp4"),"wallpaper_type":if i == 0 {"scene"} else {"video"},"engine_mode":i==0,"size_bytes":3145728,"scene_compatibility":if i==0 {json!({"level":2,"supported_features":["2D layers"],"unsupported_features":["Particles"],"warnings":["Verify on the selected backend"]})} else {Value::Null},"scene_properties":if i==0 {json!([{"key":"speed","text":"Animation speed","prop_type":{"kind":"slider","min":0,"max":10},"value":1}])} else {json!([])}})).collect::<Vec<_>>(),"roots":["~/Videos/Wallpapers"],"engine_roots":[],"truncated":false})
                }
                ("GET", "/api/v1/logs") => {
                    json!({"lines":["INFO Wallpaper service started", "DEBUG Playback ready"]})
                }
                (_, path) if path.starts_with("/api/v1/library/thumbnail?") => {
                    request
                        .respond(tiny_http::Response::from_data(
                            include_bytes!("../../../assets/icons/better-wallpaper-256.png")
                                .to_vec(),
                        ))
                        .unwrap();
                    continue;
                }
                _ => {
                    request
                        .respond(
                            tiny_http::Response::from_string("not found").with_status_code(404),
                        )
                        .unwrap();
                    continue;
                }
            };
            request
                .respond(tiny_http::Response::from_string(body.to_string()))
                .unwrap();
        }
    });
    let app = gtk::Application::builder()
        .application_id("org.betterwallpaper.GtkTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let ui = Desktop::open(&app, Some(endpoint));
    pump_until("connection and library", || {
        ui.connected.get() && !ui.busy.get() && ui.library.borrow().entries.len() == 15
    });
    assert!(!ui.dirty());
    assert_eq!(
        ui.pagination.text(),
        localized!(
            "15 wallpapers · Page 1 of 2",
            "15 个壁纸 · 第 1 页，共 2 页"
        )
    );
    assert_eq!(ui.save.label().as_deref(), Some(tr("Save changes")));
    assert_eq!(
        ui.stack
            .page(&ui.stack.child_by_name("library").unwrap())
            .title()
            .as_deref(),
        Some(tr("Wallpapers"))
    );
    screenshot(&ui, "library");
    ui.next.emit_clicked();
    assert_eq!(ui.page.get(), 1);
    ui.filter.set_selected(3);
    assert_eq!(
        ui.pagination.text(),
        localized!("1 wallpapers · Page 1 of 1", "1 个壁纸 · 第 1 页，共 1 页")
    );
    ui.search.set_text("missing");
    pump_until("search filter", || {
        ui.pagination
            .text()
            .starts_with(&localized!("0 wallpapers", "0 个壁纸"))
    });
    screenshot(&ui, "empty-search");
    ui.search.set_text("");
    ui.filter.set_selected(0);

    ui.pause.emit_clicked();
    pump_until("pause request", || !ui.busy.get() && ui.paused.get());
    assert!(paused.load(Ordering::Relaxed));
    ui.pause.emit_clicked();
    pump_until("resume request", || !ui.busy.get() && !ui.paused.get());

    let scene = ui.library.borrow().entries[0].clone();
    ui.choose(&scene);
    assert!(ui.dirty());
    assert_eq!(ui.draft().unwrap()["wallpaper"]["engine_mode"], true);
    assert_eq!(ui.draft().unwrap()["scene"]["properties"], json!({}));
    let Input::Text(speed) = ui.property_fields.borrow()[0].input.clone() else {
        panic!()
    };
    speed.set_text("2");
    ui.outputs.add("DP-1", true);
    // A concurrent tray mute update must survive this client's wallpaper save.
    config.lock().unwrap()["wallpaper"]["muted"] = json!(false);
    ui.save.emit_clicked();
    pump_until("configuration save", || {
        !ui.busy.get() && ui.baseline.borrow()["wallpaper"]["path"] == scene.path
    });
    assert!(!ui.dirty());
    assert_eq!(config.lock().unwrap()["wallpaper"]["muted"], false);
    assert_eq!(
        config.lock().unwrap()["future_setting"],
        json!({"keep":true})
    );
    assert_eq!(
        config.lock().unwrap()["scene"]["properties"],
        json!({"speed":"2"})
    );
    assert_eq!(
        config.lock().unwrap()["outputs"],
        json!([{"name":"DP-1","enabled":true}])
    );
    screenshot(&ui, "playback");

    let backend = ui
        .fields
        .borrow()
        .iter()
        .find_map(|field| {
            if field.pointer == "/general/backend" {
                if let Input::Choice(choice, _) = &field.input {
                    Some(choice.clone())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .unwrap();
    backend.set_selected(3);
    ui.save.emit_clicked();
    pump_until("backend save", || !ui.busy.get());
    assert_eq!(
        ui.notice.text(),
        tr("Settings saved. Restart the Better Wallpaper service to switch the running backend.")
    );
    ui.stack.set_visible_child_name("settings");
    screenshot(&ui, "settings");
    ui.stack.set_visible_child_name("logs");
    pump_until("log messages", || {
        buffer_text(&ui.logs).contains("Playback ready")
    });
    ui.log_search.set_text("DEBUG");
    pump_until("filtered logs", || !buffer_text(&ui.logs).contains("INFO"));

    // Same-field edits in another client are reported, retaining the local draft.
    backend.set_selected(2);
    config.lock().unwrap()["general"]["backend"] = json!("niri");
    ui.save.emit_clicked();
    pump_until("conflicting save", || !ui.busy.get());
    assert!(ui.notice.text().contains(&localized!(
        "changed in another client",
        "已在其他客户端中更改"
    )));
    assert_eq!(config.lock().unwrap()["general"]["backend"], "niri");
    offline.store(true, Ordering::Relaxed);
    ui.poll();
    pump_until("offline state", || !ui.connected.get());
    assert!(!ui.save.is_sensitive());
    assert!(ui.dirty());
    screenshot(&ui, "offline");
    offline.store(false, Ordering::Relaxed);
    ui.poll();
    pump_until("automatic recovery", || ui.connected.get());
    assert!(ui.dirty());
    let weak = Rc::downgrade(&ui);
    ui.window.destroy();
    drop(ui);
    pump_until("controller released", || weak.upgrade().is_none());
    shutdown.store(true, Ordering::Relaxed);
    worker.join().unwrap();
}
