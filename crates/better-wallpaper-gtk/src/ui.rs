use crate::i18n::{localized, tr};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

use anyhow::{Result, bail};
use gtk::{gdk, gio, glib, prelude::*};
use reqwest::Url;
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::client::{Api, Entry, Library, Status, select_wallpaper};
use crate::properties::{PropertyInput, property_value, property_visible};

const LIBRARY_BATCH_SIZE: usize = 12;
const THUMBNAIL_CONCURRENCY: usize = 4;
const LIBRARY_PREFETCH: f64 = 300.0;

struct Thumbnail {
    path: String,
    picture: glib::WeakRef<gtk::Picture>,
    fallback: glib::WeakRef<gtk::Image>,
}

pub struct Desktop {
    window: gtk::ApplicationWindow,
    explicit_url: Option<Url>,
    api: RefCell<Option<Api>>,
    baseline: RefCell<Value>,
    pending_selection: RefCell<Option<Entry>>,
    reset_properties: Cell<bool>,
    fields: RefCell<Vec<Field>>,
    outputs: Outputs,
    properties: gtk::Box,
    property_fields: RefCell<Vec<Field>>,
    property_rows: RefCell<Vec<(gtk::Box, Option<String>)>>,
    stack: gtk::Stack,
    content: gtk::Box,
    notice: gtk::Label,
    status: gtk::Label,
    diagnostics: gtk::Label,
    spinner: gtk::Spinner,
    save: gtk::Button,
    reload: gtk::Button,
    pause: gtk::Button,
    paused: Cell<bool>,
    busy: Cell<bool>,
    polling: Cell<bool>,
    logs_pending: Cell<bool>,
    library_pending: Cell<bool>,
    connected: Cell<bool>,
    library: RefCell<Library>,
    grid: gtk::FlowBox,
    search: gtk::SearchEntry,
    filter: gtk::DropDown,
    library_scroll: gtk::ScrolledWindow,
    library_matches: RefCell<Vec<usize>>,
    library_loaded: Cell<usize>,
    library_update_pending: Cell<bool>,
    thumbnails: RefCell<Vec<Thumbnail>>,
    thumbnails_active: Cell<usize>,
    generation: Cell<u64>,
    library_info: gtk::Label,
    library_count: gtk::Label,
    logs: gtk::TextView,
    follow_logs: gtk::CheckButton,
    log_search: gtk::SearchEntry,
    log_text: RefCell<String>,
}

#[derive(Clone)]
enum Input {
    Text(gtk::Entry),
    Bool(gtk::Switch),
    Number(gtk::SpinButton),
    Choice(gtk::DropDown, Vec<String>),
    Paths(gtk::TextView),
    Property(PropertyInput),
}

struct Field {
    pointer: String,
    input: Input,
    default: Value,
    initial: RefCell<Value>,
}

impl Field {
    fn read(&self) -> Value {
        match &self.input {
            Input::Property(input) => json!(input.read()),
            Input::Text(entry) => json!(entry.text().as_str()),
            Input::Bool(toggle) => json!(toggle.is_active()),
            Input::Number(spin) => json!(spin.value_as_int()),
            Input::Choice(choice, values) => json!(
                values
                    .get(choice.selected() as usize)
                    .cloned()
                    .unwrap_or_default()
            ),
            Input::Paths(view) => json!(
                buffer_text(view)
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            ),
        }
    }

    fn load(&self, config: &Value) {
        let value = config
            .pointer(&self.pointer)
            .filter(|value| !value.is_null())
            .unwrap_or(&self.default);
        match &self.input {
            // Property editors are constructed with the current string value.
            Input::Property(_) => {}
            Input::Text(entry) => entry.set_text(value.as_str().unwrap_or_default()),
            Input::Bool(toggle) => toggle.set_active(value.as_bool().unwrap_or(false)),
            Input::Number(spin) => spin.set_value(value.as_f64().unwrap_or(0.0)),
            Input::Choice(choice, values) => choice.set_selected(
                values
                    .iter()
                    .position(|s| Some(s.as_str()) == value.as_str())
                    .unwrap_or(0) as u32,
            ),
            Input::Paths(view) => view.buffer().set_text(
                &value
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default(),
            ),
        }
        *self.initial.borrow_mut() = self.read();
    }

    fn changed(&self) -> bool {
        self.read() != *self.initial.borrow()
    }

    fn apply(&self, draft: &mut Value) {
        if !self.changed() {
            return;
        }
        let mut value = self.read();
        if self.pointer == "/wallpaper/path" {
            if value == "" {
                value = Value::Null;
            }
            draft["wallpaper"]["engine_mode"] = json!(false);
        }
        set_pointer(draft, &self.pointer, value);
    }
}

fn set_pointer(value: &mut Value, pointer: &str, new: Value) {
    let parts: Vec<_> = pointer
        .trim_start_matches('/')
        .split('/')
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect();
    let mut cursor = value;
    for part in &parts[..parts.len() - 1] {
        if !cursor.is_object() {
            *cursor = json!({});
        }
        cursor = &mut cursor[part];
    }
    if !cursor.is_object() {
        *cursor = json!({});
    }
    cursor[parts.last().unwrap()] = new;
}

#[derive(Clone)]
struct Outputs {
    list: gtk::Box,
    rows: Rc<RefCell<Vec<(gtk::Entry, gtk::Switch, gtk::Box)>>>,
    initial: Rc<RefCell<Value>>,
}

impl Outputs {
    fn new() -> Self {
        Self {
            list: column(8),
            rows: Rc::default(),
            initial: Rc::new(RefCell::new(json!([]))),
        }
    }

