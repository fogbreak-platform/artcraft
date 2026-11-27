#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};

use artcraft_native::app::ArtcraftApp;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use js_sys::{Array, Function, JSON, Object, Promise, Reflect, Uint8Array};
use serde_json::{Value, json};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Blob, BlobPropertyBag, CustomEvent, CustomEventInit, HtmlAnchorElement, HtmlCanvasElement, Url};

use crate::theme;

pub const ENGINE_ID: &str = "fogbreak-hub";
pub const DEFAULT_HOST_KEY: &str = "fogbreakHubHost";
const MAX_OPEN_BYTES: usize = 512 << 20;

struct PendingCall {
  method: String,
  params: Value,
  resolve: Function,
  reject: Function,
}

struct PendingShot {
  rx: Receiver<Result<Value, String>>,
  resolve: Function,
  reject: Function,
}

#[derive(Default)]
struct BridgeInner {
  ctx: Option<egui::Context>,
  canvas: Option<HtmlCanvasElement>,
  queue: Vec<PendingCall>,
  shots: Vec<PendingShot>,
  ready_sent: bool,
  last_dirty: Option<bool>,
  last_rev: u64,
}

#[derive(Clone)]
pub struct HostBridge {
  inner: Rc<RefCell<BridgeInner>>,
}

pub fn find_page_canvas() -> Option<HtmlCanvasElement> {
  let doc = web_sys::window()?.document()?;
  let sel = format!("canvas[data-host=\"page\"][data-engine=\"{ENGINE_ID}\"]");
  if let Ok(Some(el)) = doc.query_selector(&sel) {
    if let Ok(canvas) = el.dyn_into::<HtmlCanvasElement>() {
      return Some(canvas);
    }
  }
  doc.get_element_by_id("fogbreak_canvas").and_then(|el| el.dyn_into::<HtmlCanvasElement>().ok())
}

fn event_prefix(canvas: Option<&HtmlCanvasElement>) -> String {
  canvas
    .and_then(|c| c.get_attribute("data-engine"))
    .filter(|s| !s.trim().is_empty())
    .unwrap_or_else(|| ENGINE_ID.to_owned())
}

fn rpc_key(canvas: &HtmlCanvasElement) -> String {
  canvas
    .get_attribute("data-rpc-key")
    .filter(|s| !s.trim().is_empty())
    .unwrap_or_else(|| DEFAULT_HOST_KEY.to_owned())
}

impl HostBridge {
  pub fn install(ctx: &egui::Context, canvas: HtmlCanvasElement) -> Self {
    let key = rpc_key(&canvas);
    let inner = Rc::new(RefCell::new(BridgeInner {
      ctx: Some(ctx.clone()),
      canvas: Some(canvas.clone()),
      ..BridgeInner::default()
    }));
    attach_window_api(&inner, &key);
    listen_host_events(&inner, &canvas);
    Self { inner }
  }

  pub fn pump(&self, app: &mut ArtcraftApp, ctx: &egui::Context) {
    self.mark_ready(ctx);
    self.drain_shots();
    let calls = {
      let mut st = self.inner.borrow_mut();
      st.ctx = Some(ctx.clone());
      std::mem::take(&mut st.queue)
    };
    for call in calls {
      self.dispatch_call(app, ctx, call);
    }
    self.drain_shots();
    self.emit_dirty_if_changed(app);
  }

  fn mark_ready(&self, ctx: &egui::Context) {
    let canvas = {
      let mut st = self.inner.borrow_mut();
      if st.ready_sent {
        return;
      }
      st.ready_sent = true;
      st.canvas.clone()
    };
    let rect = ctx.content_rect();
    emit_event(
      canvas.as_ref(),
      "ready",
      &json!({
        "engine": ENGINE_ID,
        "hostKey": DEFAULT_HOST_KEY,
        "version": env!("CARGO_PKG_VERSION"),
        "width": rect.width(),
        "height": rect.height(),
        "capabilities": capabilities_json(),
      }),
    );
  }

