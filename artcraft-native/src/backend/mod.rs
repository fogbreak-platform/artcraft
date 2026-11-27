//! API client runtime across desktop (`tokio` + `reqwest`) and browser (`wasm32` `fetch`).

pub mod audio;
pub mod media_cache;
pub mod providers;
pub mod session;
pub mod video;
#[cfg(target_arch = "wasm32")]
pub mod web_http;
pub mod wire;

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::feed::types::MediaKind;
use crate::models::ModelInfo;
use providers::Provider;
use wire::{MediaFile, SessionUser};

/// Items per library page.
pub const LIBRARY_PAGE_SIZE: u32 = 40;
/// Model listings are retried a few times before giving up.
const MODEL_LIST_ATTEMPTS: u32 = 3;
const MODEL_LIST_BACKOFF: Duration = Duration::from_millis(250);
/// The share link base.
pub const SHARE_URL_BASE: &str = "https://fogbreak.io/media/";
/// Provider-aware cost estimate endpoint.
const PROVIDER_COST_PATH: &str = "/v1/generate/cost_estimate/image";
/// Characters are created for this model.
const CHARACTER_MODEL: &str = "seedance_2p0";

/// Which media an operation is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Modality {
  Image,
  Video,
  Audio,
}

impl Modality {
  fn path(self) -> &'static str {
    match self {
      Modality::Image => "image",
      Modality::Video => "video",
      Modality::Audio => "audio",
    }
  }
}

impl From<MediaKind> for Modality {
  fn from(kind: MediaKind) -> Self {
    match kind {
      MediaKind::Image => Modality::Image,
      MediaKind::Video => Modality::Video,
      MediaKind::Audio => Modality::Audio,
    }
  }
}

/// A result coming back from the backend.
pub enum Event {
  Session(Option<SessionUser>),
  LoginFailed(String),
  DeviceLogin {
    url: String,
    code: String,
  },
  Credits(u64),
  Models(Modality, Result<Vec<ModelInfo>, String>),
  Cost {
    key: String,
    credits: Option<u64>,
  },
  Enqueued {
    modality: Modality,
    job_tokens: Vec<String>,
    meta: EnqueueMeta,
  },
  EnqueueFailed {
    modality: Modality,
    message: String,
    status: Option<u16>,
  },
  Uploaded {
    ref_id: u64,
    result: Result<UploadedMedia, String>,
  },
  Jobs(Vec<wire::Job>),
  LibraryPage {
    key: String,
    page: u32,
    items: Vec<MediaFile>,
    total_pages: u32,
  },
  LibraryFailed {
    key: String,
    message: String,
  },
  Batch {
    job_token: String,
    items: Vec<MediaFile>,
  },
  Prompt(wire::Prompt),
  Characters(Vec<wire::Character>),
  CharacterCreating {
    job_token: String,
    name: String,
  },
  CharacterChanged,
  ProviderSetupNeeded(Provider),
  ProviderJobFinished {
    job_id: String,
    batch_token: String,
  },
  ProviderJobFailed {
    job_id: String,
    reason: String,
  },
  ProviderAccountsChanged,
  Deleted(String),
  Saved(PathBuf),
  Toast {
    error: bool,
    message: String,
  },
}

/// What the UI remembers about a request it just sent, to label the pending card.
#[derive(Clone, Debug)]
pub struct EnqueueMeta {
  pub prompt: String,
  pub model_id: String,
  pub batch_count: u32,
  pub ref_image: Option<String>,
  pub provider: Provider,
}

/// A finished upload: the media token, plus what the server knows about the file.
#[derive(Clone, Debug)]
pub struct UploadedMedia {
  pub token: String,
  pub duration_secs: f32,
  pub thumbnail: Option<String>,
  pub full_url: Option<String>,
}

fn fal_cost_body(fields: &Map<String, Value>) -> Value {
  let images = fields.get("image_media_tokens").and_then(Value::as_array).map_or(0, Vec::len);
  let mode = if images == 0 { serde_json::json!({ "type": "text_to_image" }) } else { serde_json::json!({ "type": "image_edit", "count": images }) };
  let field = |name: &str| fields.get(name).cloned().unwrap_or(Value::Null);
  serde_json::json!({ "model": field("model"), "provider": "fal", "generation_mode": mode, "aspect_ratio": field("aspect_ratio"), "resolution": field("resolution"), "quality": field("quality"), "image_batch_count": field("image_batch_count") })
}

fn job_tokens(response: &Value) -> Vec<String> {
  let mut tokens: Vec<String> = response.get("inference_job_token").and_then(Value::as_str).map(str::to_owned).into_iter().collect();
  for t in response.get("all_job_tokens").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
    if !tokens.iter().any(|x| x == t) {
      tokens.push(t.to_owned());
    }
  }
  tokens
}

fn body_message(body: &str) -> Option<String> {
  let v: Value = serde_json::from_str(body).ok()?;
  v.get("error_message").or_else(|| v.get("message")).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(not(target_arch = "wasm32"))]
mod desktop {
  use std::future::Future;
  use std::path::{Path, PathBuf};
  use std::sync::atomic::{AtomicBool, Ordering};
  use std::sync::mpsc::{Receiver, Sender, channel};
  use std::sync::{Arc, RwLock};
  use std::time::Duration;

  use artcraft_api_defs::users::login_challenges::LoginChallengeState;
  use artcraft_client::credentials::storyteller_credential_set::StorytellerCredentialSet;
  use artcraft_client::endpoints::credits::get_session_credits::get_session_credits;
  use artcraft_client::endpoints::media_files::delete_media_file::delete_media_file;
  use artcraft_client::endpoints::media_files::upload_image_media_file_from_bytes::{ImageType, UploadImageBytesArgs, upload_image_media_file_from_bytes};
  use artcraft_client::endpoints::media_files::upload_image_media_file_from_file::{UploadImageFromFileArgs, upload_image_media_file_from_file};
  use artcraft_client::endpoints::media_files::upload_video_media_file_from_file::{UploadVideoFromFileArgs, upload_video_media_file_from_file};
  use artcraft_client::endpoints::users::password_login::PasswordLoginRequest;
  use artcraft_client::error::api_error::ApiError;
  use artcraft_client::error::storyteller_error::StorytellerError;
  use artcraft_client::utils::api_host::ApiHost;
  use artcraft_client::utils::basic_json_post_request::basic_json_post_request;
  use artcraft_client::utils::login_challenge_client::{LoginChallengeClient, LoginChallengeClientError};
  use cookie_store_wrapper::cookie_store::CookieStore;
  use enums::common::payments_namespace::PaymentsNamespace;
  use log::{error, warn};
  use serde::de::DeserializeOwned;
  use serde_json::{Map, Value};
  use tokens::tokens::media_files::MediaFileToken;

  use super::providers::accounts::ProviderAccounts;
  use super::providers::jobs::{JobStore, ProviderJob};
  use super::providers::submit::{self, SubmitError};
  use super::providers::worker::Worker;
  use super::providers::Provider;
  use super::session::{self, Credentials};
  use super::wire::{AudioModels, BatchMedia, CharactersPage, FileResponse, ImageModels, MediaFile, PromptResponse, SessionInfo, SessionJobs, UploadResponse, UserMediaList, VideoModels};
  use super::{CHARACTER_MODEL, EnqueueMeta, Event, LIBRARY_PAGE_SIZE, MODEL_LIST_ATTEMPTS, MODEL_LIST_BACKOFF, Modality, PROVIDER_COST_PATH, UploadedMedia, body_message, fal_cost_body, job_tokens};
  use crate::models;
  use crate::prompt_box::refs::RefKind;

  const USER_AGENT: &str = "storyteller-client/1.0";

