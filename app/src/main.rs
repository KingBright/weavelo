#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // A Dock tile owns only an AppKit event loop and its parent pipe. Avoid a
    // full network/codec executor, with one worker per CPU, for every window.
    #[cfg(all(target_os = "macos", feature = "gpui-restoration"))]
    if std::env::args().skip(1).eq(["--macos-dock-tile"]) {
        return weavelo_app::restored_ui::run_macos_dock_tile();
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(weavelo_app::entrypoint::run())
}
