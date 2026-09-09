use crate::HostConfig;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use muxy_core::extensions::bridge::{ExtensionBridgeSurface, extension_bridge_script};
use muxy_proto::client::{ExtensionHostClient, ExtensionHostClientError, HostIncoming};
use muxy_proto::extension::{ExtensionLocalEvent, InvokeResult};
use objc2_javascript_core::{
    JSContext, JSContextRef, JSEvaluateScript, JSGlobalContextCreate, JSGlobalContextRef,
    JSGlobalContextRelease, JSObjectCallAsFunction, JSObjectMakeFunctionWithCallback, JSObjectRef,
    JSObjectSetProperty, JSStringCreateWithCharacters, JSStringGetMaximumUTF8CStringSize,
    JSStringGetUTF8CString, JSStringRef, JSStringRelease, JSValue, JSValueRef,
    kJSPropertyAttributeNone,
};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::time::{Duration, Instant};
use thiserror::Error;

const LOOP_INTERVAL: Duration = Duration::from_millis(2);
const MINIMUM_REPEAT_INTERVAL: Duration = Duration::from_millis(1);
const MAXIMUM_TIMER_DELAY: Duration = Duration::from_secs(31 * 24 * 60 * 60);

thread_local! {
    static ACTIVE_STATE: RefCell<Option<Rc<RefCell<HostState>>>> = const { RefCell::new(None) };
}