  pub struct Backend {
    rt: tokio::runtime::Runtime,
    ctx: egui::Context,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    host: ApiHost,
    root: PathBuf,
    creds: Arc<RwLock<Option<Credentials>>>,
    http: reqwest::Client,
    device_login_cancel: Arc<AtomicBool>,
    accounts: ProviderAccounts,
    jobs: JobStore,
  }

  impl Backend {
    pub fn new(ctx: egui::Context) -> Self {
      let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(4).thread_name("fogbreak-hub-io").enable_all().build().expect("tokio runtime");
      let (tx, rx) = channel();
      let root = session::data_root();
      let host = session::api_host(&root);
      let creds = session::load(&root);
      let http = reqwest::Client::builder().user_agent(USER_AGENT).gzip(true).timeout(Duration::from_secs(60)).build().unwrap_or_default();
      let (accounts, jobs) = (ProviderAccounts::load(&root), JobStore::load(&root));
      let creds = Arc::new(RwLock::new(creds));
      rt.spawn(Worker { host: host.clone(), creds: creds.clone(), http: http.clone(), accounts: accounts.clone(), jobs: jobs.clone(), tx: tx.clone(), ctx: ctx.clone() }.run());
      Self { rt, ctx, tx, rx, host, root, creds, http, device_login_cancel: Arc::new(AtomicBool::new(false)), accounts, jobs }
    }

    pub fn runtime(&self) -> tokio::runtime::Handle {
      self.rt.handle().clone()
    }

    pub fn http(&self) -> reqwest::Client {
      self.http.clone()
    }

    pub fn data_root(&self) -> &Path {
      &self.root
    }

    pub fn drain(&self) -> Vec<Event> {
      self.rx.try_iter().collect()
    }

    pub fn refresh_session(&self) {
      let api = self.api();
      self.spawn(move |tx| async move {
        if api.creds.is_none() {
          let _ = tx.send(Event::Session(None));
          return;
        }
        match api.get_json::<SessionInfo>("/v1/session").await {
          Ok(info) if info.logged_in => {
            let _ = tx.send(Event::Session(info.user));
            api.send_credits(&tx).await;
          },
          Ok(_) => {
            let _ = tx.send(Event::Session(None));
          },
          Err(err) => {
            warn!("Session check failed: {err}");
            let _ = tx.send(Event::Session(None));
            let _ = tx.send(Event::Toast { error: true, message: format!("Couldn't reach Fogbreak Hub: {err}") });
          },
        }
      });
    }

    pub fn refresh_credits(&self) {
      let api = self.api();
      self.spawn(move |tx| async move { api.send_credits(&tx).await });
    }

    pub fn login_with_password(&self, username_or_email: String, password: String) {
      let (host, root, creds, ctx) = (self.host.clone(), self.root.clone(), self.creds.clone(), self.ctx.clone());
      let this = self.handle();
      self.spawn(move |tx| async move {
        let result = async {
          let client = LoginChallengeClient::new(&host).map_err(login_error)?;
          let (response, set_cookie) = client.password_login(&PasswordLoginRequest { username_or_email, password }).await.map_err(login_error)?;
          response.signed_session.filter(|s| !s.is_empty()).or_else(|| set_cookie.as_deref().and_then(session_from_set_cookie)).ok_or_else(|| "The server didn't return a session.".to_owned())
        }
        .await;
        match result {
          Ok(session) => {
            store_session(&root, &creds, session);
            this.refresh_session();
          },
          Err(message) => {
            let _ = tx.send(Event::LoginFailed(message));
          },
        }
        ctx.request_repaint();
      });
    }

    pub fn login_with_browser(&self) {
      self.device_login_cancel.store(false, Ordering::Relaxed);
      let (host, root, creds, cancel) = (self.host.clone(), self.root.clone(), self.creds.clone(), self.device_login_cancel.clone());
      let this = self.handle();
      self.spawn(move |tx| async move {
        let client = match LoginChallengeClient::new(&host) {
          Ok(c) => c,
          Err(err) => {
            let _ = tx.send(Event::LoginFailed(login_error(err)));
            return;
          },
        };
        let challenge = match client.create().await {
          Ok(c) => c,
          Err(err) => {
            let _ = tx.send(Event::LoginFailed(login_error(err)));
            return;
          },
        };
        let _ = tx.send(Event::DeviceLogin { url: challenge.verification_url.clone(), code: challenge.confirmation_code.clone() });
        if let Err(err) = open::that(&challenge.verification_url) {
          warn!("Couldn't open the browser: {err}");
        }
        let interval = Duration::from_secs(u64::from(challenge.poll_interval_seconds.max(1)));
        loop {
          tokio::time::sleep(interval).await;
          if cancel.load(Ordering::Relaxed) {
            return;
          }
          if chrono::Utc::now() > challenge.expires_at {
            let _ = tx.send(Event::LoginFailed("The sign-in code expired. Please try again.".to_owned()));
            return;
          }
          match client.poll(&challenge.device_token).await {
            Ok(result) => {
              let signed = result.response.maybe_signed_session.or_else(|| result.session_set_cookie.as_deref().and_then(session_from_set_cookie));
              if let Some(session) = signed {
                store_session(&root, &creds, session);
                this.refresh_session();
                return;
              }
              if result.response.status == LoginChallengeState::Failed {
                let _ = tx.send(Event::LoginFailed("Sign-in was declined or failed.".to_owned()));
                return;
              }
            },
            Err(err) => warn!("Login poll failed: {}", login_error(err)),
          }
        }
      });
    }

    pub fn cancel_browser_login(&self) {
      self.device_login_cancel.store(true, Ordering::Relaxed);
    }

    pub fn logout(&self) {
      let api = self.api();
      let (root, creds) = (self.root.clone(), self.creds.clone());
      if let Ok(mut c) = creds.write() {
        *c = None;
      }
      session::delete(&root);
      self.spawn(move |tx| async move {
        if let Some(set) = api.creds.as_ref() {
          let result: Result<Value, _> = basic_json_post_request(&api.host, "/v1/logout", Some(set), serde_json::json!({})).await;
          if let Err(err) = result {
            warn!("Logout request failed: {err}");
          }
        }
        let _ = tx.send(Event::Session(None));
      });
    }

    pub fn load_models(&self, modality: Modality) {
      let api = self.api();
      self.spawn(move |tx| async move {
        let mut last_err = String::new();
        for attempt in 0..MODEL_LIST_ATTEMPTS {
          if attempt > 0 {
            tokio::time::sleep(MODEL_LIST_BACKOFF).await;
          }
          let result = match modality {
            Modality::Image => api.get_json::<ImageModels>("/v1/omni_gen/models/image").await.map(|r| r.models.iter().map(models::image_model).collect::<Vec<_>>()),
            Modality::Video => api.get_json::<VideoModels>("/v1/omni_gen/models/video").await.map(|r| r.models.iter().map(models::video_model).collect::<Vec<_>>()),
            Modality::Audio => api.get_json::<AudioModels>("/v1/omni_gen/models/audio").await.map(|r| r.models.iter().map(models::audio_model).collect::<Vec<_>>()),
          };
          match result {
            Ok(models) => {
              let _ = tx.send(Event::Models(modality, Ok(models)));
              return;
            },
            Err(err) => last_err = err,
          }
        }
        error!("Listing {modality:?} models failed: {last_err}");
        let _ = tx.send(Event::Models(modality, Err(last_err)));
      });
    }

    pub fn estimate_cost(&self, modality: Modality, key: String, fields: Map<String, Value>) {
      let api = self.api();
      self.spawn(move |tx| async move {
        let path = format!("/v1/omni_gen/cost/{}", modality.path());
        let credits = match basic_json_post_request::<_, Value>(&api.host, &path, api.creds.as_ref(), fields).await {
          Ok(response) => response.get("cost_in_credits").and_then(Value::as_u64),
          Err(err) => {
            warn!("Cost estimate failed: {}", api_message(&err));
            None
          },
        };
        let _ = tx.send(Event::Cost { key, credits });
      });
    }