  fn emit_dirty_if_changed(&self, app: &ArtcraftApp) {
    let dirty = app.is_dirty();
    let rev = app.state_revision();
    let (changed, canvas) = {
      let mut st = self.inner.borrow_mut();
      if st.last_dirty != Some(dirty) || (dirty && st.last_rev != rev) {
        st.last_dirty = Some(dirty);
        st.last_rev = rev;
        (true, st.canvas.clone())
      } else {
        (false, None)
      }
    };
    if changed {
      emit_event(canvas.as_ref(), "dirty", &json!({ "dirty": dirty, "revision": rev }));
    }
  }

  fn drain_shots(&self) {
    let pending = std::mem::take(&mut self.inner.borrow_mut().shots);
    let mut keep = Vec::with_capacity(pending.len());
    for shot in pending {
      match shot.rx.try_recv() {
        Ok(res) => resolve_result(&shot.resolve, &shot.reject, res),
        Err(TryRecvError::Empty) => keep.push(shot),
        Err(TryRecvError::Disconnected) => reject_str(&shot.reject, "screenshot channel closed"),
      }
    }
    self.inner.borrow_mut().shots.extend(keep);
  }

  fn dispatch_call(&self, app: &mut ArtcraftApp, ctx: &egui::Context, call: PendingCall) {
    let canvas = self.inner.borrow().canvas.clone();
    match call.method.as_str() {
      "ping" => resolve_value(
        &call.resolve,
        &json!({
          "ok": true,
          "engine": ENGINE_ID,
          "hostKey": DEFAULT_HOST_KEY,
          "version": env!("CARGO_PKG_VERSION"),
        }),
      ),
      "engine.commands" | "commands.list" | "listCommands" => {
        resolve_value(&call.resolve, &list_all_commands(&call.params));
      },
      "commands.describe" | "command.describe" | "describeCommand" => {
        resolve_result(&call.resolve, &call.reject, describe_command(&call.params));
      },
      "ui.theme" | "theme.get" | "theme.set" => {
        let dark_req = call.params.get("dark").and_then(Value::as_bool).or_else(|| call.params.get("mode").and_then(Value::as_str).map(|s| !s.eq_ignore_ascii_case("light")));
        let res = app.execute_command(ctx, &call.method, &call.params);
        if let Ok(ref payload) = res {
          if dark_req.is_some() {
            let dark = payload.get("dark").and_then(Value::as_bool).unwrap_or(true);
            theme::apply_palette(ctx, dark);
            emit_event(canvas.as_ref(), "theme", payload);
          }
        }
        resolve_result(&call.resolve, &call.reject, res);
      },
      "ui.screenshot" => {
        let rx = app.request_screenshot(ctx);
        self.inner.borrow_mut().shots.push(PendingShot { rx, resolve: call.resolve, reject: call.reject });
      },
      "io.import" | "doc.open" | "file.open" | "open" => {
        let res = handle_import(app, ctx, canvas.as_ref(), &call.params);
        resolve_result(&call.resolve, &call.reject, res);
      },
      "io.export" => {
        let res = handle_export(app, canvas.as_ref(), &call.params);
        resolve_result(&call.resolve, &call.reject, res);
      },
      "doc.handoff" | "handoff.emit" => {
        let target = call.params.get("target").and_then(Value::as_str).unwrap_or("");
        let payload = call.params.get("payload").cloned().unwrap_or(Value::Null);
        emit_event(canvas.as_ref(), "handoff", &json!({ "target": target, "payload": payload }));
        resolve_value(&call.resolve, &json!({ "emitted": true, "target": target }));
      },
      "engine.execute" | "execute" | "executeCommand" => {
        let id = call.params.get("command").and_then(Value::as_str).unwrap_or("");
        let sub = call.params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.execute_mapped(app, ctx, id, sub, call.resolve, call.reject);
      },
      other => self.execute_mapped(app, ctx, other, call.params, call.resolve, call.reject),
    }
  }