    fn add(&self, name: &str, enabled: bool) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let entry = gtk::Entry::builder()
            .text(name)
            .placeholder_text(tr("Output name, e.g. DP-1"))
            .hexpand(true)
            .build();
        let toggle = gtk::Switch::builder()
            .active(enabled)
            .valign(gtk::Align::Center)
            .build();
        toggle.set_tooltip_text(Some(tr("Enable this output")));
        let remove = gtk::Button::from_icon_name("list-remove-symbolic");
        remove.set_tooltip_text(Some(tr("Remove output")));
        row.append(&entry);
        row.append(&toggle);
        row.append(&remove);
        self.list.append(&row);
        self.rows.borrow_mut().push((entry, toggle, row.clone()));
        let rows = Rc::downgrade(&self.rows);
        let list = self.list.downgrade();
        let weak_row = row.downgrade();
        remove.connect_clicked(move |_| {
            if let (Some(list), Some(row), Some(rows)) =
                (list.upgrade(), weak_row.upgrade(), rows.upgrade())
            {
                rows.borrow_mut().retain(|(_, _, r)| *r != row);
                list.remove(&row);
            }
        });
    }

    fn value(&self) -> Value {
        json!(self.rows.borrow().iter().map(|(entry, toggle, _)| json!({"name":entry.text().trim(),"enabled":toggle.is_active()})).collect::<Vec<_>>())
    }

    fn load(&self, config: &Value) {
        clear_box(&self.list);
        self.rows.borrow_mut().clear();
        if let Some(outputs) = config["outputs"].as_array() {
            for output in outputs {
                self.add(
                    output["name"].as_str().unwrap_or_default(),
                    output["enabled"] == true,
                );
            }
        }
        *self.initial.borrow_mut() = self.value();
    }
}

impl Desktop {
    pub fn open(app: &gtk::Application, explicit_url: Option<Url>) -> Rc<Self> {
        let css = gtk::CssProvider::new();
        css.load_from_data(".title-1 { font-size: 26px; font-weight: 700; } .title-3 { font-size: 17px; font-weight: 600; } .card { border: 1px solid alpha(currentColor, 0.15); border-radius: 12px; background: alpha(currentColor, 0.025); } .card.accent { border-color: @theme_selected_bg_color; } .error { color: @error_color; }");
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Better Wallpaper")
            .default_width(1120)
            .default_height(780)
            .build();
        window.set_size_request(760, 540);
        window.set_icon_name(Some("better-wallpaper"));
        let header = gtk::HeaderBar::new();
        let title = gtk::Label::new(Some("Better Wallpaper"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));
        let status = gtk::Label::new(Some(tr("Connecting…")));
        status.add_css_class("dim-label");
        let spinner = gtk::Spinner::new();
        header.pack_start(&spinner);
        header.pack_start(&status);
        let pause = gtk::Button::with_label(tr("Pause"));
        pause.set_sensitive(false);
        header.pack_end(&pause);
        window.set_titlebar(Some(&header));

        let root = column(0);
        let notice = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .selectable(true)
            .build();
        margins(&notice, 12);
        notice.set_visible(false);
        root.append(&notice);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.set_vexpand(true);
        let stack = gtk::Stack::builder()
            .hexpand(true)
            .vexpand(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        let sidebar = gtk::StackSidebar::builder()
            .stack(&stack)
            .width_request(170)
            .build();
        sidebar.add_css_class("navigation-sidebar");
        body.append(&sidebar);
        body.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        body.append(&stack);
        root.append(&body);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        margins(&footer, 12);
        let hint = gtk::Label::new(Some(tr(
            "Closing this window keeps your wallpaper running.",
        )));
        hint.set_hexpand(true);
        hint.set_xalign(0.0);
        hint.add_css_class("dim-label");
        let reload = gtk::Button::with_label(tr("Reconnect"));
        let save = gtk::Button::with_label(tr("Save changes"));
        save.add_css_class("suggested-action");
        save.set_sensitive(false);
        footer.append(&hint);
        footer.append(&reload);
        footer.append(&save);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&footer);
        window.set_child(Some(&root));

        let library_page = page(
            tr("Your wallpapers"),
            tr("Choose a wallpaper, then save to apply it to your desktop."),
        );
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let search = gtk::SearchEntry::builder()
            .placeholder_text(tr("Search wallpapers"))
            .hexpand(true)
            .build();
        let filter =
            gtk::DropDown::from_strings(&[tr("All types"), tr("Video"), tr("Web"), tr("Scene")]);
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some(tr("Refresh library")));
        toolbar.append(&search);
        toolbar.append(&filter);
        toolbar.append(&refresh);
        library_page.append(&toolbar);
        let library_info = wrapped_label("");
        library_info.add_css_class("dim-label");
        library_page.append(&library_info);
        let grid = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(1)
            .max_children_per_line(3)
            .row_spacing(14)
            .column_spacing(14)
            .build();
        library_page.append(&grid);
        let library_count = gtk::Label::new(None);
        library_page.append(&library_count);
        let library_scroll = add_page(&stack, &library_page, "library", tr("Wallpapers"));