    pub fn generate(&self, modality: Modality, mut fields: Map<String, Value>, meta: EnqueueMeta) {
      let api = self.api();
      self.spawn(move |tx| async move {
        let Some(creds) = api.creds.as_ref() else {
          let _ = tx.send(Event::EnqueueFailed { modality, message: "Please sign in to generate.".to_owned(), status: None });
          return;
        };
        fields.entry("idempotency_token").or_insert_with(|| Value::String(uuid::Uuid::new_v4().to_string()));
        let path = format!("/v1/omni_gen/generate/{}", modality.path());
        match basic_json_post_request::<_, Value>(&api.host, &path, Some(creds), fields).await {
          Ok(response) => {
            let tokens = job_tokens(&response);
            if tokens.is_empty() {
              let _ = tx.send(Event::EnqueueFailed { modality, message: "The server didn't start a job.".to_owned(), status: None });
            } else {
              let _ = tx.send(Event::Enqueued { modality, job_tokens: tokens, meta });
            }
            api.send_credits(&tx).await;
          },
          Err(err) => {
            let _ = tx.send(Event::EnqueueFailed { modality, message: api_message(&err), status: api_status(&err) });
          },
        }
      });
    }

    pub fn generate_with_provider(&self, provider: Provider, fields: Map<String, Value>, image_urls: Vec<String>, meta: EnqueueMeta) {
      let (api, accounts, jobs) = (self.api(), self.accounts.clone(), self.jobs.clone());
      self.spawn(move |tx| async move {
        match submit::submit(&api.host, api.creds.as_ref(), &accounts, provider, &fields, image_urls).await {
          Ok(job) => {
            let id = job.id.clone();
            jobs.put(job);
            let _ = tx.send(Event::Enqueued { modality: Modality::Image, job_tokens: vec![id], meta });
          },
          Err(SubmitError::NeedsAccount(provider)) => {
            let _ = tx.send(Event::ProviderSetupNeeded(provider));
          },
          Err(SubmitError::Message(message)) => {
            let _ = tx.send(Event::EnqueueFailed { modality: Modality::Image, message, status: None });
          },
        }
      });
    }

    pub fn estimate_provider_cost(&self, provider: Provider, key: String, fields: Map<String, Value>) {
      match provider {
        Provider::Artcraft => self.estimate_cost(Modality::Image, key, fields),
        Provider::Midjourney => {
          let _ = self.tx.send(Event::Cost { key, credits: Some(0) });
        },
        Provider::Fal => {
          let api = self.api();
          self.spawn(move |tx| async move {
            let credits = match basic_json_post_request::<_, Value>(&api.host, PROVIDER_COST_PATH, None, fal_cost_body(&fields)).await {
              Ok(response) => response.get("cost_in_credits").and_then(Value::as_u64),
              Err(err) => {
                warn!("FAL cost estimate failed: {}", api_message(&err));
                None
              },
            };
            let _ = tx.send(Event::Cost { key, credits });
          });
        },
      }
    }

    pub fn accounts(&self) -> &ProviderAccounts {
      &self.accounts
    }

    pub fn provider_jobs(&self) -> Vec<ProviderJob> {
      self.jobs.all()
    }

    pub fn save_midjourney_login(&self, cookies: CookieStore) {
      let accounts = self.accounts.clone();
      self.spawn(move |tx| async move {
        if let Err(err) = accounts.midjourney.save_login(cookies).await {
          let _ = tx.send(Event::Toast { error: true, message: format!("Couldn't save the Midjourney login: {err}") });
        }
        let _ = tx.send(Event::ProviderAccountsChanged);
      });
    }

    pub fn disconnect_midjourney(&self) {
      let accounts = self.accounts.clone();
      self.spawn(move |tx| async move {
        if let Err(err) = accounts.midjourney.clear_credentials().await {
          let _ = tx.send(Event::Toast { error: true, message: format!("Couldn't disconnect Midjourney: {err}") });
        }
        let _ = tx.send(Event::ProviderAccountsChanged);
      });
    }

    pub fn upload_file(&self, ref_id: u64, kind: RefKind, path: PathBuf) {
      let api = self.api();
      self.spawn(move |tx| async move {
        let token = match kind {
          RefKind::Image => upload_image_media_file_from_file(UploadImageFromFileArgs { api_host: &api.host, maybe_creds: api.creds.as_ref(), path: &path, is_intermediate_system_file: true, maybe_prompt_token: None, maybe_generation_provider: None, maybe_batch_token: None }).await.map(|r| r.media_file_token.to_string()).map_err(|e| api_message(&e)),
          RefKind::Video => upload_video_media_file_from_file(UploadVideoFromFileArgs { api_host: &api.host, maybe_creds: api.creds.as_ref(), path: &path, maybe_prompt_token: None, maybe_generation_provider: None }).await.map(|r| r.media_file_token.to_string()).map_err(|e| api_error_message(&e)),
          RefKind::Audio => api.upload_audio(&path).await,
        };
        let result = match token {
          Ok(token) => Ok(api.describe_upload(token).await),
          Err(message) => Err(message),
        };
        let _ = tx.send(Event::Uploaded { ref_id, result });
      });
    }

    pub fn upload_bytes(&self, ref_id: u64, kind: RefKind, name: String, bytes: Vec<u8>) {
      let api = self.api();
      self.spawn(move |tx| async move {
        let token = api.upload_multipart_bytes(kind, &name, bytes).await;
        let result = match token {
          Ok(token) => Ok(api.describe_upload(token).await),
          Err(message) => Err(message),
        };
        let _ = tx.send(Event::Uploaded { ref_id, result });
      });
    }

    pub fn upload_png(&self, ref_id: u64, png: Vec<u8>) {
      let api = self.api();
      self.spawn(move |tx| async move {
        let result = upload_image_media_file_from_bytes(UploadImageBytesArgs { api_host: &api.host, maybe_creds: api.creds.as_ref(), image_bytes: png, image_type: ImageType::Png, is_intermediate_system_file: true, maybe_generation_provider: None }).await;
        let result = match result {
          Ok(r) => Ok(api.describe_upload(r.media_file_token.to_string()).await),
          Err(err) => Err(api_message(&err)),
        };
        let _ = tx.send(Event::Uploaded { ref_id, result });
      });
    }

    pub fn poll_jobs(&self) {
      let api = self.api();
      if api.creds.is_none() {
        return;
      }
      self.spawn(move |tx| async move {
        match api.get_json::<SessionJobs>("/v1/jobs/session").await {
          Ok(jobs) => {
            let _ = tx.send(Event::Jobs(jobs.jobs));
          },
          Err(err) => warn!("Polling jobs failed: {err}"),
        }
      });
    }

    pub fn load_library(&self, key: String, username: String, classes: &'static str, page: u32) {
      let api = self.api();
      self.spawn(move |tx| async move {
        let path = format!("/v1/media_files/list/user/{username}?filter_media_classes={classes}&page_size={LIBRARY_PAGE_SIZE}&page_index={page}");
        match api.get_json::<UserMediaList>(&path).await {
          Ok(list) => {
            let total_pages = list.pagination.map_or(0, |p| p.total_page_count);
            let items = list.results.into_iter().filter(|m| m.media_type != "scene_json" && m.origin_category != "upload").collect();
            let _ = tx.send(Event::LibraryPage { key, page, items, total_pages });
          },
          Err(message) => {
            let _ = tx.send(Event::LibraryFailed { key, message });
          },
        }
      });
    }