  fn execute_mapped(&self, app: &mut ArtcraftApp, ctx: &egui::Context, cmd: &str, params: Value, resolve: Function, reject: Function) {
    let canvas = self.inner.borrow().canvas.clone();
    match cmd {
      "ui.screenshot" => {
        let rx = app.request_screenshot(ctx);
        self.inner.borrow_mut().shots.push(PendingShot { rx, resolve, reject });
      },
      "io.import" => resolve_result(&resolve, &reject, handle_import(app, ctx, canvas.as_ref(), &params)),
      "io.export" => resolve_result(&resolve, &reject, handle_export(app, canvas.as_ref(), &params)),
      _ => {
        let res = app.execute_command(ctx, cmd, &params);
        if cmd == "ui.theme" && (params.get("dark").is_some() || params.get("mode").is_some()) {
          if let Ok(ref payload) = res {
            if let Some(dark) = payload.get("dark").and_then(Value::as_bool) {
              theme::apply_palette(ctx, dark);
              emit_event(canvas.as_ref(), "theme", payload);
            }
          }
        }
        resolve_result(&resolve, &reject, res);
      },
    }
  }
}

fn capabilities_json() -> Value {
  json!({
    "execute": true,
    "commands": true,
    "listCommands": true,
    "describeCommand": true,
    "docInspect": true,
    "uiInspect": true,
    "screenshot": true,
    "open": true,
    "theme": true,
    "io": true,
  })
}

fn list_all_commands(params: &Value) -> Value {
  let filter = params.get("filter").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_ascii_lowercase);
  let out: Vec<Value> = ArtcraftApp::commands_list()
    .iter()
    .filter(|c| match filter.as_deref() {
      Some(f) => c.id.to_ascii_lowercase().contains(f) || c.label.to_ascii_lowercase().contains(f) || c.menu.to_ascii_lowercase().contains(f),
      None => true,
    })
    .map(|c| {
      json!({
        "id": c.id,
        "label": c.label,
        "menu": c.menu,
        "shortcut": c.shortcut,
        "enabled": true,
        "params": c.params,
      })
    })
    .collect();
  Value::Array(out)
}

fn describe_command(params: &Value) -> Result<Value, String> {
  let id = params.get("id").or_else(|| params.get("command")).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| "missing `id`".to_owned())?;
  let spec = ArtcraftApp::describe_command(id).ok_or_else(|| format!("unknown command `{id}`"))?;
  Ok(json!({
    "id": spec.id,
    "label": spec.label,
    "menu": spec.menu,
    "shortcut": spec.shortcut,
    "params": spec.params,
  }))
}

fn decode_import_bytes(params: &Value) -> Result<Vec<u8>, String> {
  if let Some(b64) = params.get("bytes_base64").or_else(|| params.get("base64")).and_then(Value::as_str) {
    return STANDARD.decode(b64.trim()).map_err(|e| format!("invalid base64: {e}"));
  }
  if let Some(text) = params.get("text").and_then(Value::as_str) {
    return Ok(text.as_bytes().to_vec());
  }
  if let Some(arr) = params.get("bytes").and_then(Value::as_array) {
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
      let byte = item.as_u64().and_then(|v| u8::try_from(v).ok()).ok_or_else(|| "byte out of range".to_owned())?;
      out.push(byte);
    }
    return Ok(out);
  }
  if let Some(data) = params.get("data").and_then(Value::as_str) {
    let trimmed = data.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
      return Ok(data.as_bytes().to_vec());
    }
    return STANDARD.decode(data.trim()).map_err(|e| format!("invalid base64 data: {e}"));
  }
  if let Some(obj) = params.get("data").filter(|v| v.is_object()) {
    return serde_json::to_vec(obj).map_err(|e| e.to_string());
  }
  Err("expected `bytes_base64`, `bytes`, `text`, or `data`".to_owned())
}