        let wallpaper_page = page(
            tr("Wallpaper settings"),
            tr("Adjust playback and choose a local file or project."),
        );
        let outputs = Outputs::new();
        let display_page = page(
            tr("Displays"),
            tr(
                "Use the output names reported by your desktop. An empty list selects outputs automatically.",
            ),
        );
        display_page.append(&outputs.list);
        let add_output = gtk::Button::with_label(tr("Add output"));
        add_output.set_halign(gtk::Align::Start);
        display_page.append(&add_output);
        let folders_page = page(
            tr("Library folders"),
            tr("One folder per line. Steam Workshop libraries are discovered automatically."),
        );
        let settings_page = page(
            tr("Settings & diagnostics"),
            tr("Backend changes take effect after restarting the wallpaper service."),
        );
        let diagnostics = wrapped_label("");
        diagnostics.set_selectable(true);
        let properties = column(10);
        let logs_page = page(
            tr("Service logs"),
            tr("Recent messages from the wallpaper service."),
        );
        let follow_logs = gtk::CheckButton::with_label(tr("Refresh automatically"));
        follow_logs.set_active(true);
        let log_search = gtk::SearchEntry::builder()
            .placeholder_text(tr("Filter log messages"))
            .build();
        let logs = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .vexpand(true)
            .build();
        logs_page.append(&follow_logs);
        logs_page.append(&log_search);
        logs_page.append(&logs);
        let content = body;
        let ui = Rc::new(Self {
            window,
            explicit_url,
            api: RefCell::new(None),
            baseline: RefCell::new(Value::Null),
            pending_selection: RefCell::new(None),
            reset_properties: Cell::new(false),
            fields: RefCell::default(),
            outputs,
            properties,
            property_fields: RefCell::default(),
            property_rows: RefCell::default(),
            stack,
            content,
            notice,
            status,
            diagnostics,
            spinner,
            save,
            reload,
            pause,
            paused: Cell::new(false),
            busy: Cell::new(false),
            polling: Cell::new(false),
            logs_pending: Cell::new(false),
            library_pending: Cell::new(false),
            connected: Cell::new(false),
            library: RefCell::default(),
            grid,
            search,
            filter,
            library_scroll,
            library_matches: RefCell::default(),
            library_loaded: Cell::new(0),
            library_update_pending: Cell::new(false),
            thumbnails: RefCell::default(),
            thumbnails_active: Cell::new(0),
            generation: Cell::new(0),
            library_info,
            library_count,
            logs,
            follow_logs,
            log_search,
            log_text: RefCell::default(),
        });
        ui.form(&wallpaper_page, &folders_page, &settings_page);
        wallpaper_page.append(&ui.properties);
        settings_page.append(&section(tr("Connection details")));
        settings_page.append(&ui.diagnostics);
        add_page(&ui.stack, &wallpaper_page, "wallpaper", tr("Playback"));
        add_page(&ui.stack, &display_page, "displays", tr("Displays"));
        add_page(&ui.stack, &folders_page, "folders", tr("Library folders"));
        add_page(&ui.stack, &settings_page, "settings", tr("Settings"));
        add_page(&ui.stack, &logs_page, "logs", tr("Logs"));
        ui.content.set_sensitive(false);