    pub fn load_batch(&self, job_token: String, batch_token: String) {
      let api = self.api();
      self.spawn(move |tx| async move {
        match api.get_json::<BatchMedia>(&format!("/v1/media_files/batch_gen_redux/{batch_token}")).await {
          Ok(batch) => {
            let _ = tx.send(Event::Batch { job_token, items: batch.media_files });
          },
          Err(err) => warn!("Loading batch {batch_token} failed: {err}"),
        }
      });
    }

    pub fn load_prompt(&self, prompt_token: String) {
      let api = self.api();
      self.spawn(move |tx| async move {
        match api.get_json::<PromptResponse>(&format!("/v1/prompts/{prompt_token}")).await {
          Ok(r) if r.success => {
            let _ = tx.send(Event::Prompt(r.prompt));
          },
          Ok(_) => warn!("Prompt {prompt_token} not found"),
          Err(err) => warn!("Loading prompt {prompt_token} failed: {err}"),
        }
      });
    }

    pub fn load_characters(&self) {
      let api = self.api();
      if api.creds.is_none() {
        return;
      }
      self.spawn(move |tx| async move {
        let mut all = Vec::new();
        let mut cursor: Option<i64> = None;
        loop {
          let path = match cursor {
            Some(c) => format!("/v1/characters/session?cursor={c}"),
            None => "/v1/characters/session".to_owned(),
          };
          match api.request_json::<CharactersPage>(reqwest::Method::GET, &path, None).await {
            Ok(page) => {
              all.extend(page.characters);
              match page.next_cursor {
                Some(next) if Some(next) != cursor => cursor = Some(next),
                _ => break,
              }
            },
            Err(err) => {
              warn!("Loading characters failed: {err}");
              break;
            },
          }
        }
        let _ = tx.send(Event::Characters(all));
      });
    }

    pub fn create_character(&self, image_token: String, name: String, description: Option<String>) {
      let api = self.api();
      self.spawn(move |tx| async move {
        let body = serde_json::json!({ "image_media_token": image_token, "model": CHARACTER_MODEL, "uuid_idempotency_token": uuid::Uuid::new_v4().to_string(), "character_name": name, "character_description": description });
        match api.request_json::<Value>(reqwest::Method::POST, "/v1/character/create", Some(body)).await {
          Ok(r) => match r.get("inference_job_token").and_then(Value::as_str) {
            Some(job) => {
              let _ = tx.send(Event::CharacterCreating { job_token: job.to_owned(), name });
            },
            None => {
              let _ = tx.send(Event::Toast { error: true, message: "Failed to create character".to_owned() });
            },
          },
          Err(err) => {
            let _ = tx.send(Event::Toast { error: true, message: if err.is_empty() { "Failed to create character".to_owned() } else { err } });
          },
        }
      });
    }

    pub fn edit_character(&self, token: String, name: String, description: String) {
      let api = self.api();
      self.spawn(move |tx| async move {
        let body = serde_json::json!({ "token": token, "updated_name": name, "updated_description": description });
        match api.request_json::<Value>(reqwest::Method::POST, "/v1/character/edit", Some(body)).await {
          Ok(_) => {
            let _ = tx.send(Event::Toast { error: false, message: "Character updated".to_owned() });
            let _ = tx.send(Event::CharacterChanged);
          },
          Err(err) => {
            let _ = tx.send(Event::Toast { error: true, message: format!("Failed to update character: {err}") });
          },
        }
      });
    }

    pub fn delete_character(&self, token: String, name: String) {
      let api = self.api();
      self.spawn(move |tx| async move {
        match api.request_json::<Value>(reqwest::Method::DELETE, &format!("/v1/character/{token}"), None).await {
          Ok(_) => {
            let _ = tx.send(Event::Toast { error: false, message: format!("Character \"{name}\" deleted") });
            let _ = tx.send(Event::CharacterChanged);
          },
          Err(err) => {
            let _ = tx.send(Event::Toast { error: true, message: format!("Failed to delete character: {err}") });
          },
        }
      });
    }

    pub fn delete_media(&self, token: String) {
      let api = self.api();
      self.spawn(move |tx| async move {
        match delete_media_file(&api.host, api.creds.as_ref(), &MediaFileToken::new(token.clone())).await {
          Ok(_) => {
            let _ = tx.send(Event::Deleted(token));
            let _ = tx.send(Event::Toast { error: false, message: "File deleted.".to_owned() });
          },
          Err(err) => {
            let _ = tx.send(Event::Toast { error: true, message: format!("Couldn't delete: {}", api_message(&err)) });
          },
        }
      });
    }

    pub fn save_url(&self, url: String, path: PathBuf) {
      let http = self.http.clone();
      self.spawn(move |tx| async move {
        let result = async {
          let bytes = http.get(&url).send().await.and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?;
          tokio::fs::write(&path, &bytes).await.map_err(|e| e.to_string())
        }
        .await;
        match result {
          Ok(()) => {
            let _ = tx.send(Event::Saved(path));
          },
          Err(err) => {
            let _ = tx.send(Event::Toast { error: true, message: format!("File download failed: {err}") });
          },
        }
      });
    }

    fn api(&self) -> Api {
      let creds = self.creds.read().ok().and_then(|c| c.as_ref().map(Credentials::to_set));
      Api { host: self.host.clone(), creds, http: self.http.clone() }
    }

    fn handle(&self) -> BackendHandle {
      BackendHandle { host: self.host.clone(), creds: self.creds.clone(), http: self.http.clone(), tx: self.tx.clone(), ctx: self.ctx.clone(), rt: self.rt.handle().clone() }
    }

    fn spawn<F, Fut>(&self, task: F)
    where
      F: FnOnce(Sender<Event>) -> Fut + Send + 'static,
      Fut: Future<Output = ()> + Send + 'static,
    {
      let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
      self.rt.spawn(async move {
        task(tx).await;
        ctx.request_repaint();
      });
    }
  }

  #[derive(Clone)]
  struct BackendHandle {
    host: ApiHost,
    creds: Arc<RwLock<Option<Credentials>>>,
    http: reqwest::Client,
    tx: Sender<Event>,
    ctx: egui::Context,
    rt: tokio::runtime::Handle,
  }

  impl BackendHandle {
    fn refresh_session(&self) {
      let creds = self.creds.read().ok().and_then(|c| c.as_ref().map(Credentials::to_set));
      let api = Api { host: self.host.clone(), creds, http: self.http.clone() };
      let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
      self.rt.spawn(async move {
        match api.get_json::<SessionInfo>("/v1/session").await {
          Ok(info) if info.logged_in => {
            let _ = tx.send(Event::Session(info.user));
            api.send_credits(&tx).await;
          },
          Ok(_) => {
            let _ = tx.send(Event::LoginFailed("The session wasn't accepted. Please try again.".to_owned()));
          },
          Err(err) => {
            let _ = tx.send(Event::LoginFailed(err));
          },
        }
        ctx.request_repaint();
      });
    }
  }

  struct Api {
    host: ApiHost,
    creds: Option<StorytellerCredentialSet>,
    http: reqwest::Client,
  }

  impl Api {
    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
      self.request_json(reqwest::Method::GET, path, None).await
    }

