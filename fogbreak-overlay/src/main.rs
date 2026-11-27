//! Fogbreak Hub browser overlay (`wasm32`): `<canvas id="fogbreak_canvas" data-host="page" data-engine="fogbreak-hub" data-rpc-key="fogbreakHubHost" data-iframe="false">`.

#[cfg(target_arch = "wasm32")]
mod host_bridge;
#[cfg(target_arch = "wasm32")]
mod theme;

#[cfg(target_arch = "wasm32")]
mod web_app {
  use artcraft_native::app::ArtcraftApp;
  use serde_json::json;

  use crate::host_bridge::{self, HostBridge};
  use crate::theme;

  const LOADING_ID: &str = "fogbreak_loading";

  struct FogbreakHubShell {
    app: ArtcraftApp,
    bridge: HostBridge,
  }

  impl eframe::App for FogbreakHubShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
      self.app.step_logic(ctx);
      self.bridge.pump(&mut self.app, ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
      self.app.step_ui(ui, frame);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
      self.app.save_state(storage);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
      eframe::App::clear_color(&self.app, visuals)
    }
  }

  pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    let dark = theme::initial_dark_mode();
    theme::sync_document_theme(dark);

    wasm_bindgen_futures::spawn_local(async move {
      let Some(canvas) = host_bridge::find_page_canvas() else {
        log::error!("missing <canvas id=\"fogbreak_canvas\" data-engine=\"fogbreak-hub\">");
        return;
      };
      let canvas_for_start = canvas.clone();
      let canvas_for_err = canvas.clone();
      let options = eframe::WebOptions::default();

      let result = eframe::WebRunner::new()
        .start(
          canvas_for_start,
          options,
          Box::new(move |cc| {
            let app = ArtcraftApp::new(cc);
            theme::apply_palette(&cc.egui_ctx, dark);
            let bridge = HostBridge::install(&cc.egui_ctx, canvas);
            Ok(Box::new(FogbreakHubShell { app, bridge }))
          }),
        )
        .await;

      if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id(LOADING_ID)) {
        match result {
          Ok(()) => el.remove(),
          Err(ref e) => {
            let msg = format!("{e:?}");
            el.set_text_content(Some(&format!("Fogbreak Hub failed to start: {msg}. A browser with WebGL2 is required.")));
            host_bridge::emit_event(Some(&canvas_for_err), "failed", &json!({ "error": msg }));
          },
        }
      } else if let Err(ref e) = result {
        host_bridge::emit_event(Some(&canvas_for_err), "failed", &json!({ "error": format!("{e:?}") }));
      }
    });
  }
}

#[cfg(target_arch = "wasm32")]
fn main() {
  web_app::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