#[derive(Debug, Error)]
pub enum HostRuntimeError {
    #[error("could not read background script at {path}: {source}")]
    ReadScript {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not connect to Muxy socket: {0}")]
    Client(#[from] ExtensionHostClientError),
    #[error("could not create a JavaScriptCore context")]
    Context,
    #[error("JavaScript error: {0}")]
    JavaScript(String),
}

struct Timer {
    callback: JSObjectRef,
    deadline: Instant,
    repeat: Option<Duration>,
}

struct HostState {
    client: ExtensionHostClient,
    extension_id: String,
    timers: BTreeMap<u64, Timer>,
    next_timer_id: u64,
}

impl HostState {
    fn dispatch(&self, verb: &str, args: Value) -> Value {
        match verb {
            "events.emit" => self.dispatch_extension_event(args),
            "notifications.notify" => self.dispatch_notification(args),
            _ => self.dispatch_value(verb, args),
        }
    }

    fn dispatch_extension_event(&self, args: Value) -> Value {
        let Some(name) = args.get("event").and_then(Value::as_str) else {
            return api_error("extension events must start with extension.");
        };
        let payload = args.get("payload").cloned().unwrap_or(Value::Null);
        let Ok(payload) = serde_json::to_vec(&payload) else {
            return api_error("event payload must be JSON-serializable and at most 65536 bytes");
        };
        let event = ExtensionLocalEvent {
            name: name.to_owned(),
            payload,
        };
        let Some(line) = event.encode() else {
            return api_error("event payload must be JSON-serializable and at most 65536 bytes");
        };
        match self.client.send_and_wait_reply(&line) {
            Ok(reply) if reply == "ok" => api_value(Value::Null),
            Ok(reply) if reply.starts_with("error:") => api_error(&reply["error:".len()..]),
            Ok(_) => api_error("invalid events.emit reply"),
            Err(error) => api_error(&error.to_string()),
        }
    }

    fn dispatch_notification(&self, args: Value) -> Value {
        let title = sanitized(
            args.get("title")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        let body = sanitized(args.get("body").and_then(Value::as_str).unwrap_or_default());
        if title.is_empty() && body.is_empty() {
            return api_error("notification requires title or body");
        }
        let pane_id = sanitized(
            args.get("paneID")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        let line = format!("{}|{pane_id}|{title}|{body}", sanitized(&self.extension_id));
        match self.client.send(&line) {
            Ok(()) => api_value(Value::Null),
            Err(error) => api_error(&error.to_string()),
        }
    }

    fn dispatch_value(&self, verb: &str, args: Value) -> Value {
        let Ok(payload) = serde_json::to_vec(&args) else {
            return api_error(&format!("could not encode {verb} payload"));
        };
        let line = format!("{verb}|{}", STANDARD.encode(payload));
        let reply = match self.client.send_and_wait_reply(&line) {
            Ok(reply) => reply,
            Err(error) => return api_error(&error.to_string()),
        };
        if let Some(error) = reply.strip_prefix("error:") {
            return api_error(error);
        }
        let Some(payload) = STANDARD
            .decode(reply.as_bytes())
            .ok()
            .and_then(|payload| serde_json::from_slice(&payload).ok())
        else {
            return api_error(&format!("invalid {verb} reply"));
        };
        api_value(payload)
    }

    fn subscribe(&self, name: &str) {
        let reply = self
            .client
            .send_and_wait_reply(&format!("subscribe|{name}"));
        match reply {
            Ok(reply) if reply == "ok" => {}
            Ok(reply) => eprintln!("[muxy-extension-host] subscribe {name} failed: {reply}"),
            Err(error) => eprintln!("[muxy-extension-host] subscribe {name} error: {error}"),
        }
    }

    fn send_invoke_result(&self, call_id: &str, ok: bool, body: Vec<u8>) {
        let result = InvokeResult {
            call_id: call_id.to_owned(),
            ok,
            body,
        };
        if let Some(line) = result.encode() {
            let _ = self.client.send(&line);
        }
    }

    fn schedule_timer(
        &mut self,
        ctx: JSContextRef,
        callback: JSObjectRef,
        delay: Duration,
        repeats: bool,
    ) -> u64 {
        let timer_id = self.next_timer_id;
        self.next_timer_id = self.next_timer_id.saturating_add(1).max(1);
        let repeat = repeats.then_some(delay.max(MINIMUM_REPEAT_INTERVAL));
        unsafe {
            JSValue::protect(ctx, callback.cast_const());
        }
        self.timers.insert(
            timer_id,
            Timer {
                callback,
                deadline: Instant::now() + delay,
                repeat,
            },
        );
        timer_id
    }

    fn cancel_timer(&mut self, ctx: JSContextRef, timer_id: u64) {
        if let Some(timer) = self.timers.remove(&timer_id) {
            unsafe {
                JSValue::unprotect(ctx, timer.callback.cast_const());
            }
        }
    }
}

struct JavaScriptRuntime {
    context: JSGlobalContextRef,
    state: Rc<RefCell<HostState>>,
}

impl JavaScriptRuntime {
    fn new(config: &HostConfig) -> Result<Self, HostRuntimeError> {
        let source = std::fs::read_to_string(&config.script_path).map_err(|source| {
            HostRuntimeError::ReadScript {
                path: config.script_path.clone(),
                source,
            }
        })?;
        let client = ExtensionHostClient::connect_and_identify(
            &config.socket_path,
            &config.extension_id,
            &config.token,
        )?;
        let context = unsafe { JSGlobalContextCreate(null_mut()) };
        if context.is_null() {
            return Err(HostRuntimeError::Context);
        }
        let state = Rc::new(RefCell::new(HostState {
            client,
            extension_id: config.extension_id.clone(),
            timers: BTreeMap::new(),
            next_timer_id: 1,
        }));
        ACTIVE_STATE.with(|active| {
            *active.borrow_mut() = Some(Rc::clone(&state));
        });
        let runtime = Self { context, state };
        runtime.install_native_functions()?;
        runtime.evaluate(
            "globalThis.setTimeout = (fn, delay) => __muxySetTimer(fn, Number(delay) || 0, false);\n\
             globalThis.setInterval = (fn, delay) => __muxySetTimer(fn, Number(delay) || 0, true);\n\
             globalThis.clearTimeout = (id) => __muxyClearTimer(Number(id) || 0);\n\
             globalThis.clearInterval = (id) => __muxyClearTimer(Number(id) || 0);",
            None,
        )?;
        runtime.evaluate(
            &extension_bridge_script(&config.extension_id, ExtensionBridgeSurface::Background),
            None,
        )?;
        runtime.evaluate(&source, Some(&config.script_path))?;
        Ok(runtime)
    }

    fn install_native_functions(&self) -> Result<(), HostRuntimeError> {
        for (name, callback) in [
            ("__muxyDispatch", dispatch_callback as NativeCallback),
            ("__muxyConsole", console_callback as NativeCallback),
            ("__muxySubscribe", subscribe_callback as NativeCallback),
            (
                "__muxyInvokeResolve",
                invoke_resolve_callback as NativeCallback,
            ),
            (
                "__muxyInvokeReject",
                invoke_reject_callback as NativeCallback,
            ),
            ("__muxySetTimer", set_timer_callback as NativeCallback),
            ("__muxyClearTimer", clear_timer_callback as NativeCallback),
        ] {
            self.install_function(name, callback)?;
        }
        Ok(())
    }

    fn install_function(
        &self,
        name: &str,
        callback: NativeCallback,
    ) -> Result<(), HostRuntimeError> {
        let name = OwnedJsString::new(name);
        let function = unsafe {
            JSObjectMakeFunctionWithCallback(self.context.cast_const(), name.0, Some(callback))
        };
        if function.is_null() {
            return Err(HostRuntimeError::Context);
        }
        let global = unsafe { JSContext::global_object(self.context.cast_const()) };
        let mut exception = null();
        unsafe {
            JSObjectSetProperty(
                self.context.cast_const(),
                global,
                name.0,
                function.cast_const(),
                kJSPropertyAttributeNone,
                &mut exception,
            );
        }
        if exception.is_null() {
            Ok(())
        } else {
            Err(HostRuntimeError::JavaScript(value_to_string(
                self.context.cast_const(),
                exception,
            )))
        }
    }

    fn evaluate(
        &self,
        source: &str,
        source_path: Option<&PathBuf>,
    ) -> Result<(), HostRuntimeError> {
        let source = OwnedJsString::new(source);
        let source_url = source_path.map(|path| OwnedJsString::new(&path.display().to_string()));
        let mut exception = null();
        unsafe {
            JSEvaluateScript(
                self.context.cast_const(),
                source.0,
                null_mut(),
                source_url.as_ref().map_or(null_mut(), |url| url.0),
                1,
                &mut exception,
            );
        }
        if exception.is_null() {
            Ok(())
        } else {
            Err(HostRuntimeError::JavaScript(value_to_string(
                self.context.cast_const(),
                exception,
            )))
        }
    }

    fn run_loop(&self) {
        loop {
            let closed = self.state.borrow().client.is_closed();
            if closed {
                break;
            }
            loop {
                let incoming = self.state.borrow().client.try_recv();
                match incoming {
                    Ok(Some(incoming)) => self.deliver(incoming),
                    Ok(None) => break,
                    Err(_) => return,
                }
            }
            self.fire_timers();
            std::thread::sleep(LOOP_INTERVAL);
        }
    }

    fn deliver(&self, incoming: HostIncoming) {
        let script = match incoming {
            HostIncoming::Broadcast(event) => format!(
                "__muxyDispatchEvent({}, {});",
                json_string(&event.name),
                serde_json::to_string(&event.payload).unwrap_or_else(|_| "{}".to_owned())
            ),
            HostIncoming::ExtensionEvent(event) => {
                let payload =
                    serde_json::from_slice::<Value>(&event.payload).unwrap_or(Value::Null);
                format!(
                    "__muxyDispatchEvent({}, {});",
                    json_string(&event.name),
                    serde_json::to_string(&payload).unwrap_or_else(|_| "null".to_owned())
                )
            }
            HostIncoming::Invoke(request) => {
                let payload =
                    serde_json::from_slice::<Value>(&request.payload).unwrap_or(Value::Null);
                format!(
                    "__muxyDispatchInvoke({}, {}, {});",
                    json_string(&request.call_id),
                    json_string(&request.action),
                    serde_json::to_string(&payload).unwrap_or_else(|_| "null".to_owned())
                )
            }
            HostIncoming::ModalResult(result) => {
                let payload =
                    serde_json::from_slice::<Value>(&result.payload).unwrap_or(Value::Null);
                format!(
                    "__muxiDeliverModalResult({}, {});",
                    json_string(&result.request_id),
                    serde_json::to_string(&payload).unwrap_or_else(|_| "null".to_owned())
                )
            }
            HostIncoming::ModalQuery(query) => format!(
                "__muxyDeliverModalQuery({}, {}, {}, {});",
                json_string(&query.request_id),
                query.query_id,
                json_string(&query.query),
                serde_json::to_string(&query.options).unwrap_or_else(|_| "{}".to_owned())
            ),
        };
        if let Err(error) = self.evaluate(&script, None) {
            eprintln!("[muxy-extension-host] {error}");
        }
    }

    fn fire_timers(&self) {
        let now = Instant::now();
        let due = self
            .state
            .borrow()
            .timers
            .iter()
            .filter(|(_, timer)| timer.deadline <= now)
            .map(|(timer_id, _)| *timer_id)
            .collect::<Vec<_>>();
        for timer_id in due {
            let timer = {
                let mut state = self.state.borrow_mut();
                let Some(timer) = state.timers.get_mut(&timer_id) else {
                    continue;
                };
                if let Some(repeat) = timer.repeat {
                    timer.deadline = Instant::now() + repeat;
                    (timer.callback, false)
                } else {
                    let timer = state.timers.remove(&timer_id).unwrap();
                    (timer.callback, true)
                }
            };
            let mut exception = null();
            unsafe {
                JSObjectCallAsFunction(
                    self.context.cast_const(),
                    timer.0,
                    null_mut(),
                    0,
                    null_mut(),
                    &mut exception,
                );
                if timer.1 {
                    JSValue::unprotect(self.context.cast_const(), timer.0.cast_const());
                }
            }
            if !exception.is_null() {
                eprintln!(
                    "[muxy-extension-host] timer error: {}",
                    value_to_string(self.context.cast_const(), exception)
                );
            }
        }
    }
}

impl Drop for JavaScriptRuntime {
    fn drop(&mut self) {
        ACTIVE_STATE.with(|active| {
            active.borrow_mut().take();
        });
        let timers = std::mem::take(&mut self.state.borrow_mut().timers);
        for timer in timers.into_values() {
            unsafe {
                JSValue::unprotect(self.context.cast_const(), timer.callback.cast_const());
            }
        }
        unsafe {
            JSGlobalContextRelease(self.context);
        }
    }
}

type NativeCallback = unsafe extern "C-unwind" fn(
    JSContextRef,
    JSObjectRef,
    JSObjectRef,
    usize,
    *mut JSValueRef,
    *mut JSValueRef,
) -> JSValueRef;

pub fn run(config: HostConfig) -> Result<(), HostRuntimeError> {
    let runtime = JavaScriptRuntime::new(&config)?;
    if !config.oneshot {
        runtime.run_loop();
    }
    Ok(())
}

pub fn monitor_parent() {
    let parent = unsafe { libc::getppid() };
    if parent <= 1 {
        std::process::exit(0);
    }
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let current = unsafe { libc::getppid() };
            if current != parent || current <= 1 {
                std::process::exit(0);
            }
        }
    });
}

unsafe extern "C-unwind" fn dispatch_callback(
    ctx: JSContextRef,
    _function: JSObjectRef,
    _this: JSObjectRef,
    argument_count: usize,
    arguments: *mut JSValueRef,
    _exception: *mut JSValueRef,
) -> JSValueRef {
    let arguments = unsafe { callback_arguments(argument_count, arguments) };
    let Some(verb) = arguments.first().map(|value| value_to_string(ctx, *value)) else {
        return json_to_value(ctx, &api_error("missing extension api verb"));
    };
    let args = arguments
        .get(1)
        .and_then(|value| value_to_json(ctx, *value))
        .unwrap_or_else(|| json!({}));
    let reply = with_state(|state| state.dispatch(&verb, args))
        .unwrap_or_else(|| api_error("host released"));
    json_to_value(ctx, &reply)
}

unsafe extern "C-unwind" fn console_callback(
    ctx: JSContextRef,
    _function: JSObjectRef,
    _this: JSObjectRef,
    argument_count: usize,
    arguments: *mut JSValueRef,
    _exception: *mut JSValueRef,
) -> JSValueRef {
    let arguments = unsafe { callback_arguments(argument_count, arguments) };
    let level = arguments
        .first()
        .map(|value| value_to_string(ctx, *value))
        .unwrap_or_else(|| "log".to_owned());
    let message = arguments
        .get(1)
        .map(|value| value_to_string(ctx, *value))
        .unwrap_or_default();
    eprintln!("[{level}] {message}");
    unsafe { JSValue::new_undefined(ctx) }
}

unsafe extern "C-unwind" fn subscribe_callback(
    ctx: JSContextRef,
    _function: JSObjectRef,
    _this: JSObjectRef,
    argument_count: usize,
    arguments: *mut JSValueRef,
    _exception: *mut JSValueRef,
) -> JSValueRef {
    let arguments = unsafe { callback_arguments(argument_count, arguments) };
    if let Some(name) = arguments.first().map(|value| value_to_string(ctx, *value)) {
        let _ = with_state(|state| state.subscribe(&name));
    }
    unsafe { JSValue::new_undefined(ctx) }
}

unsafe extern "C-unwind" fn invoke_resolve_callback(
    ctx: JSContextRef,
    _function: JSObjectRef,
    _this: JSObjectRef,
    argument_count: usize,
    arguments: *mut JSValueRef,
    _exception: *mut JSValueRef,
) -> JSValueRef {
    send_invoke_callback(ctx, argument_count, arguments, true);
    unsafe { JSValue::new_undefined(ctx) }
}

unsafe extern "C-unwind" fn invoke_reject_callback(
    ctx: JSContextRef,
    _function: JSObjectRef,
    _this: JSObjectRef,
    argument_count: usize,
    arguments: *mut JSValueRef,
    _exception: *mut JSValueRef,
) -> JSValueRef {
    send_invoke_callback(ctx, argument_count, arguments, false);
    unsafe { JSValue::new_undefined(ctx) }
}

fn send_invoke_callback(
    ctx: JSContextRef,
    argument_count: usize,
    arguments: *mut JSValueRef,
    ok: bool,
) {
    let arguments = unsafe { callback_arguments(argument_count, arguments) };
    let Some(call_id) = arguments.first().map(|value| value_to_string(ctx, *value)) else {
        return;
    };
    let body = arguments
        .get(1)
        .map(|value| value_to_string(ctx, *value).into_bytes())
        .unwrap_or_default();
    let _ = with_state(|state| state.send_invoke_result(&call_id, ok, body));
}

unsafe extern "C-unwind" fn set_timer_callback(
    ctx: JSContextRef,
    _function: JSObjectRef,
    _this: JSObjectRef,
    argument_count: usize,
    arguments: *mut JSValueRef,
    _exception: *mut JSValueRef,
) -> JSValueRef {
    let arguments = unsafe { callback_arguments(argument_count, arguments) };
    let Some(callback) = arguments.first().copied() else {
        return unsafe { JSValue::new_number(ctx, 0.0) };
    };
    let callback = unsafe { JSValue::to_object(ctx, callback, null_mut()) };
    if callback.is_null() {
        return unsafe { JSValue::new_number(ctx, 0.0) };
    }
    let delay = arguments
        .get(1)
        .map(|value| unsafe { JSValue::to_number(ctx, *value, null_mut()) })
        .unwrap_or_default();
    let repeats = arguments
        .get(2)
        .is_some_and(|value| unsafe { JSValue::to_boolean(ctx, *value) });
    let delay = timer_delay(delay);
    let timer_id = with_state_mut(|state| state.schedule_timer(ctx, callback, delay, repeats))
        .unwrap_or_default();
    unsafe { JSValue::new_number(ctx, timer_id as f64) }
}

unsafe extern "C-unwind" fn clear_timer_callback(
    ctx: JSContextRef,
    _function: JSObjectRef,
    _this: JSObjectRef,
    argument_count: usize,
    arguments: *mut JSValueRef,
    _exception: *mut JSValueRef,
) -> JSValueRef {
    let arguments = unsafe { callback_arguments(argument_count, arguments) };
    let timer_id = arguments
        .first()
        .map(|value| unsafe { JSValue::to_number(ctx, *value, null_mut()) })
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| value as u64)
        .unwrap_or_default();
    let _ = with_state_mut(|state| state.cancel_timer(ctx, timer_id));
    unsafe { JSValue::new_undefined(ctx) }
}