    async fn request_json<T: DeserializeOwned>(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<T, String> {
      let url = format!("{}{path}", self.host.to_api_hostname_and_scheme());
      let mut request = self.http.request(method, &url).header("Accept", "application/json").header("Origin", self.host.request_origin());
      if let Some(body) = body {
        request = request.json(&body);
      }
      if let Some(cookie) = self.creds.as_ref().and_then(StorytellerCredentialSet::maybe_as_cookie_header) {
        request = request.header("Cookie", cookie);
      }
      let response = request.send().await.map_err(|e| e.to_string())?;
      let status = response.status();
      let body = response.text().await.map_err(|e| e.to_string())?;
      if !status.is_success() {
        return Err(body_message(&body).unwrap_or_else(|| format!("HTTP {status}")));
      }
      serde_json::from_str(&body).map_err(|e| format!("Unexpected response from {path}: {e}"))
    }

    async fn send_credits(&self, tx: &Sender<Event>) {
      match get_session_credits(&self.host, self.creds.as_ref(), PaymentsNamespace::Artcraft).await {
        Ok(c) => {
          let _ = tx.send(Event::Credits(c.sum_total_credits));
        },
        Err(err) => warn!("Credits check failed: {}", api_message(&err)),
      }
    }

    async fn upload_audio(&self, path: &Path) -> Result<String, String> {
      let bytes = tokio::fs::read(path).await.map_err(|e| e.to_string())?;
      let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("audio").to_owned();
      self.upload_multipart_bytes(RefKind::Audio, &name, bytes).await
    }

    async fn upload_multipart_bytes(&self, kind: RefKind, name: &str, bytes: Vec<u8>) -> Result<String, String> {
      let route = match kind {
        RefKind::Image => "/v1/media_files/upload/image",
        RefKind::Video => "/v1/media_files/upload/new_video",
        RefKind::Audio => "/v1/media_files/upload/audio",
      };
      let mut form = reqwest::multipart::Form::new().text("uuid_idempotency_token", uuid::Uuid::new_v4().to_string());
      match kind {
        RefKind::Image => form = form.text("is_intermediate_system_file", "true"),
        RefKind::Video => {},
        RefKind::Audio => form = form.text("maybe_visibility", "public"),
      }
      form = form.part("file", reqwest::multipart::Part::bytes(bytes).file_name(name.to_owned()));
      let url = format!("{}{route}", self.host.to_api_hostname_and_scheme());
      let mut request = self.http.post(url).header("Accept", "application/json").header("Origin", self.host.request_origin()).multipart(form);
      if let Some(cookie) = self.creds.as_ref().and_then(StorytellerCredentialSet::maybe_as_cookie_header) {
        request = request.header("Cookie", cookie);
      }
      let response = request.send().await.map_err(|e| e.to_string())?;
      let status = response.status();
      let body = response.text().await.map_err(|e| e.to_string())?;
      if !status.is_success() {
        return Err(body_message(&body).unwrap_or_else(|| format!("HTTP {status}")));
      }
      let upload: UploadResponse = serde_json::from_str(&body).map_err(|e| e.to_string())?;
      if upload.media_file_token.is_empty() {
        return Err("The server didn't return a media token.".to_owned());
      }
      Ok(upload.media_file_token)
    }

    async fn describe_upload(&self, token: String) -> UploadedMedia {
      let info = self.get_json::<FileResponse>(&format!("/v1/media_files/file/{token}")).await.ok();
      let file = info.map(|r| r.media_file);
      UploadedMedia { duration_secs: file.as_ref().and_then(|f| f.maybe_duration_millis).map_or(0.0, |ms| ms as f32 / 1000.0), thumbnail: file.as_ref().and_then(|f| f.thumbnail(256)), full_url: file.as_ref().map(|f| f.media_links.cdn_url.clone()).filter(|u| !u.is_empty()), token }
    }
  }

  fn store_session(root: &Path, creds: &Arc<RwLock<Option<Credentials>>>, session: String) {
    let visitor = creds.read().ok().and_then(|c| c.as_ref().and_then(|c| c.visitor.clone()));
    let new = Credentials { session, visitor };
    session::save(root, &new);
    if let Ok(mut c) = creds.write() {
      *c = Some(new);
    }
  }

  pub fn session_from_set_cookie(header: &str) -> Option<String> {
    cookie::Cookie::parse(header).ok().filter(|c| c.name() == "session").map(|c| c.value().to_owned())
  }

  pub fn api_message(err: &StorytellerError) -> String {
    match err {
      StorytellerError::Api(api) => api_error_message(api),
      StorytellerError::Client(client) => format!("{client:?}"),
    }
  }

  fn api_status(err: &StorytellerError) -> Option<u16> {
    let StorytellerError::Api(api) = err else {
      return None;
    };
    Some(match api {
      ApiError::InvalidRequest(_) => 400,
      ApiError::Unauthorized(_) => 401,
      ApiError::PaymentRequired(_) => 402,
      ApiError::Forbidden(_) => 403,
      ApiError::NotFound(_) => 404,
      ApiError::TooManyRequests(_) => 429,
      ApiError::InternalServerError { .. } => 500,
      ApiError::UncategorizedBadResponseWithStatusAndBody { status_code, .. } => status_code.as_u16(),
      _ => return None,
    })
  }

  pub fn api_error_message(err: &ApiError) -> String {
    let body = match err {
      ApiError::InvalidRequest(b) | ApiError::Unauthorized(b) | ApiError::PaymentRequired(b) | ApiError::Forbidden(b) | ApiError::NotFound(b) | ApiError::TooManyRequests(b) => Some(b.as_str()),
      ApiError::InternalServerError { body, .. } | ApiError::UncategorizedBadResponseWithStatusAndBody { body, .. } => Some(body.as_str()),
      _ => None,
    };
    let fallback = match err {
      ApiError::Unauthorized(_) | ApiError::Forbidden(_) => "Please sign in to Fogbreak Hub.".to_owned(),
      ApiError::PaymentRequired(_) => "You don't have enough credits for this.".to_owned(),
      ApiError::TooManyRequests(_) => "Too many requests. Please wait a moment.".to_owned(),
      ApiError::NetworkError(_) | ApiError::Timeout(_) => "Couldn't reach Fogbreak Hub. Check your connection.".to_owned(),
      other => format!("{other:?}"),
    };
    body.and_then(body_message).unwrap_or(fallback)
  }

  fn login_error(err: LoginChallengeClientError) -> String {
    err.maybe_rejection.and_then(|r| r.error_message).unwrap_or_else(|| err.message.to_owned())
  }

  #[cfg(test)]
  mod tests {
    use super::*;

    #[test]
    fn error_messages_decode_gracefully() {
      assert_eq!(api_error_message(&ApiError::PaymentRequired("{}".into())), "You don't have enough credits for this.");
      assert_eq!(session_from_set_cookie("session=abc.def; Path=/; Secure").as_deref(), Some("abc.def"));
      assert_eq!(session_from_set_cookie("visitor=x; Path=/"), None);
    }
  }
}

#[cfg(not(target_arch = "wasm32"))]
pub use desktop::{Backend, api_message};

#[cfg(target_arch = "wasm32")]
mod web {
  use std::future::Future;
  use std::path::{Path, PathBuf};
  use std::sync::atomic::{AtomicBool, Ordering};
  use std::sync::mpsc::{Receiver, Sender, channel};
  use std::sync::{Arc, RwLock};

  use log::{error, warn};
  use serde_json::{Map, Value};

  use super::providers::accounts::ProviderAccounts;
  use super::providers::jobs::{JobStore, ProviderJob};
  use super::providers::Provider;
  use super::session::{self, Credentials};
  use super::web_http::{fetch_bytes, fetch_json, mime_for_name, post_multipart_upload, sleep_ms, trigger_download};
  use super::wire::{AudioModels, BatchMedia, CharactersPage, CreateLoginChallengeResponse, FileResponse, ImageModels, LoginChallengeState, MediaFile, PasswordAuthResponse, PollLoginChallengeResponse, PromptResponse, SessionCredits, SessionInfo, SessionJobs, UploadResponse, UserMediaList, VideoModels};
  use super::{CHARACTER_MODEL, EnqueueMeta, Event, LIBRARY_PAGE_SIZE, MODEL_LIST_ATTEMPTS, Modality, PROVIDER_COST_PATH, UploadedMedia, fal_cost_body, job_tokens};
  use crate::models;
  use crate::prompt_box::refs::RefKind;