fn handle_import(app: &mut ArtcraftApp, ctx: &egui::Context, canvas: Option<&HtmlCanvasElement>, params: &Value) -> Result<Value, String> {
  let name = params.get("name").or_else(|| params.get("filename")).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("import.json").to_owned();
  let bytes = decode_import_bytes(params)?;
  if bytes.len() > MAX_OPEN_BYTES {
    return Err("file exceeds size limit".to_owned());
  }
  let res = app.import_bytes(ctx, name.clone(), bytes)?;
  emit_event(canvas, "open", &json!({ "name": name, "result": res }));
  Ok(res)
}

fn handle_export(app: &ArtcraftApp, canvas: Option<&HtmlCanvasElement>, params: &Value) -> Result<Value, String> {
  let format = params.get("format").and_then(Value::as_str).unwrap_or("json");
  let download = params.get("download").or_else(|| params.get("deliverSave")).and_then(Value::as_bool).unwrap_or(false);
  let custom_name = params.get("name").or_else(|| params.get("filename")).and_then(Value::as_str);
  let (default_name, mime, bytes) = app.export_workspace(format)?;
  let name = custom_name.filter(|s| !s.trim().is_empty()).map_or(default_name, str::to_owned);
  if download {
    trigger_browser_download(&name, mime, &bytes)?;
    emit_event(canvas, "save", &json!({ "filename": name, "byteLength": bytes.len() }));
  }
  Ok(json!({
    "ok": true,
    "format": format,
    "name": name,
    "mime": mime,
    "byteLength": bytes.len(),
    "bytes_base64": STANDARD.encode(&bytes),
  }))
}

fn attach_window_api(inner: &Rc<RefCell<BridgeInner>>, key: &str) {
  let Some(window) = web_sys::window() else {
    return;
  };
  let host = Object::new();
  let _ = Reflect::set(&host, &JsValue::from_str("engine"), &JsValue::from_str(ENGINE_ID));
  let _ = Reflect::set(&host, &JsValue::from_str("hostKey"), &JsValue::from_str(key));
  let _ = Reflect::set(&host, &JsValue::from_str("version"), &JsValue::from_str(env!("CARGO_PKG_VERSION")));
  let _ = Reflect::set(&host, &JsValue::from_str("capabilities"), &json_to_js(&capabilities_json()));

  bind_request(&host, inner);
  bind_execute(&host, "execute", inner);
  bind_execute(&host, "executeCommand", inner);
  bind_filter_method(&host, "commands", "engine.commands", inner);
  bind_filter_method(&host, "listCommands", "engine.commands", inner);
  bind_single_arg(&host, "describeCommand", "commands.describe", "id", inner);
  bind_detail_method(&host, "docInspect", "doc.inspect", inner);
  bind_detail_method(&host, "inspect", "doc.inspect", inner);
  bind_no_arg(&host, "uiInspect", "ui.inspect", inner);
  bind_no_arg(&host, "screenshot", "ui.screenshot", inner);
  bind_open(&host, inner);
  bind_handoff(&host, inner);
  bind_theme_arg(&host, "setTheme", inner);
  let _ = Reflect::set(&host, &JsValue::from_str("theme"), &build_theme_object(inner));
  let _ = Reflect::set(&host, &JsValue::from_str("io"), &build_io_object(inner));
  let _ = Reflect::set(&window, &JsValue::from_str(key), &host);
}

fn listen_host_events(inner: &Rc<RefCell<BridgeInner>>, canvas: &HtmlCanvasElement) {
  let Some(window) = web_sys::window() else {
    return;
  };
  let state = inner.clone();
  let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
    if let Ok(custom) = ev.dyn_into::<CustomEvent>() {
      let detail = js_to_json(&custom.detail());
      let mut st = state.borrow_mut();
      let (resolve, reject) = dummy_resolvers();
      st.queue.push(PendingCall { method: "ui.theme".to_owned(), params: detail, resolve, reject });
      if let Some(ctx) = &st.ctx {
        ctx.request_repaint();
      }
    }
  });
  let target: &web_sys::EventTarget = canvas.as_ref();
  let _ = target.add_event_listener_with_callback(&format!("{ENGINE_ID}:setTheme"), cb.as_ref().unchecked_ref());
  let _ = window.add_event_listener_with_callback("fogbreak:setTheme", cb.as_ref().unchecked_ref());
  cb.forget();
}