unsafe fn callback_arguments<'a>(
    argument_count: usize,
    arguments: *mut JSValueRef,
) -> &'a [JSValueRef] {
    if argument_count == 0 || arguments.is_null() {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(arguments, argument_count) }
    }
}

fn with_state<T>(callback: impl FnOnce(&HostState) -> T) -> Option<T> {
    ACTIVE_STATE.with(|active| {
        let state = active.borrow().as_ref()?.clone();
        let state = state.try_borrow().ok()?;
        Some(callback(&state))
    })
}

fn with_state_mut<T>(callback: impl FnOnce(&mut HostState) -> T) -> Option<T> {
    ACTIVE_STATE.with(|active| {
        let state = active.borrow().as_ref()?.clone();
        let mut state = state.try_borrow_mut().ok()?;
        Some(callback(&mut state))
    })
}

fn api_value(value: Value) -> Value {
    json!({ "ok": true, "value": value })
}

fn api_error(error: &str) -> Value {
    json!({ "ok": false, "error": error })
}

fn sanitized(value: &str) -> String {
    value.replace(['|', '\n'], " ")
}

fn timer_delay(milliseconds: f64) -> Duration {
    if !milliseconds.is_finite() || milliseconds <= 0.0 {
        return Duration::ZERO;
    }
    Duration::from_secs_f64((milliseconds / 1000.0).min(MAXIMUM_TIMER_DELAY.as_secs_f64()))
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

fn value_to_json(ctx: JSContextRef, value: JSValueRef) -> Option<Value> {
    let mut exception = null();
    let string = unsafe { JSValue::create_json_string(ctx, value, 0, &mut exception) };
    if string.is_null() || !exception.is_null() {
        return None;
    }
    let string = OwnedJsString(string);
    serde_json::from_str(&string.to_rust()).ok()
}

fn json_to_value(ctx: JSContextRef, value: &Value) -> JSValueRef {
    let json = serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned());
    let json = OwnedJsString::new(&json);
    let value = unsafe { JSValue::from_json_string(ctx, json.0) };
    if value.is_null() {
        unsafe { JSValue::new_null(ctx) }
    } else {
        value
    }
}

