//! Model makers' logos rasterized once into white-on-transparent textures so they can be
//! tinted cleanly in both dark and light modes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::{ColorImage, TextureHandle, TextureOptions};

/// Rasterized side, in pixels (crisp up to ~32 pt at 2x).
const RASTER_SIDE: u32 = 64;

/// Clean geometric Fogbreak Tidal Strata mark used for built-in / workspace engines.
const WORKSPACE_MARK_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect x="10" y="14" width="44" height="9" rx="4.5" fill="#fff"/><rect x="10" y="27.5" width="34" height="9" rx="4.5" fill="#fff"/><rect x="10" y="41" width="24" height="9" rx="4.5" fill="#fff"/></svg>"##;

macro_rules! service_svg {
  ($name:literal) => {
    include_bytes!(concat!("../../../frontend/apps/artcraft/app/public/resources/images/services/", $name, ".svg"))
  };
}

/// The logo for a `ModelCreator` (snake_case, as the model listing sends it).
fn svg_for_creator(creator: &str) -> &'static [u8] {
  match creator {
    "alibaba" => service_svg!("alibaba"),
    "artcraft" | "fogbreak" | "fogbreak-hub" => WORKSPACE_MARK_SVG,
    "black_forest_labs" | "blackforestlabs" | "bfl" => service_svg!("blackforestlabs"),
    "bytedance" => service_svg!("bytedance"),
    "fal" => service_svg!("fal"),
    "google" => service_svg!("google"),
    "grok" | "xai" => service_svg!("grok"),
    "higgsfield" => service_svg!("higgsfield"),
    "kling" | "kuaishou" => service_svg!("kling"),
    "krea" => service_svg!("krea"),
    "midjourney" => service_svg!("midjourney"),
    "minimax" | "hailuo" => service_svg!("minimax"),
    "open_ai" | "openai" => service_svg!("openai"),
    "openart" | "open_art" => service_svg!("openart"),
    "recraft" => service_svg!("recraft"),
    "replicate" => service_svg!("replicate"),
    "suno" => service_svg!("suno"),
    "tencent" => service_svg!("tencent"),
    "tensor_art" | "tensorart" => service_svg!("tensorart"),
    "vidu" => service_svg!("vidu"),
    "world_labs" | "worldlabs" => service_svg!("worldlabs"),
    _ => service_svg!("generic"),
  }
}

#[derive(Clone, Default)]
struct Cache(Arc<Mutex<HashMap<String, TextureHandle>>>);

/// A white logo for `creator`, to be tinted when drawn.
pub fn texture(ctx: &egui::Context, creator: &str) -> TextureHandle {
  let cache: Cache = ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(egui::Id::new("creator-icons")).clone());
  let mut map = cache.0.lock().unwrap_or_else(|e| e.into_inner());
  if let Some(texture) = map.get(creator) {
    return texture.clone();
  }
  let image = rasterize_mask(svg_for_creator(creator)).unwrap_or_else(|| ColorImage::filled([1, 1], egui::Color32::TRANSPARENT));
  let texture = ctx.load_texture(format!("creator-{creator}"), image, TextureOptions::LINEAR);
  map.insert(creator.to_owned(), texture.clone());
  texture
}

/// Renders an SVG and keeps only its coverage: every pixel becomes white at the original alpha.
fn rasterize_mask(svg: &[u8]) -> Option<ColorImage> {
  let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
  let size = tree.size();
  let scale = RASTER_SIDE as f32 / size.width().max(size.height());
  let mut pixmap = resvg::tiny_skia::Pixmap::new(RASTER_SIDE, RASTER_SIDE)?;
  let dx = (RASTER_SIDE as f32 - size.width() * scale) / 2.0;
  let dy = (RASTER_SIDE as f32 - size.height() * scale) / 2.0;
  resvg::render(&tree, resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy), &mut pixmap.as_mut());
  let pixels = pixmap.pixels().iter().map(|p| egui::Color32::from_white_alpha(p.alpha())).collect();
  Some(ColorImage::new([RASTER_SIDE as usize, RASTER_SIDE as usize], pixels))
}