  pub struct Backend {
    ctx: egui::Context,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    host: String,
    root: PathBuf,
    creds: Arc<RwLock<Option<Credentials>>>,
    device_login_cancel: Arc<AtomicBool>,
    accounts: ProviderAccounts,
    jobs: JobStore,
  }

  impl Backend {
    pub fn new(ctx: egui::Context) -> Self {
      let (tx, rx) = channel();
      let root = session::data_root();
      let host = session::api_host(&root);
      let creds = Arc::new(RwLock::new(session::load(&root)));
      let (accounts, jobs) = (ProviderAccounts::load(&root), JobStore::load(&root));
      Self { ctx, tx, rx, host, root, creds, device_login_cancel: Arc::new(AtomicBool::new(false)), accounts, jobs }
    }

    pub fn data_root(&self) -> &Path {
      &self.root
    }

    pub fn drain(&self) -> Vec<Event> {
      self.rx.try_iter().collect()
    }

    pub fn refresh_session(&self) {
      let (host, has_saved) = (self.host.clone(), self.has_creds());
      self.spawn(move |tx| async move {
        match fetch_json::<SessionInfo>("GET", &format!("{host}/v1/session"), None, true).await {
          Ok(info) if info.logged_in => {
            let _ = tx.send(Event::Session(info.user));
            send_credits(&host, &tx).await;
          },
          Ok(_) => {
            let _ = tx.send(Event::Session(None));
          },
          Err((_, err)) => {
            warn!("Session check failed: {err}");
            let _ = tx.send(Event::Session(None));
            if has_saved {
              let _ = tx.send(Event::Toast { error: true, message: format!("Couldn't reach Fogbreak Hub: {err}") });
            }
          },
        }
      });
    }

    pub fn refresh_credits(&self) {
      let host = self.host.clone();
      self.spawn(move |tx| async move { send_credits(&host, &tx).await });
    }

    pub fn login_with_password(&self, username_or_email: String, password: String) {
      let (host, root, creds, ctx) = (self.host.clone(), self.root.clone(), self.creds.clone(), self.ctx.clone());
      self.spawn(move |tx| async move {
        let body = serde_json::json!({ "username_or_email": username_or_email, "password": password });
        match fetch_json::<PasswordAuthResponse>("POST", &format!("{host}/v1/login"), Some(&body), true).await {
          Ok(resp) if resp.success => {
            if let Some(session) = resp.signed_session.filter(|s| !s.is_empty()) {
              store_session(&root, &creds, session);
            }
            refresh_session_inner(&host, &tx).await;
          },
          Ok(resp) => {
            let _ = tx.send(Event::LoginFailed(resp.maybe_error_message.unwrap_or_else(|| "Sign-in failed.".to_owned())));
          },
          Err((_, msg)) => {
            let _ = tx.send(Event::LoginFailed(msg));
          },
        }
        ctx.request_repaint();
      });
    }

    pub fn login_with_browser(&self) {
      self.device_login_cancel.store(false, Ordering::Relaxed);
      let (host, root, creds, cancel, ctx) = (self.host.clone(), self.root.clone(), self.creds.clone(), self.device_login_cancel.clone(), self.ctx.clone());
      self.spawn(move |tx| async move {
        let empty = serde_json::json!({});
        let challenge = match fetch_json::<CreateLoginChallengeResponse>("POST", &format!("{host}/v1/login_challenges/create"), Some(&empty), true).await {
          Ok(c) if c.success => c,
          Ok(_) => {
            let _ = tx.send(Event::LoginFailed("Couldn't start browser sign-in.".to_owned()));
            return;
          },
          Err((_, msg)) => {
            let _ = tx.send(Event::LoginFailed(msg));
            return;
          },
        };
        let _ = tx.send(Event::DeviceLogin { url: challenge.verification_url.clone(), code: challenge.confirmation_code.clone() });
        ctx.open_url(egui::OpenUrl::new_tab(&challenge.verification_url));
        let interval_ms = i32::try_from(challenge.poll_interval_seconds.max(1).saturating_mul(1000)).unwrap_or(2000);
        let expires_at = chrono::DateTime::parse_from_rfc3339(&challenge.expires_at).ok();
        loop {
          sleep_ms(interval_ms).await;
          if cancel.load(Ordering::Relaxed) {
            return;
          }
          if expires_at.is_some_and(|exp| chrono::Utc::now() > exp) {
            let _ = tx.send(Event::LoginFailed("The sign-in code expired. Please try again.".to_owned()));
            return;
          }
          let poll_body = serde_json::json!({ "device_token": challenge.device_token });
          match fetch_json::<PollLoginChallengeResponse>("POST", &format!("{host}/v1/login_challenges/poll"), Some(&poll_body), true).await {
            Ok(res) if res.status == LoginChallengeState::Completed || res.maybe_signed_session.is_some() => {
              if let Some(session) = res.maybe_signed_session.filter(|s| !s.is_empty()) {
                store_session(&root, &creds, session);
              }
              refresh_session_inner(&host, &tx).await;
              return;
            },
            Ok(res) if matches!(res.status, LoginChallengeState::Failed | LoginChallengeState::Expired) => {
              let _ = tx.send(Event::LoginFailed("Sign-in was declined or expired.".to_owned()));
              return;
            },
            Ok(_) => {},
            Err((_, err)) => warn!("Login poll failed: {err}"),
          }
        }
      });
    }

    pub fn cancel_browser_login(&self) {
      self.device_login_cancel.store(true, Ordering::Relaxed);
    }

    pub fn logout(&self) {
      let host = self.host.clone();
      if let Ok(mut c) = self.creds.write() {
        *c = None;
      }
      session::delete(&self.root);
      self.spawn(move |tx| async move {
        let empty = serde_json::json!({});
        if let Err((_, err)) = fetch_json::<Value>("POST", &format!("{host}/v1/logout"), Some(&empty), true).await {
          warn!("Logout request failed: {err}");
        }
        let _ = tx.send(Event::Session(None));
      });
    }

    pub fn load_models(&self, modality: Modality) {
      let host = self.host.clone();
      self.spawn(move |tx| async move {
        let mut last_err = String::new();
        for attempt in 0..MODEL_LIST_ATTEMPTS {
          if attempt > 0 {
            sleep_ms(250).await;
          }
          let result = match modality {
            Modality::Image => fetch_json::<ImageModels>("GET", &format!("{host}/v1/omni_gen/models/image"), None, false).await.map(|r| r.models.iter().map(models::image_model).collect::<Vec<_>>()),
            Modality::Video => fetch_json::<VideoModels>("GET", &format!("{host}/v1/omni_gen/models/video"), None, false).await.map(|r| r.models.iter().map(models::video_model).collect::<Vec<_>>()),
            Modality::Audio => fetch_json::<AudioModels>("GET", &format!("{host}/v1/omni_gen/models/audio"), None, false).await.map(|r| r.models.iter().map(models::audio_model).collect::<Vec<_>>()),
          };
          match result {
            Ok(list) => {
              let _ = tx.send(Event::Models(modality, Ok(list)));
              return;
            },
            Err((_, err)) => last_err = err,
          }
        }
        error!("Listing {modality:?} models failed: {last_err}");
        let _ = tx.send(Event::Models(modality, Err(last_err)));
      });
    }

