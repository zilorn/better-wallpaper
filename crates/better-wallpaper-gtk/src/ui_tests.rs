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

fn scene_properties() -> Value {
    json!([
        {"key":"speed","text":"Animation speed","prop_type":{"kind":"slider","min":0,"max":10,"step":0.25},"value":1,"condition":"enabled.value == true"},
        {"key":"enabled","text":"Enable animation","prop_type":{"kind":"bool"},"value":false},
        {"key":"mode","text":"Mode","prop_type":{"kind":"combo","options":[["Slow", "slow"],["Fast", "fast"]]},"value":"slow"},
        {"key":"tint","text":"Tint","prop_type":{"kind":"color"},"value":"1 0 0"},
        {"key":"texture","text":"Texture","prop_type":{"kind":"file"},"value":"textures/original.png","condition":"mode.value == 'fast'"},
        {"key":"caption","text":"Caption","prop_type":{"kind":"text_input"},"value":"Hello"},
        {"key":"future/key~","text":"Future property","prop_type":{"kind":"future"},"value":"keep"}
    ])
}

fn property_input(ui: &Desktop, key: &str) -> PropertyInput {
    let pointer = format!(
        "/scene/properties/{}",
        key.replace('~', "~0").replace('/', "~1")
    );
    ui.property_fields
        .borrow()
        .iter()
        .find_map(|field| {
            if field.pointer == pointer
                && let Input::Property(input) = &field.input
            {
                return Some(input.clone());
            }
            None
        })
        .unwrap()
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
    let thumbnail_requests = Arc::new(Mutex::new(Vec::new()));
    let hold_thumbnails = Arc::new(AtomicBool::new(false));
    let config_server = config.clone();
    let paused_server = paused.clone();
    let offline_server = offline.clone();
    let shutdown_server = shutdown.clone();
    let thumbnails_server = thumbnail_requests.clone();
    let hold_server = hold_thumbnails.clone();
    let worker = thread::spawn(move || {
        let mut delayed_thumbnails = Vec::new();
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
                    json!({"entries":(0..60).map(|i| json!({"name":format!("Wallpaper {:02}", i+1),"path":format!("/wallpapers/{i}.mp4"),"wallpaper_type":if i == 0 {"scene"} else {"video"},"engine_mode":i==0,"size_bytes":3145728,"scene_compatibility":if i==0 {json!({"level":2,"supported_features":["2D layers"],"unsupported_features":["Particles"],"warnings":["Verify on the selected backend"]})} else {Value::Null},"scene_properties":if i==0 {scene_properties()} else {json!([])}})).collect::<Vec<_>>(),"roots":["~/Videos/Wallpapers"],"engine_roots":[],"truncated":false})
                }
                ("GET", "/api/v1/logs") => {
                    json!({"lines":["INFO Wallpaper service started", "DEBUG Playback ready"]})
                }
                (_, path) if path.starts_with("/api/v1/library/thumbnail?") => {
                    thumbnails_server.lock().unwrap().push(path.to_owned());
                    if hold_server.load(Ordering::Relaxed) {
                        let hold = hold_server.clone();
                        delayed_thumbnails.push(thread::spawn(move || {
                            while hold.load(Ordering::Relaxed) {
                                thread::sleep(Duration::from_millis(10));
                            }
                            request
                                .respond(tiny_http::Response::from_data(
                                    include_bytes!(
                                        "../../../assets/icons/better-wallpaper-256.png"
                                    )
                                    .to_vec(),
                                ))
                                .unwrap();
                        }));
                        continue;
                    }
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
        for worker in delayed_thumbnails {
            worker.join().unwrap();
        }
    });
    let app = gtk::Application::builder()
        .application_id("org.betterwallpaper.GtkTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let ui = Desktop::open(&app, Some(endpoint));
    pump_until("connection and library", || {
        ui.connected.get() && !ui.busy.get() && ui.library.borrow().entries.len() == 60
    });
    assert!(!ui.dirty());
    assert_eq!(
        ui.library_count.text(),
        localized!("Showing 12 of 60 wallpapers", "已显示 12 个壁纸，共 60 个")
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
    pump_until("visible thumbnails", || {
        !thumbnail_requests.lock().unwrap().is_empty() && ui.thumbnails_active.get() == 0
    });
    // Offscreen cards and later batches must not fetch thumbnails on opening.
    assert_eq!(ui.library_loaded.get(), LIBRARY_BATCH_SIZE);
    assert!(thumbnail_requests.lock().unwrap().len() < LIBRARY_BATCH_SIZE);
    let adjustment = ui.library_scroll.vadjustment();
    for expected in [24, 36, 48, 60] {
        let old_upper = adjustment.upper();
        adjustment.set_value(old_upper - adjustment.page_size());
        pump_until("next scroll batch", || {
            ui.library_loaded.get() == expected && adjustment.upper() > old_upper
        });
    }
    adjustment.set_value(adjustment.upper() - adjustment.page_size());
    pump_until("last thumbnail", || {
        thumbnail_requests
            .lock()
            .unwrap()
            .iter()
            .any(|path| path.ends_with("59.mp4"))
            && ui.thumbnails_active.get() == 0
    });
    assert_eq!(ui.grid.observe_children().n_items(), 60);
    // Scrolling back reuses the existing cards and textures.
    let count = thumbnail_requests.lock().unwrap().len();
    adjustment.set_value(0.0);
    let start = Instant::now();
    pump_until("scroll back", || {
        start.elapsed() > Duration::from_millis(150)
    });
    assert_eq!(thumbnail_requests.lock().unwrap().len(), count);

    ui.filter.set_selected(3);
    assert_eq!(adjustment.value(), 0.0);
    assert_eq!(
        ui.library_count.text(),
        localized!("Showing 1 of 1 wallpapers", "已显示 1 个壁纸，共 1 个")
    );
    ui.search.set_text("missing");
    pump_until("search filter", || {
        ui.library_count
            .text()
            .starts_with(&localized!("Showing 0 of 0", "已显示 0 个壁纸，共 0 个"))
    });
    screenshot(&ui, "empty-search");
    ui.search.set_text("");
    ui.filter.set_selected(0);
    pump_until("search reset", || ui.library_matches.borrow().len() == 60);
    ui.search.set_text("Wallpaper 60");
    pump_until("search beyond loaded batch", || {
        ui.library_matches.borrow().len() == 1
    });
    assert_eq!(ui.library_matches.borrow()[0], 59);
    ui.search.set_text("");
    pump_until("search cleared", || ui.library_matches.borrow().len() == 60);
    assert_eq!(ui.library_loaded.get(), LIBRARY_BATCH_SIZE);
    ui.refresh_library();
    pump_until("library refreshed", || !ui.library_pending.get());
    assert_eq!(ui.library_loaded.get(), LIBRARY_BATCH_SIZE);

    // Refreshing off the page keeps cards available but defers all thumbnail I/O.
    ui.stack.set_visible_child_name("settings");
    pump_until("pending thumbnails finished", || {
        ui.thumbnails_active.get() == 0
    });
    let count = thumbnail_requests.lock().unwrap().len();
    ui.refresh_library();
    pump_until("hidden library refreshed", || !ui.library_pending.get());
    let start = Instant::now();
    pump_until("hidden library idle", || {
        start.elapsed() > Duration::from_millis(150)
    });
    assert_eq!(thumbnail_requests.lock().unwrap().len(), count);
    ui.stack.set_visible_child_name("library");
    pump_until("thumbnail loading resumed", || {
        thumbnail_requests.lock().unwrap().len() > count && ui.thumbnails_active.get() == 0
    });

    // Slow requests from the old filter still count toward the global limit,
    // and their late responses must never paint old or new search results.
    hold_thumbnails.store(true, Ordering::Relaxed);
    let count = thumbnail_requests.lock().unwrap().len();
    ui.render_library();
    let old_picture = ui.thumbnails.borrow()[0].picture.upgrade().unwrap();
    pump_until("four held thumbnails", || {
        thumbnail_requests.lock().unwrap().len() == count + THUMBNAIL_CONCURRENCY
    });
    ui.filter.set_selected(3);
    let new_picture = ui.thumbnails.borrow()[0].picture.upgrade().unwrap();
    let start = Instant::now();
    pump_until("filter while thumbnails are pending", || {
        start.elapsed() > Duration::from_millis(150)
    });
    assert_eq!(ui.thumbnails_active.get(), THUMBNAIL_CONCURRENCY);
    assert_eq!(
        thumbnail_requests.lock().unwrap().len(),
        count + THUMBNAIL_CONCURRENCY
    );
    hold_thumbnails.store(false, Ordering::Relaxed);
    pump_until("current filter thumbnail", || {
        ui.thumbnails_active.get() == 0 && new_picture.paintable().is_some()
    });
    assert!(old_picture.paintable().is_none());
    ui.filter.set_selected(0);

    ui.pause.emit_clicked();
    pump_until("pause request", || !ui.busy.get() && ui.paused.get());
    assert!(paused.load(Ordering::Relaxed));
    ui.pause.emit_clicked();
    pump_until("resume request", || !ui.busy.get() && !ui.paused.get());

    let scene = ui.library.borrow().entries[0].clone();
    let particle_limit = ui
        .fields
        .borrow()
        .iter()
        .find_map(|field| {
            if field.pointer == "/scene/particle_limit"
                && let Input::Number(spin) = &field.input
            {
                return Some(spin.clone());
            }
            None
        })
        .unwrap();
    assert_eq!(particle_limit.value(), 10_000.);
    particle_limit.set_value(0.);
    assert_eq!(ui.draft().unwrap()["scene"]["particle_limit"], 0);
    particle_limit.set_value(7.);
    assert_eq!(ui.draft().unwrap()["scene"]["particle_limit"], 7);
    ui.choose(&scene);
    assert!(ui.dirty());
    assert_eq!(ui.draft().unwrap()["wallpaper"]["engine_mode"], true);
    assert_eq!(ui.draft().unwrap()["scene"]["properties"], json!({}));
    assert!(!ui.property_rows.borrow()[0].0.is_visible());
    assert!(!ui.property_rows.borrow()[4].0.is_visible());
    let PropertyInput::Bool(enabled) = property_input(&ui, "enabled") else {
        panic!()
    };
    enabled.set_active(true);
    assert!(ui.property_rows.borrow()[0].0.is_visible());
    let PropertyInput::Number(speed) = property_input(&ui, "speed") else {
        panic!()
    };
    speed.set_value(2.5);
    let PropertyInput::Choice(mode, _) = property_input(&ui, "mode") else {
        panic!()
    };
    mode.set_selected(1);
    assert!(ui.property_rows.borrow()[4].0.is_visible());
    let PropertyInput::Color(tint) = property_input(&ui, "tint") else {
        panic!()
    };
    tint.set_rgba(&gdk::RGBA::new(0.25, 0.5, 1.0, 1.0));
    tint.emit_by_name::<()>("color-set", &[]);
    let PropertyInput::Text(texture) = property_input(&ui, "texture") else {
        panic!()
    };
    texture.set_text("textures/new.png");
    let PropertyInput::Text(caption) = property_input(&ui, "caption") else {
        panic!()
    };
    caption.set_text("New caption");
    let PropertyInput::Text(future) = property_input(&ui, "future/key~") else {
        panic!()
    };
    future.set_text("preserved");
    // Conditional hiding never removes unsaved edits or converts string values.
    enabled.set_active(false);
    mode.set_selected(0);
    assert!(!ui.property_rows.borrow()[0].0.is_visible());
    assert!(!ui.property_rows.borrow()[4].0.is_visible());
    assert_eq!(ui.draft().unwrap()["scene"]["properties"]["speed"], "2.5");
    ui.refresh_library();
    pump_until("property drafts survive refresh", || {
        !ui.library_pending.get()
    });
    assert_eq!(
        ui.draft().unwrap()["scene"]["properties"]["texture"],
        "textures/new.png"
    );
    enabled.set_active(true);
    mode.set_selected(1);
    ui.outputs.add("DP-1", true);
    // A concurrent tray mute update must survive this client's wallpaper save.
    config.lock().unwrap()["wallpaper"]["muted"] = json!(false);
    ui.save.emit_clicked();
    pump_until("configuration save", || {
        !ui.busy.get() && ui.baseline.borrow()["wallpaper"]["path"] == scene.path
    });
    assert!(!ui.dirty());
    assert_eq!(config.lock().unwrap()["wallpaper"]["muted"], false);
    assert_eq!(config.lock().unwrap()["scene"]["particle_limit"], 7);
    assert_eq!(
        config.lock().unwrap()["future_setting"],
        json!({"keep":true})
    );
    assert_eq!(
        config.lock().unwrap()["scene"]["properties"],
        json!({"speed":"2.5", "enabled":"true", "mode":"fast", "tint":"0.25 0.5 1", "texture":"textures/new.png", "caption":"New caption", "future/key~":"preserved"})
    );
    assert_eq!(
        config.lock().unwrap()["outputs"],
        json!([{"name":"DP-1","enabled":true}])
    );
    screenshot(&ui, "playback");
    let playback_scroll = ui
        .stack
        .child_by_name("wallpaper")
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    let adjustment = playback_scroll.vadjustment();
    adjustment.set_value(adjustment.upper() - adjustment.page_size());
    screenshot(&ui, "properties");

    ui.reset_property_overrides();
    assert_eq!(ui.draft().unwrap()["scene"]["properties"], json!({}));
    assert!(!ui.property_rows.borrow()[0].0.is_visible());
    assert!(!ui.property_rows.borrow()[4].0.is_visible());
    ui.save.emit_clicked();
    pump_until("property defaults saved", || !ui.busy.get());
    assert_eq!(config.lock().unwrap()["scene"]["properties"], json!({}));

    // Loading unusual existing overrides must never normalize them on save.
    let original = ui.baseline.borrow().clone();
    let mut unusual = original.clone();
    unusual["scene"]["properties"] = json!({"speed":"1.234567890123", "enabled":"future-bool", "tint":"future-color", "mode":"removed-option"});
    ui.library.borrow_mut().entries[0].scene_properties[5]["condition"] =
        json!("speed.value == '1.234567890123'");
    ui.load_config(unusual.clone());
    assert!(ui.property_rows.borrow()[5].0.is_visible());
    assert!(!ui.dirty());
    assert_eq!(ui.draft().unwrap(), unusual);
    assert!(matches!(
        property_input(&ui, "enabled"),
        PropertyInput::Text(_)
    ));
    assert!(matches!(
        property_input(&ui, "tint"),
        PropertyInput::Text(_)
    ));
    ui.library.borrow_mut().entries[0].scene_properties[5]["condition"] = Value::Null;
    ui.load_config(original);

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