fn dummy_resolvers() -> (Function, Function) {
  let ok = Closure::once_into_js(|_: JsValue| {});
  let err = Closure::once_into_js(|_: JsValue| {});
  (ok.unchecked_into(), err.unchecked_into())
}

fn build_theme_object(inner: &Rc<RefCell<BridgeInner>>) -> Object {
  let obj = Object::new();
  bind_no_arg(&obj, "get", "theme.get", inner);
  bind_theme_arg(&obj, "set", inner);
  obj
}

fn build_io_object(inner: &Rc<RefCell<BridgeInner>>) -> Object {
  let obj = Object::new();
  bind_no_arg(&obj, "formats", "io.formats", inner);

  let imp_state = inner.clone();
  let imp_cb = Closure::<dyn FnMut(JsValue, JsValue) -> Promise>::new(move |arg0: JsValue, arg1: JsValue| {
    let params = parse_io_import_args(&arg0, &arg1);
    enqueue_promise(&imp_state, "io.import".to_owned(), params)
  });
  let _ = Reflect::set(&obj, &JsValue::from_str("import"), imp_cb.as_ref());
  imp_cb.forget();

  let exp_state = inner.clone();
  let exp_cb = Closure::<dyn FnMut(JsValue, JsValue) -> Promise>::new(move |arg0: JsValue, arg1: JsValue| {
    let params = if let Some(fmt) = arg0.as_string() {
      let mut map = match js_to_json(&arg1) {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
      };
      map.insert("format".to_owned(), Value::String(fmt));
      Value::Object(map)
    } else {
      js_to_json(&arg0)
    };
    enqueue_promise(&exp_state, "io.export".to_owned(), params)
  });
  let _ = Reflect::set(&obj, &JsValue::from_str("export"), exp_cb.as_ref());
  exp_cb.forget();
  obj
}

fn parse_io_import_args(arg0: &JsValue, arg1: &JsValue) -> Value {
  if let Some(filename) = arg0.as_string() {
    return bytes_to_import_params(filename, arg1);
  }
  let name = Reflect::get(arg0, &JsValue::from_str("name")).ok().and_then(|v| v.as_string()).unwrap_or_else(|| "import.json".to_owned());
  if let Ok(bytes_val) = Reflect::get(arg0, &JsValue::from_str("bytes")) {
    if !bytes_val.is_undefined() && !bytes_val.is_null() {
      return bytes_to_import_params(name, &bytes_val);
    }
  }
  js_to_json(arg0)
}

fn bytes_to_import_params(name: String, data: &JsValue) -> Value {
  if let Ok(arr) = data.clone().dyn_into::<Uint8Array>() {
    json!({ "name": name, "bytes_base64": STANDARD.encode(arr.to_vec()) })
  } else if let Ok(buf) = data.clone().dyn_into::<js_sys::ArrayBuffer>() {
    let arr = Uint8Array::new(&buf);
    json!({ "name": name, "bytes_base64": STANDARD.encode(arr.to_vec()) })
  } else if let Some(s) = data.as_string() {
    json!({ "name": name, "data": s })
  } else {
    json!({ "name": name, "data": js_to_json(data) })
  }
}

fn bind_theme_arg(host: &Object, name: &str, inner: &Rc<RefCell<BridgeInner>>) {
  let state = inner.clone();
  let cb = Closure::<dyn FnMut(JsValue) -> Promise>::new(move |val: JsValue| {
    let params = if let Some(dark) = val.as_bool() {
      json!({ "dark": dark, "mode": if dark { "dark" } else { "light" } })
    } else if val.is_undefined() || val.is_null() {
      json!({})
    } else if let Some(mode) = val.as_string() {
      json!({ "mode": mode })
    } else {
      js_to_json(&val)
    };
    enqueue_promise(&state, "ui.theme".to_owned(), params)
  });
  let _ = Reflect::set(host, &JsValue::from_str(name), cb.as_ref());
  cb.forget();
}

