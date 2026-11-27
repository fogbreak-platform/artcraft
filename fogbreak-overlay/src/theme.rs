#![cfg(target_arch = "wasm32")]

use egui::Context;
use wasm_bindgen::JsCast as _;

pub fn initial_dark_mode() -> bool {
  let Some(window) = web_sys::window() else {
    return true;
  };
  if let Ok(search) = window.location().search() {
    if let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search) {
      if let Some(theme) = params.get("theme") {
        return !theme.eq_ignore_ascii_case("light");
      }
    }
  }
  if let Some(doc) = window.document() {
    if let Some(root) = doc.document_element() {
      if let Some(attr) = root.get_attribute("data-theme") {
        return !attr.eq_ignore_ascii_case("light");
      }
    }
  }
  match window.match_media("(prefers-color-scheme: light)") {
    Ok(Some(mql)) => !mql.matches(),
    _ => true,
  }
}

pub fn sync_document_theme(dark: bool) {
  let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
    return;
  };
  let mode = if dark { "dark" } else { "light" };
  let css_bg = if dark { "#111821" } else { "#F7FAFC" };
  if let Some(root) = doc.document_element() {
    let _ = root.set_attribute("data-theme", mode);
    if let Ok(el) = root.dyn_into::<web_sys::HtmlElement>() {
      let _ = el.style().set_property("background", css_bg);
    }
  }
  if let Some(body) = doc.body() {
    let _ = body.set_attribute("data-theme", mode);
    if let Ok(el) = body.dyn_into::<web_sys::HtmlElement>() {
      let _ = el.style().set_property("background", css_bg);
    }
  }
}

pub fn apply_palette(ctx: &Context, dark: bool) {
  artcraft_native::theme::apply_theme(ctx, dark);
  sync_document_theme(dark);
}
