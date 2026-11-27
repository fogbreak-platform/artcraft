#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Native desktop entrypoint for Fogbreak Hub (`artcraft_native`).

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
  const APP_ID: &str = "io.fogbreak.hub";

  env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
  let mut viewport = egui::ViewportBuilder::default().with_title("Fogbreak Hub").with_app_id(APP_ID).with_inner_size([1440.0, 900.0]).with_min_inner_size([960.0, 640.0]);
  if let Some(icon) = app_icon() {
    viewport = viewport.with_icon(icon);
  }
  // The app draws its own title bar; macOS keeps its traffic lights over it.
  viewport = if cfg!(target_os = "macos") { viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false) } else { viewport.with_decorations(false) };
  let options = eframe::NativeOptions { viewport, persist_window: true, ..Default::default() };
  eframe::run_native("Fogbreak Hub", options, Box::new(|cc| Ok(Box::new(artcraft_native::app::ArtcraftApp::new(cc)))))
}

/// The window and taskbar icon (Fogbreak Tidal Strata geometric emblem).
#[cfg(not(target_arch = "wasm32"))]
fn app_icon() -> Option<egui::IconData> {
  const ICON_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="#111821"/><rect x="14" y="16" width="36" height="8" rx="4" fill="#087F9C"/><rect x="14" y="28" width="28" height="8" rx="4" fill="#EAF1F7"/><rect x="14" y="40" width="20" height="8" rx="4" fill="#087F9C"/></svg>"##;
  let tree = resvg::usvg::Tree::from_data(ICON_SVG, &resvg::usvg::Options::default()).ok()?;
  let mut pixmap = resvg::tiny_skia::Pixmap::new(64, 64)?;
  resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut());
  Some(egui::IconData { width: 64, height: 64, rgba: pixmap.take() })
}

#[cfg(target_arch = "wasm32")]
fn main() {}