fn bind_request(host: &Object, inner: &Rc<RefCell<BridgeInner>>) {
  let state = inner.clone();
  let cb = Closure::<dyn FnMut(JsValue, JsValue) -> Promise>::new(move |method: JsValue, params: JsValue| {
    enqueue_promise(&state, method.as_string().unwrap_or_default(), js_to_json(&params))
  });
  let _ = Reflect::set(host, &JsValue::from_str("request"), cb.as_ref());
  cb.forget();
}

fn bind_execute(host: &Object, name: &str, inner: &Rc<RefCell<BridgeInner>>) {
  let state = inner.clone();
  let cb = Closure::<dyn FnMut(JsValue, JsValue) -> Promise>::new(move |cmd: JsValue, params: JsValue| {
    enqueue_promise(&state, "engine.execute".to_owned(), json!({ "command": cmd.as_string().unwrap_or_default(), "params": js_to_json(&params) }))
  });
  let _ = Reflect::set(host, &JsValue::from_str(name), cb.as_ref());
  cb.forget();
}

fn bind_filter_method(host: &Object, name: &str, method: &str, inner: &Rc<RefCell<BridgeInner>>) {
  let state = inner.clone();
  let method_owned = method.to_owned();
  let cb = Closure::<dyn FnMut(JsValue) -> Promise>::new(move |filter: JsValue| {
    let p = match filter.as_string() {
      Some(f) if !f.is_empty() => json!({ "filter": f }),
      _ => json!({}),
    };
    enqueue_promise(&state, method_owned.clone(), p)
  });
  let _ = Reflect::set(host, &JsValue::from_str(name), cb.as_ref());
  cb.forget();
}

fn bind_detail_method(host: &Object, name: &str, method: &str, inner: &Rc<RefCell<BridgeInner>>) {
  let state = inner.clone();
  let method_owned = method.to_owned();
  let cb = Closure::<dyn FnMut(JsValue) -> Promise>::new(move |detail: JsValue| {
    let p = match detail.as_string() {
      Some(d) if !d.is_empty() => json!({ "detail": d }),
      _ => json!({}),
    };
    enqueue_promise(&state, method_owned.clone(), p)
  });
  let _ = Reflect::set(host, &JsValue::from_str(name), cb.as_ref());
  cb.forget();
}

fn bind_single_arg(host: &Object, name: &str, method: &str, field: &'static str, inner: &Rc<RefCell<BridgeInner>>) {
  let state = inner.clone();
  let method_owned = method.to_owned();
  let cb = Closure::<dyn FnMut(JsValue) -> Promise>::new(move |val: JsValue| {
    enqueue_promise(&state, method_owned.clone(), json!({ field: js_to_json(&val) }))
  });
  let _ = Reflect::set(host, &JsValue::from_str(name), cb.as_ref());
  cb.forget();
}

fn bind_no_arg(host: &Object, name: &str, method: &str, inner: &Rc<RefCell<BridgeInner>>) {
  let state = inner.clone();
  let method_owned = method.to_owned();
  let cb = Closure::<dyn FnMut() -> Promise>::new(move || enqueue_promise(&state, method_owned.clone(), json!({})));
  let _ = Reflect::set(host, &JsValue::from_str(name), cb.as_ref());
  cb.forget();
}

fn bind_open(host: &Object, inner: &Rc<RefCell<BridgeInner>>) {
  let state = inner.clone();
  let cb = Closure::<dyn FnMut(JsValue, JsValue) -> Promise>::new(move |name: JsValue, data: JsValue| {
    let filename = name.as_string().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "import.json".to_owned());
    enqueue_promise(&state, "io.import".to_owned(), bytes_to_import_params(filename, &data))
  });
  let _ = Reflect::set(host, &JsValue::from_str("open"), cb.as_ref());
  cb.forget();
}