        let weak = Rc::downgrade(&ui);
        ui.save.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.save_config();
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.reload.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.reload_or_confirm();
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.pause.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.toggle_pause();
            }
        });
        let weak = Rc::downgrade(&ui);
        refresh.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.refresh_library();
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.search.connect_search_changed(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.render_library();
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.filter.connect_selected_notify(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.render_library();
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.library_scroll
            .vadjustment()
            .connect_value_changed(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.queue_library_update();
                }
            });
        let weak = Rc::downgrade(&ui);
        ui.library_scroll.vadjustment().connect_changed(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.queue_library_update();
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.grid.connect_map(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.queue_library_update();
            }
        });
        let weak = Rc::downgrade(&ui);
        add_output.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.outputs.add("", true);
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.log_search.connect_search_changed(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.render_logs();
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.stack.connect_visible_child_name_notify(move |_| {
            if let Some(ui) = weak.upgrade() {
                match ui.stack.visible_child_name().as_deref() {
                    Some("logs") => ui.refresh_logs(),
                    Some("library") => ui.queue_library_update(),
                    _ => {}
                }
            }
        });

        // The window owns the controller until destruction. Signals above use weak
        // references; closing it drops polling state and late request results.
        let retained = Rc::new(RefCell::new(Some(ui.clone())));
        ui.window.connect_unrealize(move |_| {
            retained.borrow_mut().take();
        });
        let weak = Rc::downgrade(&ui);
        glib::timeout_add_local(Duration::from_secs(2), move || {
            let Some(ui) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !ui.window.is_visible() {
                return glib::ControlFlow::Break;
            }
            ui.poll();
            glib::ControlFlow::Continue
        });
        ui.window.present();
        info!("GTK desktop management window started");
        ui.connect();
        ui
    }

    fn form(self: &Rc<Self>, wallpaper: &gtk::Box, folders: &gtk::Box, settings: &gtk::Box) {
        let path = gtk::Entry::builder()
            .placeholder_text(tr("Choose a wallpaper file…"))
            .hexpand(true)
            .build();
        let path_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let browse = gtk::Button::with_label(tr("Browse…"));
        path_row.append(&path);
        path_row.append(&browse);
        wallpaper.append(&wrapped_label(tr("Wallpaper path")));
        wallpaper.append(&path_row);
        self.register("/wallpaper/path", Input::Text(path.clone()), json!(""));
        let weak = Rc::downgrade(self);
        browse.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let chooser = gtk::FileChooserNative::new(
                    Some(tr("Choose wallpaper")),
                    Some(&ui.window),
                    gtk::FileChooserAction::Open,
                    Some(tr("Choose")),
                    Some(tr("Cancel")),
                );
                let entry = path.clone();
                chooser.connect_response(move |dialog, response| {
                    if response == gtk::ResponseType::Accept
                        && let Some(path) = dialog.file().and_then(|f| f.path())
                    {
                        entry.set_text(&path.to_string_lossy());
                    }
                    dialog.destroy();
                });
                chooser.show();
            }
        });
        self.choice(
            wallpaper,
            tr("Wallpaper type"),
            "/wallpaper/wallpaper_type",
            &["video", "web", "scene"],
            "video",
        );
        self.choice(
            wallpaper,
            tr("Fill mode"),
            "/wallpaper/fill_mode",
            &["cover", "contain", "stretch"],
            "cover",
        );
        self.number(
            wallpaper,
            tr("Frame rate limit"),
            "/wallpaper/fps_limit",
            1.0,
            240.0,
            60,
        );
        self.toggle(
            wallpaper,
            tr("Loop playback"),
            "/wallpaper/loop_playback",
            true,
        );
        self.toggle(wallpaper, tr("Mute sound"), "/wallpaper/muted", true);
        wallpaper.append(&section(tr("Scene settings")));
        wallpaper.append(&wrapped_label(tr("Scene support depends on the playback backend. Basic pointer parallax is available on exposed niri wallpaper surfaces. Particle limits and scripts are not implemented.")));
        self.choice(
            wallpaper,
            tr("Quality"),
            "/scene/quality",
            &["high", "medium", "low"],
            "high",
        );
        self.toggle(wallpaper, tr("Mouse input"), "/scene/mouse", true);
        self.toggle(wallpaper, tr("Basic parallax"), "/scene/parallax", true);
        self.toggle(
            wallpaper,
            tr("Audio response"),
            "/scene/audio_processing",
            false,
        );
        let reset = gtk::Button::with_label(tr("Reset wallpaper properties to defaults"));
        reset.set_halign(gtk::Align::Start);
        wallpaper.append(&reset);
        let weak = Rc::downgrade(self);
        reset.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.reset_property_overrides();
            }
        });

        let paths = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .height_request(140)
            .build();
        paths.add_css_class("view");
        folders.append(&paths);
        self.register("/library/paths", Input::Paths(paths.clone()), json!([]));
        let add_folder = gtk::Button::with_label(tr("Add folder…"));
        add_folder.set_halign(gtk::Align::Start);
        folders.append(&add_folder);
        let weak = Rc::downgrade(self);
        add_folder.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let chooser = gtk::FileChooserNative::new(
                    Some(tr("Add library folder")),
                    Some(&ui.window),
                    gtk::FileChooserAction::SelectFolder,
                    Some(tr("Add")),
                    Some(tr("Cancel")),
                );
                let paths = paths.clone();
                chooser.connect_response(move |dialog, response| {
                    if response == gtk::ResponseType::Accept
                        && let Some(path) = dialog.file().and_then(|f| f.path())
                    {
                        let old = buffer_text(&paths);
                        let path = path.to_string_lossy();
                        if !old.lines().any(|line| line == path) {
                            paths.buffer().set_text(&format!(
                                "{}{}{}",
                                old,
                                if old.is_empty() { "" } else { "\n" },
                                path
                            ));
                        }
                    }
                    dialog.destroy();
                });
                chooser.show();
            }
        });
        folders.append(&wrapped_label(tr("Save changes to update the indexed folders. Removing a folder here does not delete its files.")));
        self.choice(
            settings,
            tr("Backend"),
            "/general/backend",
            &["auto", "niri", "kde", "headless"],
            "auto",
        );
        self.choice(
            settings,
            tr("Hardware decoding"),
            "/decode/hardware",
            &["auto", "software"],
            "auto",
        );
        self.number(
            settings,
            tr("Maximum decode height (0 = original)"),
            "/decode/max_height",
            0.0,
            16384.0,
            0,
        );
        self.choice(
            settings,
            tr("Log level"),
            "/general/log_level",
            &["info", "debug"],
            "info",
        );
        self.toggle(
            settings,
            tr("Restore playback on startup"),
            "/general/restore_on_start",
            true,
        );
    }

    fn register(&self, pointer: &str, input: Input, default: Value) {
        self.fields.borrow_mut().push(Field {
            pointer: pointer.into(),
            input,
            default,
            initial: RefCell::new(Value::Null),
        });
    }

    fn choice(&self, page: &gtk::Box, title: &str, pointer: &str, values: &[&str], default: &str) {
        let labels: Vec<_> = values
            .iter()
            .map(|value| crate::i18n::option_label(value))
            .collect();
        let choice = gtk::DropDown::from_strings(&labels);
        row(page, title, &choice);
        self.register(
            pointer,
            Input::Choice(choice, values.iter().map(|s| s.to_string()).collect()),
            json!(default),
        );
    }

    fn toggle(&self, page: &gtk::Box, title: &str, pointer: &str, default: bool) {
        let toggle = gtk::Switch::builder().valign(gtk::Align::Center).build();
        row(page, title, &toggle);
        self.register(pointer, Input::Bool(toggle), json!(default));
    }

    fn number(
        &self,
        page: &gtk::Box,
        title: &str,
        pointer: &str,
        min: f64,
        max: f64,
        default: u32,
    ) {
        let spin = gtk::SpinButton::with_range(min, max, 1.0);
        row(page, title, &spin);
        self.register(pointer, Input::Number(spin), json!(default));
    }

    fn draft(&self) -> Result<Value> {
        let mut draft = self.baseline.borrow().clone();
        for field in self.fields.borrow().iter() {
            field.apply(&mut draft);
        }
        if let Some(entry) = self.pending_selection.borrow().as_ref()
            && draft["wallpaper"]["path"].as_str() == Some(&entry.path)
            && draft["wallpaper"]["wallpaper_type"].as_str() == Some(&entry.wallpaper_type)
        {
            // Use the saved path to decide whether scene overrides need resetting.
            draft["wallpaper"]["path"] = self.baseline.borrow()["wallpaper"]["path"].clone();
            select_wallpaper(&mut draft, entry);
        }
        if self.reset_properties.get() {
            set_pointer(&mut draft, "/scene/properties", json!({}));
        }
        for field in self.property_fields.borrow().iter() {
            field.apply(&mut draft);
        }
        let outputs = self.outputs.value();
        if outputs != *self.outputs.initial.borrow() {
            let mut names = std::collections::HashSet::new();
            for output in outputs.as_array().unwrap() {
                let name = output["name"].as_str().unwrap_or_default();
                if name.is_empty() || !names.insert(name) {
                    bail!(tr("Each display needs a unique, nonempty output name."));
                }
            }
            draft["outputs"] = outputs;
        }
        Ok(draft)
    }

    fn dirty(&self) -> bool {
        self.pending_selection.borrow().is_some()
            || self.reset_properties.get()
            || self
                .fields
                .borrow()
                .iter()
                .chain(self.property_fields.borrow().iter())
                .any(Field::changed)
            || self.outputs.value() != *self.outputs.initial.borrow()
    }

    fn load_config(self: &Rc<Self>, config: Value) {
        self.pending_selection.borrow_mut().take();
        self.reset_properties.set(false);
        for field in self.fields.borrow().iter() {
            field.load(&config);
        }
        self.outputs.load(&config);
        *self.baseline.borrow_mut() = config.clone();
        self.render_properties_for(&config);
    }

    fn render_properties(self: &Rc<Self>) {
        let config = self
            .draft()
            .unwrap_or_else(|_| self.baseline.borrow().clone());
        self.render_properties_for(&config);
    }

    fn render_properties_for(self: &Rc<Self>, config: &Value) {
        clear_box(&self.properties);
        self.property_fields.borrow_mut().clear();
        self.property_rows.borrow_mut().clear();
        let library = self.library.borrow();
        let Some(entry) = library
            .entries
            .iter()
            .find(|entry| Some(entry.path.as_str()) == config["wallpaper"]["path"].as_str())
        else {
            return;
        };
        if entry.scene_properties.is_empty() {
            return;
        }
        self.properties.append(&section(tr("Wallpaper properties")));
        for property in &entry.scene_properties {
            let Some(key) = property["key"].as_str() else {
                continue;
            };
            let label = property["text"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(key);
            let default = property_value(&property["value"]);
            let current = config["scene"]["properties"][key]
                .as_str()
                .unwrap_or(&default);
            let (input, widget) = PropertyInput::new(property, current, &self.window);
            widget.set_tooltip_text(Some(&format!("{} · {}", key, property["prop_type"])));
            let property_row = row(&self.properties, label, &widget);
            self.property_rows.borrow_mut().push((
                property_row,
                property["condition"].as_str().map(str::to_owned),
            ));
            let field = Field {
                pointer: format!(
                    "/scene/properties/{}",
                    key.replace('~', "~0").replace('/', "~1")
                ),
                input: Input::Property(input),
                default: json!(default),
                initial: RefCell::new(Value::Null),
            };
            field.load(config);
            self.property_fields.borrow_mut().push(field);
        }
        for field in self.property_fields.borrow().iter() {
            if let Input::Property(input) = &field.input {
                let weak = Rc::downgrade(self);
                input.connect_changed(Rc::new(move || {
                    if let Some(ui) = weak.upgrade() {
                        ui.refresh_property_visibility();
                    }
                }));
            }
        }
        self.refresh_property_visibility();
    }

    fn reset_property_overrides(self: &Rc<Self>) {
        self.reset_properties.set(true);
        if let Ok(mut draft) = self.draft() {
            draft["scene"]["properties"] = json!({});
            self.render_properties_for(&draft);
            self.message(
                tr("Property overrides cleared. Save changes to apply the defaults."),
                false,
            );
        }
    }

    fn refresh_property_visibility(&self) {
        let values = self
            .property_fields
            .borrow()
            .iter()
            .map(|field| {
                let key = field
                    .pointer
                    .strip_prefix("/scene/properties/")
                    .unwrap_or_default()
                    .replace("~1", "/")
                    .replace("~0", "~");
                (key, property_value(&field.read()))
            })
            .collect();
        for (row, condition) in self.property_rows.borrow().iter() {
            row.set_visible(property_visible(condition.as_deref(), &values));
        }
    }

    fn message(&self, text: &str, error: bool) {
        self.notice.set_text(text);
        self.notice.set_visible(!text.is_empty());
        if error {
            self.notice.add_css_class("error");
        } else {
            self.notice.remove_css_class("error");
        }
    }

    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        self.spinner.set_spinning(busy);
        self.content.set_sensitive(!busy && self.connected.get());
        self.save.set_sensitive(!busy && self.connected.get());
        self.reload.set_sensitive(!busy);
        self.pause.set_sensitive(false);
    }

    fn connect(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        self.set_busy(true);
        self.message(tr("Connecting to the wallpaper service…"), false);
        let explicit = self.explicit_url.clone();
        task(
            self,
            move || Api::connect(explicit),
            |ui, result| match result {
                Ok((api, status, config)) => {
                    info!(url = %api.url, "GTK desktop connected to management API");
                    *ui.api.borrow_mut() = Some(api);
                    ui.connected.set(true);
                    ui.load_config(config);
                    ui.set_busy(false);
                    ui.show_status(status);
                    ui.reload.set_label(tr("Reload settings"));
                    ui.message("", false);
                    ui.refresh_library();
                }
                Err(error) => {
                    ui.connected.set(false);
                    ui.set_busy(false);
                    ui.status.set_text(tr("Offline"));
                    ui.reload.set_label(tr("Retry connection"));
                    ui.message(&format!("{error:#}"), true);
                }
            },
        );
    }

    fn reload_or_confirm(self: &Rc<Self>) {
        if !self.dirty() || self.baseline.borrow().is_null() {
            self.connect();
            return;
        }
        let dialog = gtk::MessageDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .message_type(gtk::MessageType::Question)
            .text(tr("Discard unsaved changes?"))
            .secondary_text(tr(
                "Reloading replaces the settings you have edited with the saved configuration.",
            ))
            .build();
        dialog.add_button(tr("Keep editing"), gtk::ResponseType::Cancel);
        dialog.add_button(tr("Reload"), gtk::ResponseType::Accept);
        let weak = Rc::downgrade(self);
        dialog.connect_response(move |dialog, response| {
            dialog.close();
            if response == gtk::ResponseType::Accept
                && let Some(ui) = weak.upgrade()
            {
                ui.connect();
            }
        });
        dialog.present();
    }

    fn show_status(&self, status: Status) {
        self.connected.set(true);
        self.status
            .set_text(&format!("{} · {}", status.backend, status.playback.label()));
        self.paused.set(status.playback.paused);
        self.pause.set_label(if status.playback.paused {
            tr("Resume")
        } else {
            tr("Pause")
        });
        self.pause.set_sensitive(
            !self.busy.get() && status.playback.running && !status.playback.cancelled,
        );
        self.diagnostics.set_text(&localized!("Desktop: {}\nRunning backend: {}\nPlayback: {}\nPlasma outputs: {}\nDetection: {}\nCandidates: {}\nAPI version: {}", "桌面环境：{}\n当前后端：{}\n播放状态：{}\nPlasma 显示器：{}\n检测依据：{}\n候选桌面：{}\nAPI 版本：{}", status.desktop, status.backend, status.playback.label(), status.plasma_instances.iter().map(|v| v.output.as_str()).collect::<Vec<_>>().join(", "), status.evidence, status.candidates.join(", "), status.api_version));
        if !self.busy.get() {
            self.content.set_sensitive(true);
            self.save.set_sensitive(true);
        }
    }

    fn save_config(self: &Rc<Self>) {
        let Some(api) = self.api.borrow().clone() else {
            return;
        };
        let draft = match self.draft() {
            Ok(draft) => draft,
            Err(error) => {
                self.message(&error.to_string(), true);
                return;
            }
        };
        let base = self.baseline.borrow().clone();
        self.set_busy(true);
        self.message(tr("Saving changes…"), false);
        task(
            self,
            move || api.save(&base, &draft),
            |ui, result| {
                ui.set_busy(false);
                match result {
                    Ok(saved) => {
                        ui.load_config(saved.config);
                        ui.message(if saved.restart_required { tr("Settings saved. Restart the Better Wallpaper service to switch the running backend.") }
                        else if saved.reload_requested { tr("Settings saved. Wallpaper reload requested.") } else { tr("Settings saved.") }, false);
                        ui.refresh_library();
                    }
                    Err(error) => ui.message(&format!("{error:#}"), true),
                }
                ui.poll();
            },
        );
    }

    fn toggle_pause(self: &Rc<Self>) {
        let Some(api) = self.api.borrow().clone() else {
            return;
        };
        let paused = !self.paused.get();
        self.set_busy(true);
        task(
            self,
            move || api.pause(paused),
            |ui, result| {
                ui.set_busy(false);
                match result {
                    Ok(status) => {
                        ui.show_status(status);
                        ui.message(tr("Playback updated."), false);
                    }
                    Err(error) => ui.message(&format!("{error:#}"), true),
                }
            },
        );
    }

    fn poll(self: &Rc<Self>) {
        if self.busy.get() || self.polling.get() {
            return;
        }
        let Some(api) = self.api.borrow().clone() else {
            return;
        };
        self.polling.set(true);
        task(
            self,
            move || api.status(),
            |ui, result| {
                ui.polling.set(false);
                match result {
                    Ok(status) => {
                        let was_offline = !ui.connected.get();
                        ui.show_status(status);
                        if was_offline {
                            ui.message(
                                tr("Connection restored. Unsaved edits have been kept."),
                                false,
                            );
                        }
                    }
                    Err(error) => {
                        if ui.connected.replace(false) {
                            ui.message(&localized!("Connection lost: {error:#}. Retry to rediscover the service; unsaved edits are kept.", "连接已断开：{error:#}。请重试以重新查找服务；尚未保存的更改已保留。"), true);
                        }
                        ui.status.set_text(tr("Offline"));
                        ui.pause.set_sensitive(false);
                        ui.save.set_sensitive(false);
                    }
                }
            },
        );
        if self.stack.visible_child_name().as_deref() == Some("logs")
            && self.follow_logs.is_active()
        {
            self.refresh_logs();
        }
    }

    fn refresh_library(self: &Rc<Self>) {
        if self.library_pending.replace(true) {
            return;
        }
        let Some(api) = self.api.borrow().clone() else {
            self.library_pending.set(false);
            return;
        };
        self.library_info
            .set_text(tr("Scanning wallpaper folders…"));
        task(
            self,
            move || api.library(),
            |ui, result| {
                ui.library_pending.set(false);
                match result {
                    Ok(library) => {
                        *ui.library.borrow_mut() = library;
                        ui.render_library();
                        if !ui.property_fields.borrow().iter().any(Field::changed) {
                            ui.render_properties();
                        }
                    }
                    Err(error) => {
                        ui.library_info.set_text(tr(
                            "The library could not be loaded. Use Refresh to try again.",
                        ));
                        ui.message(&format!("{error:#}"), true);
                    }
                }
            },
        );
    }

    fn render_library(self: &Rc<Self>) {
        self.generation.set(self.generation.get() + 1);
        self.thumbnails.borrow_mut().clear();
        self.library_loaded.set(0);
        while let Some(child) = self.grid.first_child() {
            self.grid.remove(&child);
        }
        let library = self.library.borrow();
        let query = self.search.text().to_lowercase();
        let kind = ["", "video", "web", "scene"][self.filter.selected().min(3) as usize];
        let matching: Vec<_> = library
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                (kind.is_empty() || entry.wallpaper_type == kind)
                    && (entry.name.to_lowercase().contains(&query)
                        || entry.path.to_lowercase().contains(&query))
            })
            .map(|(index, _)| index)
            .collect();
        self.library_info.set_text(&format!(
            "{}{}",
            library
                .roots
                .iter()
                .chain(&library.engine_roots)
                .cloned()
                .collect::<Vec<_>>()
                .join(" · "),
            if library.truncated {
                tr("\nShowing the first 1,000 entries (scan limit).")
            } else {
                ""
            }
        ));
        if matching.is_empty() {
            let empty = wrapped_label(if library.entries.is_empty() {
                tr("No wallpapers found. Add a folder in Library folders, save, then refresh.")
            } else {
                tr("No wallpapers match your search.")
            });
            margins(&empty, 24);
            self.grid.insert(&empty, -1);
        }
        *self.library_matches.borrow_mut() = matching;
        drop(library);
        self.library_scroll.vadjustment().set_value(0.0);
        self.append_library_batch();
        self.queue_library_update();
    }

    fn append_library_batch(self: &Rc<Self>) {
        let library = self.library.borrow();
        let matching = self.library_matches.borrow();
        let start = self.library_loaded.get();
        let end = (start + LIBRARY_BATCH_SIZE).min(matching.len());
        for &index in &matching[start..end] {
            let entry = &library.entries[index];
            let card = column(10);
            margins(&card, 12);
            card.add_css_class("card");
            card.set_size_request(220, -1);
            let picture = gtk::Picture::builder()
                .height_request(130)
                .can_shrink(true)
                .content_fit(gtk::ContentFit::Cover)
                .build();
            let fallback = gtk::Image::from_icon_name("image-x-generic-symbolic");
            fallback.set_pixel_size(48);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&fallback));
            overlay.add_overlay(&picture);
            overlay.set_height_request(130);
            card.append(&overlay);
            let title = gtk::Label::builder()
                .label(&entry.name)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(24)
                .build();
            title.add_css_class("heading");
            title.set_tooltip_text(Some(&entry.path));
            card.append(&title);
            let details = wrapped_label(&format!(
                "{} · {:.1} MB",
                crate::i18n::option_label(&entry.wallpaper_type),
                entry.size_bytes as f64 / 1048576.0
            ));
            details.add_css_class("dim-label");
            card.append(&details);
            if let Some(report) = &entry.scene_compatibility {
                let report_button = gtk::Button::with_label(&localized!(
                    "Compatibility L{} · details",
                    "兼容等级 L{} · 查看详情",
                    report["level"]
                ));
                let report = report.clone();
                let weak = Rc::downgrade(self);
                report_button.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.compatibility(&report);
                    }
                });
                card.append(&report_button);
            }
            let selected =
                self.baseline.borrow()["wallpaper"]["path"].as_str() == Some(&entry.path);
            let choose = gtk::Button::with_label(if selected {
                tr("Use current wallpaper")
            } else {
                tr("Use wallpaper")
            });
            if selected {
                card.add_css_class("accent");
            } else {
                choose.add_css_class("suggested-action");
            }
            let weak = Rc::downgrade(self);
            let entry_copy = entry.clone();
            choose.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.choose(&entry_copy);
                }
            });
            card.append(&choose);
            if entry.wallpaper_type == "video" {
                let preview = gtk::Button::with_label(tr("Preview"));
                let weak = Rc::downgrade(self);
                let path = entry.path.clone();
                preview.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.preview(&path);
                    }
                });
                card.append(&preview);
            }
            self.grid.insert(&card, -1);
            self.thumbnails.borrow_mut().push(Thumbnail {
                path: entry.path.clone(),
                picture: picture.downgrade(),
                fallback: fallback.downgrade(),
            });
        }
        self.library_loaded.set(end);
        self.library_count.set_text(&localized!(
            "Showing {} of {} wallpapers",
            "已显示 {} 个壁纸，共 {} 个",
            end,
            matching.len()
        ));
    }

    fn queue_library_update(self: &Rc<Self>) {
        if self.library_update_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        // Run against allocated widgets and coalesce scroll/layout notifications.
        // GTK pauses this callback while the Wallpapers page is unmapped.
        self.grid.add_tick_callback(move |grid, _| {
            // Tick callbacks run before layout. Replacing cards can leave the
            // scroll bounds unchanged, so no adjustment notification follows.
            // Keep the callback until the replacement cards are allocated.
            if grid.last_child().is_some_and(|card| card.height() == 0) {
                return glib::ControlFlow::Continue;
            }
            if let Some(ui) = weak.upgrade() {
                ui.library_update_pending.set(false);
                ui.update_library_viewport();
            }
            glib::ControlFlow::Break
        });
    }

    fn update_library_viewport(self: &Rc<Self>) {
        if self.stack.visible_child_name().as_deref() != Some("library") || !self.grid.is_mapped() {
            return;
        }
        let adjustment = self.library_scroll.vadjustment();
        if adjustment.page_size() <= 0.0 {
            return;
        }
        let Some(api) = self.api.borrow().clone() else {
            return;
        };
        while self.thumbnails_active.get() < THUMBNAIL_CONCURRENCY {
            let candidate = self.thumbnails.borrow().iter().position(|thumbnail| {
                thumbnail
                    .picture
                    .upgrade()
                    .filter(|picture| picture.height() > 0)
                    .and_then(|picture| picture.compute_bounds(&self.library_scroll))
                    .is_some_and(|bounds| {
                        f64::from(bounds.y() + bounds.height()) >= -LIBRARY_PREFETCH
                            && f64::from(bounds.y()) <= adjustment.page_size() + LIBRARY_PREFETCH
                    })
            });
            let Some(index) = candidate else { break };
            let thumbnail = self.thumbnails.borrow_mut().remove(index);
            let generation = self.generation.get();
            self.thumbnails_active.set(self.thumbnails_active.get() + 1);
            let api = api.clone();
            let weak = Rc::downgrade(self);
            glib::spawn_future_local(async move {
                let result = gio::spawn_blocking(move || api.thumbnail(&thumbnail.path)).await;
                let Some(ui) = weak.upgrade() else { return };
                ui.thumbnails_active.set(ui.thumbnails_active.get() - 1);
                if ui.generation.get() == generation
                    && let Ok(Ok(bytes)) = result
                    && let Some(picture) = thumbnail.picture.upgrade()
                    && let Ok(texture) = gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes))
                {
                    picture.set_paintable(Some(&texture));
                    if let Some(fallback) = thumbnail.fallback.upgrade() {
                        fallback.set_visible(false);
                    }
                }
                ui.queue_library_update();
            });
        }
        if self.library_loaded.get() < self.library_matches.borrow().len()
            && adjustment.value() + adjustment.page_size() + LIBRARY_PREFETCH >= adjustment.upper()
        {
            self.append_library_batch();
        }
    }

    fn choose(self: &Rc<Self>, entry: &Entry) {
        let mut draft = match self.draft() {
            Ok(value) => value,
            Err(error) => {
                self.message(&error.to_string(), true);
                return;
            }
        };
        select_wallpaper(&mut draft, entry);
        // Keep the saved baseline for conflict detection while rendering the new
        // draft. Updating field initials here would otherwise hide pending edits.
        for field in self.fields.borrow().iter() {
            let initial = field.initial.borrow().clone();
            field.load(&draft);
            *field.initial.borrow_mut() = initial;
        }
        // Preserve the selection metadata until save, including engine_mode and
        // reset scene properties, through a separate pending document.
        *self.pending_selection.borrow_mut() = Some(entry.clone());
        self.render_properties_for(&draft);
        self.stack.set_visible_child_name("wallpaper");
        self.message(
            &localized!(
                "Selected {}. Save changes to apply this wallpaper.",
                "已选择 {}。保存更改后将应用此壁纸。",
                entry.name
            ),
            false,
        );
    }

    fn compatibility(&self, report: &Value) {
        let mut text = String::from(tr(
            "Metadata analysis does not guarantee playback on the current backend.\n",
        ));
        for (key, title) in [
            ("supported_features", tr("Supported features")),
            ("unsupported_features", tr("Unsupported features")),
            ("warnings", tr("Warnings")),
        ] {
            text.push_str(&format!("\n{title}\n"));
            if let Some(items) = report[key].as_array() {
                for item in items {
                    text.push_str(&format!("• {}\n", item.as_str().unwrap_or_default()));
                }
            }
        }
        let dialog = gtk::Window::builder()
            .title(tr("Scene compatibility"))
            .transient_for(&self.window)
            .default_width(560)
            .default_height(420)
            .build();
        let label = wrapped_label(&text);
        label.set_selectable(true);
        margins(&label, 24);
        dialog.set_child(Some(&gtk::ScrolledWindow::builder().child(&label).build()));
        dialog.present();
    }

    fn preview(&self, path: &str) {
        let Some(api) = self.api.borrow().clone() else {
            return;
        };
        let Ok(mut url) = api.url.join("api/v1/library/media") else {
            return;
        };
        url.query_pairs_mut().append_pair("path", path);
        let media = gtk::MediaFile::for_file(&gio::File::for_uri(url.as_str()));
        media.set_muted(true);
        media.set_loop(true);
        let video = gtk::Video::builder()
            .media_stream(&media)
            .autoplay(true)
            .build();
        let dialog = gtk::Window::builder()
            .title(tr("Wallpaper preview"))
            .transient_for(&self.window)
            .default_width(720)
            .default_height(440)
            .build();
        let content = column(8);
        let error = wrapped_label("");
        error.set_visible(false);
        margins(&error, 12);
        video.set_vexpand(true);
        content.append(&video);
        content.append(&error);
        dialog.set_child(Some(&content));
        media.connect_error_notify(move |media| { if let Some(message) = media.error() { error.set_text(&localized!("Preview unavailable: {message}. Check the GTK media backend and installed codecs.", "无法预览：{message}。请检查 GTK 媒体后端和已安装的解码器。")); error.set_visible(true); } });
        dialog.connect_destroy(move |_| {
            media.pause();
            media.clear();
        });
        dialog.present();
    }

    fn refresh_logs(self: &Rc<Self>) {
        if self.logs_pending.replace(true) {
            return;
        }
        let Some(api) = self.api.borrow().clone() else {
            self.logs_pending.set(false);
            return;
        };
        task(
            self,
            move || api.logs(),
            |ui, result| {
                ui.logs_pending.set(false);
                match result {
                    Ok(text) => {
                        *ui.log_text.borrow_mut() = text;
                        ui.render_logs();
                    }
                    Err(error) => ui.message(
                        &localized!("Cannot read logs: {error:#}", "无法读取日志：{error:#}"),
                        true,
                    ),
                }
            },
        );
    }

    fn render_logs(&self) {
        let query = self.log_search.text().to_lowercase();
        let text = self
            .log_text
            .borrow()
            .lines()
            .filter(|line| line.to_lowercase().contains(&query))
            .collect::<Vec<_>>()
            .join("\n");
        self.logs.buffer().set_text(&text);
    }
}