fn value_to_string(ctx: JSContextRef, value: JSValueRef) -> String {
    if value.is_null() {
        return String::new();
    }
    let string = unsafe { JSValue::to_string_copy(ctx, value, null_mut()) };
    if string.is_null() {
        String::new()
    } else {
        OwnedJsString(string).to_rust()
    }
}

struct OwnedJsString(JSStringRef);

impl OwnedJsString {
    fn new(value: &str) -> Self {
        let characters = value.encode_utf16().collect::<Vec<_>>();
        Self(unsafe { JSStringCreateWithCharacters(characters.as_ptr(), characters.len()) })
    }

    fn to_rust(&self) -> String {
        let capacity = unsafe { JSStringGetMaximumUTF8CStringSize(self.0) };
        if capacity == 0 {
            return String::new();
        }
        let mut bytes = vec![0_u8; capacity];
        let count =
            unsafe { JSStringGetUTF8CString(self.0, bytes.as_mut_ptr().cast(), bytes.len()) };
        if count == 0 {
            return String::new();
        }
        bytes.truncate(count.saturating_sub(1));
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl Drop for OwnedJsString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                JSStringRelease(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_delays_are_bounded_and_repeatable() {
        assert_eq!(timer_delay(-1.0), Duration::ZERO);
        assert_eq!(timer_delay(f64::NAN), Duration::ZERO);
        assert_eq!(timer_delay(250.0), Duration::from_millis(250));
        assert_eq!(timer_delay(f64::INFINITY), Duration::ZERO);
        assert_eq!(timer_delay(f64::MAX), MAXIMUM_TIMER_DELAY);
    }

    #[test]
    fn api_replies_and_sanitization_match_the_bridge_contract() {
        assert_eq!(
            api_value(json!({"ready": true})),
            json!({
                "ok": true,
                "value": {"ready": true}
            })
        );
        assert_eq!(
            api_error("denied"),
            json!({
                "ok": false,
                "error": "denied"
            })
        );
        assert_eq!(sanitized("one|two\nthree"), "one two three");
    }
}
