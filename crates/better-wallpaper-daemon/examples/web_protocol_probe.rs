//! Test-only private origin; no saved desktop/service configuration is touched.
use anyhow::Result;
use better_wallpaper_daemon::{
    web_protocol::WebOptions,
    web_wallpaper::{AssetServer, WebProject},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::{self, BufRead},
};
fn main() -> Result<()> {
    let path = std::env::args().nth(1).expect("fixture directory");
    let project = WebProject::load(path.as_ref())?;
    let assets = AssetServer::start_with_options(
        project.clone(),
        WebOptions {
            audio: true,
            media: true,
        },
    )?;
    let properties = project.resolved_properties(
        &BTreeMap::from([
            ("caption".into(), "Saved caption".into()),
            ("enabled".into(), "false".into()),
            ("speed".into(), "0.25".into()),
        ]),
        &format!("{}/", assets.origin),
    )?;
    println!(
        "{}",
        json!({"url":format!("{}{}",assets.origin, project.entry_url("/")),"protocolUrl":assets.protocol_url,"properties":properties,"outputs":[],"muted":true,"paused":false,"fps":37})
    );
    for line in io::stdin().lock().lines() {
        if line?.trim() == "stop" {
            break;
        }
    }
    drop(assets);
    Ok(())
}