    pub fn estimate_cost(&self, modality: Modality, key: String, fields: Map<String, Value>) {
      let host = self.host.clone();
      let body = Value::Object(fields);
      self.spawn(move |tx| async move {
        let url = format!("{host}/v1/omni_gen/cost/{}", modality.path());
        let credits = match fetch_json::<Value>("POST", &url, Some(&body), true).await {
          Ok(response) => response.get("cost_in_credits").and_then(Value::as_u64),
          Err((_, err)) => {
            warn!("Cost estimate failed: {err}");
            None
          },
        };
        let _ = tx.send(Event::Cost { key, credits });
      });
    }

    pub fn generate(&self, modality: Modality, mut fields: Map<String, Value>, meta: EnqueueMeta) {
      let host = self.host.clone();
      fields.entry("idempotency_token").or_insert_with(|| Value::String(uuid::Uuid::new_v4().to_string()));
      let body = Value::Object(fields);
      self.spawn(move |tx| async move {
        let url = format!("{host}/v1/omni_gen/generate/{}", modality.path());
        match fetch_json::<Value>("POST", &url, Some(&body), true).await {
          Ok(response) => {
            let tokens = job_tokens(&response);
            if tokens.is_empty() {
              let _ = tx.send(Event::EnqueueFailed { modality, message: "The server didn't start a job.".to_owned(), status: None });
            } else {
              let _ = tx.send(Event::Enqueued { modality, job_tokens: tokens, meta });
            }
            send_credits(&host, &tx).await;
          },
          Err((status, message)) => {
            let _ = tx.send(Event::EnqueueFailed { modality, message, status });
          },
        }
      });
    }

    pub fn generate_with_provider(&self, provider: Provider, fields: Map<String, Value>, _image_urls: Vec<String>, meta: EnqueueMeta) {
      match provider {
        Provider::Artcraft => self.generate(Modality::Image, fields, meta),
        Provider::Midjourney => {
          let _ = self.tx.send(Event::ProviderSetupNeeded(Provider::Midjourney));
        },
        Provider::Fal => {
          if self.accounts.fal_key().is_none() {
            let _ = self.tx.send(Event::ProviderSetupNeeded(Provider::Fal));
          } else {
            let _ = self.tx.send(Event::EnqueueFailed { modality: Modality::Image, message: "Direct FAL provider routing runs on desktop; select Fogbreak Hub provider in the browser.".to_owned(), status: None });
          }
        },
      }
    }

    pub fn estimate_provider_cost(&self, provider: Provider, key: String, fields: Map<String, Value>) {
      match provider {
        Provider::Artcraft => self.estimate_cost(Modality::Image, key, fields),
        Provider::Midjourney => {
          let _ = self.tx.send(Event::Cost { key, credits: Some(0) });
        },
        Provider::Fal => {
          let host = self.host.clone();
          let body = fal_cost_body(&fields);
          self.spawn(move |tx| async move {
            let credits = match fetch_json::<Value>("POST", &format!("{host}{PROVIDER_COST_PATH}"), Some(&body), false).await {
              Ok(response) => response.get("cost_in_credits").and_then(Value::as_u64),
              Err((_, err)) => {
                warn!("FAL cost estimate failed: {err}");
                None
              },
            };
            let _ = tx.send(Event::Cost { key, credits });
          });
        },
      }
    }

    pub fn accounts(&self) -> &ProviderAccounts {
      &self.accounts
    }

    pub fn provider_jobs(&self) -> Vec<ProviderJob> {
      self.jobs.all()
    }

    pub fn upload_bytes(&self, ref_id: u64, kind: RefKind, name: String, bytes: Vec<u8>) {
      let host = self.host.clone();
      self.spawn(move |tx| async move {
        let route = match kind {
          RefKind::Image => "/v1/media_files/upload/image",
          RefKind::Video => "/v1/media_files/upload/new_video",
          RefKind::Audio => "/v1/media_files/upload/audio",
        };
        let token = uuid::Uuid::new_v4().to_string();
        let extra: Vec<(&str, &str)> = match kind {
          RefKind::Image => vec![("uuid_idempotency_token", &token), ("is_intermediate_system_file", "true")],
          RefKind::Video => vec![("uuid_idempotency_token", &token)],
          RefKind::Audio => vec![("uuid_idempotency_token", &token), ("maybe_visibility", "public")],
        };
        let result = match post_multipart_upload(&format!("{host}{route}"), &bytes, &name, mime_for_name(&name), &extra, true).await {
          Ok(body) => match serde_json::from_str::<UploadResponse>(&body) {
            Ok(up) if !up.media_file_token.is_empty() => Ok(describe_upload(&host, up.media_file_token).await),
            Ok(_) => Err("The server didn't return a media token.".to_owned()),
            Err(err) => Err(err.to_string()),
          },
          Err(err) => Err(err),
        };
        let _ = tx.send(Event::Uploaded { ref_id, result });
      });
    }

    pub fn upload_png(&self, ref_id: u64, png: Vec<u8>) {
      self.upload_bytes(ref_id, RefKind::Image, "pasted.png".to_owned(), png);
    }

    pub fn poll_jobs(&self) {
      if !self.has_creds() {
        return;
      }
      let host = self.host.clone();
      self.spawn(move |tx| async move {
        match fetch_json::<SessionJobs>("GET", &format!("{host}/v1/jobs/session"), None, true).await {
          Ok(jobs) => {
            let _ = tx.send(Event::Jobs(jobs.jobs));
          },
          Err((_, err)) => warn!("Polling jobs failed: {err}"),
        }
      });
    }

    pub fn load_library(&self, key: String, username: String, classes: &'static str, page: u32) {
      let host = self.host.clone();
      self.spawn(move |tx| async move {
        let url = format!("{host}/v1/media_files/list/user/{username}?filter_media_classes={classes}&page_size={LIBRARY_PAGE_SIZE}&page_index={page}");
        match fetch_json::<UserMediaList>("GET", &url, None, true).await {
          Ok(list) => {
            let total_pages = list.pagination.map_or(0, |p| p.total_page_count);
            let items = list.results.into_iter().filter(|m| m.media_type != "scene_json" && m.origin_category != "upload").collect();
            let _ = tx.send(Event::LibraryPage { key, page, items, total_pages });
          },
          Err((_, message)) => {
            let _ = tx.send(Event::LibraryFailed { key, message });
          },
        }
      });
    }

    pub fn load_batch(&self, job_token: String, batch_token: String) {
      let host = self.host.clone();
      self.spawn(move |tx| async move {
        match fetch_json::<BatchMedia>("GET", &format!("{host}/v1/media_files/batch_gen_redux/{batch_token}"), None, true).await {
          Ok(batch) => {
            let _ = tx.send(Event::Batch { job_token, items: batch.media_files });
          },
          Err((_, err)) => warn!("Loading batch {batch_token} failed: {err}"),
        }
      });
    }

    pub fn load_prompt(&self, prompt_token: String) {
      let host = self.host.clone();
      self.spawn(move |tx| async move {
        match fetch_json::<PromptResponse>("GET", &format!("{host}/v1/prompts/{prompt_token}"), None, true).await {
          Ok(r) if r.success => {
            let _ = tx.send(Event::Prompt(r.prompt));
          },
          Ok(_) => warn!("Prompt {prompt_token} not found"),
          Err((_, err)) => warn!("Loading prompt {prompt_token} failed: {err}"),
        }
      });
    }

    pub fn load_characters(&self) {
      if !self.has_creds() {
        return;
      }
      let host = self.host.clone();
      self.spawn(move |tx| async move {
        let mut all = Vec::new();
        let mut cursor: Option<i64> = None;
        loop {
          let url = match cursor {
            Some(c) => format!("{host}/v1/characters/session?cursor={c}"),
            None => format!("{host}/v1/characters/session"),
          };
          match fetch_json::<CharactersPage>("GET", &url, None, true).await {
            Ok(page) => {
              all.extend(page.characters);
              match page.next_cursor {
                Some(next) if Some(next) != cursor => cursor = Some(next),
                _ => break,
              }
            },
            Err((_, err)) => {
              warn!("Loading characters failed: {err}");
              break;
            },
          }
        }
        let _ = tx.send(Event::Characters(all));
      });
    }

