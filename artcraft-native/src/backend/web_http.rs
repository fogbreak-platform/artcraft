//! Shared browser HTTP, download, and async timer helpers for `wasm32`.

#![cfg(target_arch = "wasm32")]

use js_sys::{Array, ArrayBuffer, Promise, Uint8Array};
use serde::de::DeserializeOwned;
use serde_json::Value;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, BlobPropertyBag, FormData, Headers, HtmlAnchorElement, Request, RequestCredentials, RequestInit, Response, Url};

pub fn js_err(err: JsValue) -> String {
  err.as_string().unwrap_or_else(|| format!("{err:?}"))
}

pub async fn sleep_ms(ms: i32) {
  let promise = Promise::new(&mut |resolve, _| {
    if let Some(window) = web_sys::window() {
      let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    } else {
      let _ = resolve.call0(&JsValue::NULL);
    }
  });
  let _ = JsFuture::from(promise).await;
}

pub async fn fetch_response(method: &str, url: &str, json_body: Option<&str>, include_creds: bool) -> Result<(u16, String), String> {
  let window = web_sys::window().ok_or_else(|| "no window".to_owned())?;
  let opts = RequestInit::new();
  opts.set_method(method);
  opts.set_credentials(if include_creds { RequestCredentials::Include } else { RequestCredentials::Omit });
  let headers = Headers::new().map_err(js_err)?;
  let _ = headers.set("Accept", "application/json");
  if let Some(body) = json_body {
    let _ = headers.set("Content-Type", "application/json");
    opts.set_body(&JsValue::from_str(body));
  }
  opts.set_headers(&headers);
  let request = Request::new_with_str_and_init(url, &opts).map_err(js_err)?;
  let resp_val = JsFuture::from(window.fetch_with_request(&request)).await.map_err(js_err)?;
  let resp: Response = resp_val.dyn_into().map_err(js_err)?;
  let status = resp.status();
  let text_promise = resp.text().map_err(js_err)?;
  let text = JsFuture::from(text_promise).await.map_err(js_err)?.as_string().unwrap_or_default();
  Ok((status, text))
}

pub async fn fetch_json<T: DeserializeOwned>(method: &str, url: &str, body: Option<&Value>, include_creds: bool) -> Result<T, (Option<u16>, String)> {
  let encoded = body.and_then(|v| serde_json::to_string(v).ok());
  let (status, text) = fetch_response(method, url, encoded.as_deref(), include_creds).await.map_err(|e| (None, e))?;
  if !(200..300).contains(&status) {
    let msg = extract_error_message(&text).unwrap_or_else(|| match status {
      401 | 403 => "Please sign in to Fogbreak Hub.".to_owned(),
      402 => "You don't have enough credits for this.".to_owned(),
      429 => "Too many requests. Please wait a moment.".to_owned(),
      _ => format!("HTTP {status}"),
    });
    return Err((Some(status), msg));
  }
  serde_json::from_str(&text).map_err(|e| (Some(status), format!("Unexpected response from server: {e}")))
}

pub fn extract_error_message(body: &str) -> Option<String> {
  let v: Value = serde_json::from_str(body).ok()?;
  v.get("error_message").or_else(|| v.get("message")).and_then(Value::as_str).map(str::to_owned)
}

pub async fn post_multipart_upload(url: &str, file_bytes: &[u8], file_name: &str, mime: &str, fields: &[(&str, &str)], include_creds: bool) -> Result<String, String> {
  let window = web_sys::window().ok_or_else(|| "no window".to_owned())?;
  let form = FormData::new().map_err(js_err)?;
  for (key, val) in fields {
    form.append_with_str(key, val).map_err(js_err)?;
  }
  let parts = Array::of1(&Uint8Array::from(file_bytes));
  let bag = BlobPropertyBag::new();
  bag.set_type(mime);
  let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &bag).map_err(js_err)?;
  form.append_with_blob_and_filename("file", &blob, file_name).map_err(js_err)?;

  let opts = RequestInit::new();
  opts.set_method("POST");
  opts.set_body(&form);
  opts.set_credentials(if include_creds { RequestCredentials::Include } else { RequestCredentials::Omit });
  let headers = Headers::new().map_err(js_err)?;
  let _ = headers.set("Accept", "application/json");
  opts.set_headers(&headers);
  let request = Request::new_with_str_and_init(url, &opts).map_err(js_err)?;
  let resp: Response = JsFuture::from(window.fetch_with_request(&request)).await.map_err(js_err)?.dyn_into().map_err(js_err)?;
  let status = resp.status();
  let text = JsFuture::from(resp.text().map_err(js_err)?).await.map_err(js_err)?.as_string().unwrap_or_default();
  if !(200..300).contains(&status) {
    return Err(extract_error_message(&text).unwrap_or_else(|| format!("HTTP {status}")));
  }
  Ok(text)
}

pub async fn fetch_array_buffer(url: &str) -> Result<ArrayBuffer, String> {
  let window = web_sys::window().ok_or_else(|| "no window".to_owned())?;
  let opts = RequestInit::new();
  opts.set_method("GET");
  opts.set_credentials(RequestCredentials::Omit);
  let request = Request::new_with_str_and_init(url, &opts).map_err(js_err)?;
  let resp: Response = JsFuture::from(window.fetch_with_request(&request)).await.map_err(js_err)?.dyn_into().map_err(js_err)?;
  if !resp.ok() {
    return Err(format!("HTTP {}", resp.status()));
  }
  let buf = JsFuture::from(resp.array_buffer().map_err(js_err)?).await.map_err(js_err)?;
  buf.dyn_into().map_err(js_err)
}

pub async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
  let buf = fetch_array_buffer(url).await?;
  Ok(Uint8Array::new(&buf).to_vec())
}

pub fn trigger_download(name: &str, bytes: &[u8]) -> Result<(), String> {
  let window = web_sys::window().ok_or_else(|| "no window".to_owned())?;
  let document = window.document().ok_or_else(|| "no document".to_owned())?;
  let parts = Array::of1(&Uint8Array::from(bytes));
  let bag = BlobPropertyBag::new();
  bag.set_type(mime_for_name(name));
  let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &bag).map_err(js_err)?;
  let url = Url::create_object_url_with_blob(&blob).map_err(js_err)?;
  let anchor: HtmlAnchorElement = document.create_element("a").map_err(js_err)?.dyn_into().map_err(|_| "anchor cast failed".to_owned())?;
  anchor.set_href(&url);
  anchor.set_download(name);
  anchor.style().set_property("display", "none").map_err(js_err)?;
  document.body().ok_or_else(|| "no body".to_owned())?.append_child(&anchor).map_err(js_err)?;
  anchor.click();
  anchor.remove();
  let revoke = Closure::once_into_js(move || {
    let _ = Url::revoke_object_url(&url);
  });
  let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000);
  Ok(())
}

pub fn mime_for_name(name: &str) -> &'static str {
  let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
  match ext.as_str() {
    "png" => "image/png",
    "jpg" | "jpeg" => "image/jpeg",
    "webp" => "image/webp",
    "gif" => "image/gif",
    "mp4" => "video/mp4",
    "webm" => "video/webm",
    "mov" => "video/quicktime",
    "mp3" => "audio/mpeg",
    "wav" => "audio/wav",
    "ogg" => "audio/ogg",
    "flac" => "audio/flac",
    "json" => "application/json",
    _ => "application/octet-stream",
  }
}
