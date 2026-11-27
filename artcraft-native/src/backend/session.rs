//! Where the signed-in session lives: on desktop under `~/Artcraft/credentials` (seeded from
//! the cookie jar), and in the browser in `localStorage`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The two session cookies, as sent on every API request (`Cookie: visitor=…; session=…`).
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
  pub session: String,
  #[serde(default)]
  pub visitor: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
mod desktop {
  use std::fs;
  use std::path::{Path, PathBuf};

  use artcraft_client::credentials::storyteller_avt_cookie::StorytellerAvtCookie;
  use artcraft_client::credentials::storyteller_credential_set::StorytellerCredentialSet;
  use artcraft_client::credentials::storyteller_session_cookie::StorytellerSessionCookie;
  use artcraft_client::utils::api_host::ApiHost;
  use log::{info, warn};
  use serde::Deserialize;

  use super::Credentials;

  const TAURI_APP_IDENTIFIER: &str = "ai.artcraft.app";
  const TAURI_COOKIES_FILENAME: &str = ".cookies";
  const CREDENTIALS_FILENAME: &str = "native_session.json";

  impl Credentials {
    pub fn to_set(&self) -> StorytellerCredentialSet {
      StorytellerCredentialSet::initialize(self.visitor.clone().map(StorytellerAvtCookie::new), Some(StorytellerSessionCookie::new(self.session.clone())))
    }
  }

  /// Desktop data root shared with local credentials.
  pub fn data_root() -> PathBuf {
    directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from(".")).join("Artcraft")
  }

  /// The API host from `settings/env_configs.json` (production unless configured to localhost).
  pub fn api_host(root: &Path) -> ApiHost {
    #[derive(Deserialize)]
    struct EnvConfigs {
      storyteller_host: Option<String>,
      storyteller_port: Option<u32>,
    }
    let path = root.join("settings").join("env_configs.json");
    let configs = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<EnvConfigs>(&s).ok());
    match configs {
      Some(EnvConfigs { storyteller_host: Some(host), storyteller_port }) if host == "localhost" => ApiHost::Localhost { port: storyteller_port.unwrap_or(12345) },
      _ => ApiHost::Storyteller,
    }
  }

  /// The saved session, or imported from the desktop cookie jar.
  pub fn load(root: &Path) -> Option<Credentials> {
    let path = credentials_path(root);
    if let Ok(text) = fs::read_to_string(&path) {
      match serde_json::from_str::<Credentials>(&text) {
        Ok(creds) if !creds.session.is_empty() => return Some(creds),
        Ok(_) => {},
        Err(err) => warn!("Ignoring unreadable {}: {err}", path.display()),
      }
    }
    let imported = import_tauri_cookies()?;
    info!("Imported existing login from the desktop cookie jar");
    save(root, &imported);
    Some(imported)
  }

  pub fn save(root: &Path, creds: &Credentials) {
    let path = credentials_path(root);
    let result = path.parent().map_or(Ok(()), fs::create_dir_all).and_then(|_| fs::write(&path, serde_json::to_vec_pretty(creds).unwrap_or_default()));
    if let Err(err) = result {
      warn!("Couldn't save the session to {}: {err}", path.display());
    }
  }

  pub fn delete(root: &Path) {
    let path = credentials_path(root);
    if let Err(err) = fs::remove_file(&path) {
      if err.kind() != std::io::ErrorKind::NotFound {
        warn!("Couldn't delete {}: {err}", path.display());
      }
    }
  }

  fn credentials_path(root: &Path) -> PathBuf {
    root.join("credentials").join(CREDENTIALS_FILENAME)
  }

  fn import_tauri_cookies() -> Option<Credentials> {
    let path = directories::BaseDirs::new()?.cache_dir().join(TAURI_APP_IDENTIFIER).join(TAURI_COOKIES_FILENAME);
    let text = fs::read_to_string(&path).ok()?;
    let cookies: Vec<serde_json::Value> = serde_json::from_str(&text).ok()?;
    let now = chrono::Utc::now();
    let mut creds = Credentials::default();
    for entry in &cookies {
      let for_api = entry.get("domain").map(|d| d.to_string()).is_some_and(|d| d.contains("storyteller.ai"));
      let expired = entry.pointer("/expires/AtUtc").and_then(|v| v.as_str()).and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).is_some_and(|t| t < now);
      if !for_api || expired {
        continue;
      }
      let Some((name, value)) = entry.get("raw_cookie").and_then(|v| v.as_str()).and_then(|s| s.split(';').next()).and_then(|kv| kv.split_once('=')) else {
        continue;
      };
      match name.trim() {
        "session" => creds.session = value.trim().to_owned(),
        "visitor" => creds.visitor = Some(value.trim().to_owned()),
        _ => {},
      }
    }
    (!creds.session.is_empty()).then_some(creds)
  }
}

#[cfg(not(target_arch = "wasm32"))]
pub use desktop::{api_host, data_root, delete, load, save};

#[cfg(target_arch = "wasm32")]
mod web {
  use std::path::{Path, PathBuf};

  use super::Credentials;

  const STORAGE_KEY: &str = "fogbreak-hub.session";
  const HOST_STORAGE_KEY: &str = "fogbreak-hub.api_host";
  const DEFAULT_API_HOST: &str = "";

  fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok()).flatten()
  }

  pub fn data_root() -> PathBuf {
    PathBuf::from("browser-storage")
  }

  pub fn api_host(_root: &Path) -> String {
    if let Some(search) = web_sys::window().and_then(|w| w.location().search().ok()) {
      for pair in search.trim_start_matches('?').split('&') {
        if let Some((k, v)) = pair.split_once('=') {
          if k == "api_host" {
            let val = v.trim();
            if val == "localhost" {
              return "http://localhost:12345".to_owned();
            }
            if val.starts_with("http://") || val.starts_with("https://") {
              return val.trim_end_matches('/').to_owned();
            }
          }
        }
      }
    }
    if let Some(saved) = storage().and_then(|s| s.get_item(HOST_STORAGE_KEY).ok()).flatten() {
      let trimmed = saved.trim();
      if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.trim_end_matches('/').to_owned();
      }
    }
    DEFAULT_API_HOST.to_owned()
  }

  pub fn load(_root: &Path) -> Option<Credentials> {
    let text = storage()?.get_item(STORAGE_KEY).ok().flatten()?;
    let creds: Credentials = serde_json::from_str(&text).ok()?;
    (!creds.session.is_empty()).then_some(creds)
  }

  pub fn save(_root: &Path, creds: &Credentials) {
    if let Some(store) = storage() {
      if let Ok(text) = serde_json::to_string(creds) {
        let _ = store.set_item(STORAGE_KEY, &text);
      }
    }
  }

  pub fn delete(_root: &Path) {
    if let Some(store) = storage() {
      let _ = store.remove_item(STORAGE_KEY);
    }
  }
}

#[cfg(target_arch = "wasm32")]
pub use web::{api_host, data_root, delete, load, save};