    pub fn create_character(&self, image_token: String, name: String, description: Option<String>) {
      let host = self.host.clone();
      self.spawn(move |tx| async move {
        let body = serde_json::json!({ "image_media_token": image_token, "model": CHARACTER_MODEL, "uuid_idempotency_token": uuid::Uuid::new_v4().to_string(), "character_name": name, "character_description": description });
        match fetch_json::<Value>("POST", &format!("{host}/v1/character/create"), Some(&body), true).await {
          Ok(r) => match r.get("inference_job_token").and_then(Value::as_str) {
            Some(job) => {
              let _ = tx.send(Event::CharacterCreating { job_token: job.to_owned(), name });
            },
            None => {
              let _ = tx.send(Event::Toast { error: true, message: "Failed to create character".to_owned() });
            },
          },
          Err((_, err)) => {
            let _ = tx.send(Event::Toast { error: true, message: if err.is_empty() { "Failed to create character".to_owned() } else { err } });
          },
        }
      });
    }

    pub fn edit_character(&self, token: String, name: String, description: String) {
      let host = self.host.clone();
      self.spawn(move |tx| async move {
        let body = serde_json::json!({ "token": token, "updated_name": name, "updated_description": description });
        match fetch_json::<Value>("POST", &format!("{host}/v1/character/edit"), Some(&body), true).await {
          Ok(_) => {
            let _ = tx.send(Event::Toast { error: false, message: "Character updated".to_owned() });
            let _ = tx.send(Event::CharacterChanged);
          },
          Err((_, err)) => {
            let _ = tx.send(Event::Toast { error: true, message: format!("Failed to update character: {err}") });
          },
        }
      });
    }

    pub fn delete_character(&self, token: String, name: String) {
      let host = self.host.clone();
      self.spawn(move |tx| async move {
        match fetch_json::<Value>("DELETE", &format!("{host}/v1/character/{token}"), None, true).await {
          Ok(_) => {
            let _ = tx.send(Event::Toast { error: false, message: format!("Character \"{name}\" deleted") });
            let _ = tx.send(Event::CharacterChanged);
          },
          Err((_, err)) => {
            let _ = tx.send(Event::Toast { error: true, message: format!("Failed to delete character: {err}") });
          },
        }
      });
    }

    pub fn delete_media(&self, token: String) {
      let host = self.host.clone();
      self.spawn(move |tx| async move {
        let body = serde_json::json!({ "set_delete": true, "as_mod": Value::Null });
        match fetch_json::<Value>("DELETE", &format!("{host}/v1/media_files/file/{token}"), Some(&body), true).await {
          Ok(_) => {
            let _ = tx.send(Event::Deleted(token));
            let _ = tx.send(Event::Toast { error: false, message: "File deleted.".to_owned() });
          },
          Err((_, err)) => {
            let _ = tx.send(Event::Toast { error: true, message: format!("Couldn't delete: {err}") });
          },
        }
      });
    }

    pub fn save_url(&self, url: String, path: PathBuf) {
      let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("download.bin").to_owned();
      self.spawn(move |tx| async move {
        let result = match fetch_bytes(&url).await {
          Ok(bytes) => trigger_download(&name, &bytes),
          Err(err) => Err(err),
        };
        match result {
          Ok(()) => {
            let _ = tx.send(Event::Saved(path));
          },
          Err(err) => {
            let _ = tx.send(Event::Toast { error: true, message: format!("File download failed: {err}") });
          },
        }
      });
    }

    fn has_creds(&self) -> bool {
      self.creds.read().ok().is_some_and(|c| c.is_some())
    }

    fn spawn<F, Fut>(&self, task: F)
    where
      F: FnOnce(Sender<Event>) -> Fut + 'static,
      Fut: Future<Output = ()> + 'static,
    {
      let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
      wasm_bindgen_futures::spawn_local(async move {
        task(tx).await;
        ctx.request_repaint();
      });
    }
  }

  async fn refresh_session_inner(host: &str, tx: &Sender<Event>) {
    match fetch_json::<SessionInfo>("GET", &format!("{host}/v1/session"), None, true).await {
      Ok(info) if info.logged_in => {
        let _ = tx.send(Event::Session(info.user));
        send_credits(host, tx).await;
      },
      Ok(_) => {
        let _ = tx.send(Event::LoginFailed("The session wasn't accepted. Please try again.".to_owned()));
      },
      Err((_, err)) => {
        let _ = tx.send(Event::LoginFailed(err));
      },
    }
  }

  async fn send_credits(host: &str, tx: &Sender<Event>) {
    match fetch_json::<SessionCredits>("GET", &format!("{host}/v1/credits/namespace/artcraft"), None, true).await {
      Ok(c) => {
        if let Some(total) = c.sum_total_credits {
          let _ = tx.send(Event::Credits(u64::try_from(total.max(0)).unwrap_or(0)));
        }
      },
      Err((_, err)) => warn!("Credits check failed: {err}"),
    }
  }

  async fn describe_upload(host: &str, token: String) -> UploadedMedia {
    let info = fetch_json::<FileResponse>("GET", &format!("{host}/v1/media_files/file/{token}"), None, true).await.ok();
    let file = info.map(|r| r.media_file);
    UploadedMedia { duration_secs: file.as_ref().and_then(|f| f.maybe_duration_millis).map_or(0.0, |ms| ms as f32 / 1000.0), thumbnail: file.as_ref().and_then(|f| f.thumbnail(256)), full_url: file.as_ref().map(|f| f.media_links.cdn_url.clone()).filter(|u| !u.is_empty()), token }
  }

  fn store_session(root: &Path, creds: &Arc<RwLock<Option<Credentials>>>, session: String) {
    let visitor = creds.read().ok().and_then(|c| c.as_ref().and_then(|c| c.visitor.clone()));
    let new = Credentials { session, visitor };
    session::save(root, &new);
    if let Ok(mut c) = creds.write() {
      *c = Some(new);
    }
  }
}

#[cfg(target_arch = "wasm32")]
pub use web::Backend;

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn collects_every_job_token_once() {
    let response = serde_json::json!({ "success": true, "inference_job_token": "j1", "all_job_tokens": ["j1", "j2", "j3"] });
    assert_eq!(job_tokens(&response), ["j1", "j2", "j3"]);
    assert_eq!(job_tokens(&serde_json::json!({ "inference_job_token": "only" })), ["only"]);
  }

  #[test]
  fn fal_costs_count_references_as_the_mode() {
    let fields = serde_json::json!({ "model": "flux_1_dev", "prompt": "x", "aspect_ratio": "square", "image_batch_count": 2, "image_media_tokens": ["mf_estimate_0"] });
    let body = fal_cost_body(fields.as_object().unwrap());
    assert_eq!(body, serde_json::json!({ "model": "flux_1_dev", "provider": "fal", "generation_mode": { "type": "image_edit", "count": 1 }, "aspect_ratio": "square", "resolution": null, "quality": null, "image_batch_count": 2 }));
  }

  #[test]
  fn reads_the_servers_error_message() {
    assert_eq!(body_message(r#"{"success":false,"error_message":"Not enough credits"}"#).as_deref(), Some("Not enough credits"));
    assert_eq!(body_message(r#"{"message":"Origin is not allowed"}"#).as_deref(), Some("Origin is not allowed"));
    assert_eq!(body_message("<html>502</html>"), None);
  }
}