fn bind_handoff(host: &Object, inner: &Rc<RefCell<BridgeInner>>) {
  let state = inner.clone();
  let cb = Closure::<dyn FnMut(JsValue, JsValue) -> Promise>::new(move |target: JsValue, payload: JsValue| {
    enqueue_promise(&state, "doc.handoff".to_owned(), json!({ "target": target.as_string().unwrap_or_default(), "payload": js_to_json(&payload) }))
  });
  let _ = Reflect::set(host, &JsValue::from_str("emitHandoff"), cb.as_ref());
  cb.forget();
}

fn enqueue_promise(state: &Rc<RefCell<BridgeInner>>, method: String, params: Value) -> Promise {
  Promise::new(&mut |resolve, reject| {
    let mut st = state.borrow_mut();
    st.queue.push(PendingCall { method, params, resolve, reject });
    if let Some(ctx) = &st.ctx {
      ctx.request_repaint();
    }
  })
}

fn js_to_json(val: &JsValue) -> Value {
  if val.is_undefined() || val.is_null() {
    return json!({});
  }
  let Ok(str_val) = JSON::stringify(val) else {
    return json!({});
  };
  str_val.as_string().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({}))
}

fn json_to_js(val: &Value) -> JsValue {
  let Ok(s) = serde_json::to_string(val) else {
    return JsValue::NULL;
  };
  JSON::parse(&s).unwrap_or(JsValue::NULL)
}

fn resolve_value(resolve: &Function, val: &Value) {
  let js_val = json_to_js(val);
  if let Some(b64) = val.get("bytes_base64").and_then(Value::as_str) {
    if let Ok(decoded) = STANDARD.decode(b64) {
      let arr = Uint8Array::from(decoded.as_slice());
      let _ = Reflect::set(&js_val, &JsValue::from_str("bytes"), &arr);
      let _ = Reflect::set(&js_val, &JsValue::from_str("data"), &arr);
    }
  }
  let _ = resolve.call1(&JsValue::NULL, &js_val);
}

fn reject_str(reject: &Function, msg: &str) {
  let _ = reject.call1(&JsValue::NULL, &js_sys::Error::new(msg).into());
}

fn resolve_result(resolve: &Function, reject: &Function, res: Result<Value, String>) {
  match res {
    Ok(val) => resolve_value(resolve, &val),
    Err(msg) => reject_str(reject, &msg),
  }
}

pub fn emit_event(canvas: Option<&HtmlCanvasElement>, suffix: &str, detail: &Value) {
  let name = format!("{}:{suffix}", event_prefix(canvas));
  let init = CustomEventInit::new();
  init.set_detail(&json_to_js(detail));
  if let Some(c) = canvas {
    if let Ok(ev) = CustomEvent::new_with_event_init_dict(&name, &init) {
      let _ = c.dispatch_event(&ev);
    }
  }
  if let Some(window) = web_sys::window() {
    if let Ok(ev) = CustomEvent::new_with_event_init_dict(&name, &init) {
      let _ = window.dispatch_event(&ev);
    }
    if let Some(doc) = window.document() {
      if let Ok(ev) = CustomEvent::new_with_event_init_dict(&name, &init) {
        let _ = doc.dispatch_event(&ev);
      }
    }
  }
}

fn trigger_browser_download(name: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
  let js = |e: JsValue| format!("Download failed: {e:?}");
  let window = web_sys::window().ok_or("no window")?;
  let document = window.document().ok_or("no document")?;
  let parts = Array::of1(&Uint8Array::from(bytes));
  let opts = BlobPropertyBag::new();
  opts.set_type(mime);
  let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
  let url = Url::create_object_url_with_blob(&blob).map_err(js)?;
  let anchor: HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor".to_owned())?;
  anchor.set_href(&url);
  anchor.set_download(name);
  anchor.style().set_property("display", "none").map_err(js)?;
  document.body().ok_or("no body")?.append_child(&anchor).map_err(js)?;
  anchor.click();
  anchor.remove();
  let revoke = Closure::once_into_js(move || {
    let _ = Url::revoke_object_url(&url);
  });
  window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000).map_err(js)?;
  Ok(())
}
