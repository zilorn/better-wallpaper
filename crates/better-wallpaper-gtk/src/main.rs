mod client;
mod i18n;
mod properties;
mod ui;

use clap::Parser;
use gtk::prelude::*;
use reqwest::Url;

#[derive(Parser)]
#[command(version, about = "Better Wallpaper GTK desktop client")]
struct Cli {
    /// Connect to a local daemon without starting the installed user service.
    #[arg(long, value_parser = client::validate_endpoint)]
    url: Option<Url>,
    /// Interface language (en or zh-CN); defaults to the system locale.
    #[arg(long, value_enum)]
    language: Option<i18n::Language>,
}

fn main() -> gtk::glib::ExitCode {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    i18n::init(cli.language);
    let app = gtk::Application::builder()
        .application_id("org.betterwallpaper.desktop")
        .build();
    app.connect_activate(move |app| {
        if let Some(window) = app.active_window() {
            window.present();
        } else {
            ui::Desktop::open(app, cli.url.clone());
        }
    });
    // Clap handles our arguments; GApplication still forwards activation to the
    // primary instance through the desktop session bus.
    app.run_with_args(&["better-wallpaper-desktop"])
}