fn task<T: Send + 'static>(
    ui: &Rc<Desktop>,
    work: impl FnOnce() -> Result<T> + Send + 'static,
    done: impl FnOnce(Rc<Desktop>, Result<T>) + 'static,
) {
    let weak: Weak<Desktop> = Rc::downgrade(ui);
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(work)
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!(tr("Background request failed"))));
        if let Err(error) = &result {
            warn!(%error, "GTK desktop request failed");
        }
        if let Some(ui) = weak.upgrade() {
            done(ui, result);
        }
    });
}

fn column(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}
fn margins(widget: &impl IsA<gtk::Widget>, value: i32) {
    widget.set_margin_top(value);
    widget.set_margin_bottom(value);
    widget.set_margin_start(value);
    widget.set_margin_end(value);
}
fn wrapped_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .build()
}
fn section(text: &str) -> gtk::Label {
    let label = wrapped_label(text);
    label.add_css_class("title-3");
    label.set_margin_top(16);
    label
}
fn page(title: &str, subtitle: &str) -> gtk::Box {
    let page = column(16);
    margins(&page, 28);
    let title = wrapped_label(title);
    title.add_css_class("title-1");
    page.append(&title);
    let subtitle = wrapped_label(subtitle);
    subtitle.add_css_class("dim-label");
    page.append(&subtitle);
    page
}
fn add_page(stack: &gtk::Stack, child: &gtk::Box, name: &str, title: &str) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(child)
        .build();
    stack.add_titled(&scroll, Some(name), title);
    scroll
}
fn row(page: &gtk::Box, title: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let label = wrapped_label(title);
    label.set_hexpand(true);
    label.set_mnemonic_widget(Some(widget));
    row.append(&label);
    row.append(widget);
    page.append(&row);
    row
}
fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
fn buffer_text(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
