//! Generating through another provider (`Midjourney` with the user's account, or `FAL` with
//! an API key) alongside the built-in workspace engine.

pub mod accounts;
pub mod jobs;
pub mod submit;
#[cfg(not(target_arch = "wasm32"))]
pub mod worker;

use serde::{Deserialize, Serialize};

use crate::models::ModelInfo;

/// Midjourney's models; direct jobs always make four images and take no references.
const MIDJOURNEY_MODELS: [&str; 4] = ["midjourney", "midjourney_7", "midjourney_7_niji", "midjourney_8"];
const MIDJOURNEY_BATCH: u16 = 4;

/// Who runs a generation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
  #[default]
  Artcraft,
  Midjourney,
  Fal,
}

impl Provider {
  /// The picker's display name.
  pub fn label(self) -> &'static str {
    match self {
      Provider::Artcraft => "Fogbreak Hub",
      Provider::Midjourney => "Midjourney",
      Provider::Fal => "FAL",
    }
  }

  /// The logo's key in the creator icons.
  pub fn icon(self) -> &'static str {
    match self {
      Provider::Artcraft => "fogbreak",
      Provider::Midjourney => "midjourney",
      Provider::Fal => "fal",
    }
  }

  /// The providers `model_id` can run on, the default first.
  pub fn for_model(model_id: &str) -> &'static [Provider] {
    match model_id {
      "flux_1_dev" | "flux_1_schnell" | "nano_banana_2" | "nano_banana_pro" => &[Provider::Artcraft, Provider::Fal],
      "midjourney" => &[Provider::Midjourney, Provider::Artcraft],
      "midjourney_7" | "midjourney_7_niji" | "midjourney_8" => &[Provider::Artcraft, Provider::Midjourney],
      _ => &[Provider::Artcraft],
    }
  }

  /// `picked` if `model_id` runs there, else the model's default.
  pub fn resolve(model_id: &str, picked: Option<Provider>) -> Provider {
    let allowed = Provider::for_model(model_id);
    picked.filter(|p| allowed.contains(p)).unwrap_or(allowed[0])
  }

  /// What the model can do on this provider: direct Midjourney makes four images without
  /// references, resolution or quality choices.
  pub fn adapt(self, model: &ModelInfo) -> ModelInfo {
    let mut m = model.clone();
    if self == Provider::Midjourney && is_midjourney(&model.id) {
      m.batch_sizes = vec![MIDJOURNEY_BATCH];
      m.batch_default = MIDJOURNEY_BATCH;
      m.image_refs_max = 0;
      m.resolutions.clear();
      m.resolution_default = None;
      m.qualities.clear();
      m.quality_default = None;
    }
    m
  }
}

pub fn is_midjourney(model_id: &str) -> bool {
  MIDJOURNEY_MODELS.contains(&model_id)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn models_offer_their_providers_default_first() {
    assert_eq!(Provider::for_model("nano_banana_pro"), [Provider::Artcraft, Provider::Fal]);
    assert_eq!(Provider::for_model("midjourney"), [Provider::Midjourney, Provider::Artcraft]);
    assert_eq!(Provider::for_model("midjourney_8"), [Provider::Artcraft, Provider::Midjourney]);
    assert_eq!(Provider::for_model("seedream_4"), [Provider::Artcraft]);
    assert_eq!(Provider::resolve("seedream_4", Some(Provider::Fal)), Provider::Artcraft, "a pick the model can't use falls back");
    assert_eq!(Provider::resolve("flux_1_dev", Some(Provider::Fal)), Provider::Fal);
  }

  #[test]
  fn direct_midjourney_makes_four_images_without_options() {
    let model = ModelInfo { id: "midjourney_7".into(), batch_sizes: vec![1, 2, 4], batch_default: 1, image_refs_max: 1, resolutions: vec!["one_k".into()], aspect_ratios: vec!["square".into()], ..Default::default() };
    let direct = Provider::Midjourney.adapt(&model);
    assert_eq!((direct.batch_sizes.as_slice(), direct.batch_default, direct.image_refs_max), ([4].as_slice(), 4, 0));
    assert!(direct.resolutions.is_empty());
    assert_eq!(direct.aspect_ratios, model.aspect_ratios, "aspect ratios stay");
    assert_eq!(Provider::Artcraft.adapt(&model).batch_sizes, model.batch_sizes);
  }
}
