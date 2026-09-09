use super::{
    ExtensionLifecycleCoordinator, ExtensionWebViewApiCall, ExtensionWebViewDescriptor,
    ExtensionWebViewEvent, WEBVIEW_MESSAGE_HANDLER_NAME,
};
use crate::native_compositor::{NativeViewCompositor, NativeViewRegistration};
use block2::{DynBlock, RcBlock};
use gpui::{AnyElement, IntoElement, Rgba, Styled};
use muxy_api::extensions::{ExtensionApiError, ExtensionApiRequest, invalid_webview_message};
use muxy_core::extensions::assets::{
    ExtensionAssetError, allows_extension_navigation, resolve_extension_asset,
};
use objc2::rc::{Retained, Weak as ObjcWeak};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSColor, NSEvent, NSEventMask, NSView};
use objc2_foundation::{
    MainThreadMarker, NSData, NSDictionary, NSError, NSHTTPURLResponse, NSJSONReadingOptions,
    NSJSONSerialization, NSJSONWritingOptions, NSPoint, NSRect, NSSize, NSString, NSURL,
    NSURLErrorBadURL, NSURLErrorDataLengthExceedsMaximum, NSURLErrorDomain,
    NSURLErrorFileDoesNotExist, NSURLErrorNoPermissionsToReadFile, NSURLRequest,
};
use objc2_web_kit::{
    WKContentWorld, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
    WKScriptMessage, WKScriptMessageHandlerWithReply, WKURLSchemeHandler, WKURLSchemeTask,
    WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebView,
    WKWebViewConfiguration,
};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

struct AssetRequest {
    task: usize,
    absolute_url: String,
}

struct AssetHandlerIvars {
    worker: mpsc::Sender<AssetRequest>,
    active_tasks: Arc<Mutex<HashSet<usize>>>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MuxyExtensionAssetSchemeHandler"]
    #[ivars = AssetHandlerIvars]
    struct ExtensionAssetSchemeHandler;

    #[allow(non_snake_case)]
    unsafe impl WKURLSchemeHandler for ExtensionAssetSchemeHandler {
        #[unsafe(method(webView:startURLSchemeTask:))]
        unsafe fn webView_startURLSchemeTask(
            &self,
            _web_view: &WKWebView,
            task: &ProtocolObject<dyn WKURLSchemeTask>,
        ) {
            let task_id = scheme_task_id(task);
            self.ivars().active_tasks.lock().unwrap().insert(task_id);
            let request = unsafe { task.request() };
            let Some(absolute_url) = request
                .URL()
                .and_then(|url| url.absoluteString())
                .map(|url| url.to_string())
            else {
                fail_scheme_task_if_active(
                    &self.ivars().active_tasks,
                    task,
                    ExtensionAssetError::InvalidUrl,
                );
                return;
            };
            let retained: Retained<ProtocolObject<dyn WKURLSchemeTask>> =
                unsafe { Retained::retain(std::ptr::from_ref(task).cast_mut()) }.unwrap();
            let retained = Retained::into_raw(retained);
            let request = AssetRequest {
                task: retained.cast::<()>() as usize,
                absolute_url,
            };
            if let Err(error) = self.ivars().worker.send(request) {
                let request = error.0;
                let task = unsafe {
                    Retained::from_raw(request.task as *mut ProtocolObject<dyn WKURLSchemeTask>)
                }
                .unwrap();
                fail_scheme_task_if_active(
                    &self.ivars().active_tasks,
                    &task,
                    ExtensionAssetError::ReadFailed,
                );
            }
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        unsafe fn webView_stopURLSchemeTask(
            &self,
            _web_view: &WKWebView,
            task: &ProtocolObject<dyn WKURLSchemeTask>,
        ) {
            self.ivars()
                .active_tasks
                .lock()
                .unwrap()
                .remove(&scheme_task_id(task));
        }
    }

    unsafe impl NSObjectProtocol for ExtensionAssetSchemeHandler {}
);

impl ExtensionAssetSchemeHandler {
    fn new(mtm: MainThreadMarker, extension_id: String, resource_root: PathBuf) -> Retained<Self> {
        let active_tasks = Arc::new(Mutex::new(HashSet::new()));
        let (worker, requests) = mpsc::channel();
        let worker_active_tasks = active_tasks.clone();
        std::thread::Builder::new()
            .name(format!("muxy-extension-assets-{extension_id}"))
            .spawn(move || {
                while let Ok(request) = requests.recv() {
                    handle_asset_request(
                        &extension_id,
                        &resource_root,
                        &worker_active_tasks,
                        request,
                    );
                }
            })
            .expect("failed to start extension asset worker");
        let this = Self::alloc(mtm).set_ivars(AssetHandlerIvars {
            worker,
            active_tasks,
        });
        unsafe { msg_send![super(this), init] }
    }
}

struct BridgeHandlerIvars {
    extension_id: String,
    surface_id: String,
    sender: async_channel::Sender<ExtensionWebViewEvent>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MuxyExtensionWebViewBridgeHandler"]
    #[ivars = BridgeHandlerIvars]
    struct ExtensionBridgeHandler;

    #[allow(non_snake_case)]
    unsafe impl WKScriptMessageHandlerWithReply for ExtensionBridgeHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:replyHandler:))]
        unsafe fn userContentController_didReceiveScriptMessage_replyHandler(
            &self,
            _controller: &WKUserContentController,
            message: &WKScriptMessage,
            reply_handler: &DynBlock<dyn Fn(*mut AnyObject, *mut NSString)>,
        ) {
            let reply_handler = reply_handler.copy();
            let body = unsafe { message.body() };
            let Ok(message) = foundation_to_json(&body) else {
                reply_to_webview(&reply_handler, invalid_webview_message());
                return;
            };
            let call = ExtensionWebViewApiCall::new(
                self.ivars().extension_id.clone(),
                self.ivars().surface_id.clone(),
                message,
                move |reply| reply_to_webview(&reply_handler, reply),
            );
            if let Err(error) = self
                .ivars()
                .sender
                .try_send(ExtensionWebViewEvent::Api(call))
                && let ExtensionWebViewEvent::Api(call) = error.into_inner()
            {
                call.complete(serde_json::json!({
                    "ok": false,
                    "error": "app state unavailable"
                }));
            }
        }
    }

    unsafe impl NSObjectProtocol for ExtensionBridgeHandler {}
);

impl ExtensionBridgeHandler {
    fn new(
        mtm: MainThreadMarker,
        extension_id: String,
        surface_id: String,
        sender: async_channel::Sender<ExtensionWebViewEvent>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(BridgeHandlerIvars {
            extension_id,
            surface_id,
            sender,
        });
        unsafe { msg_send![super(this), init] }
    }
}

struct NavigationDelegateIvars;

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MuxyExtensionNavigationDelegate"]
    #[ivars = NavigationDelegateIvars]
    struct ExtensionNavigationDelegate;

    #[allow(non_snake_case)]
    unsafe impl WKNavigationDelegate for ExtensionNavigationDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        unsafe fn webView_decidePolicyForNavigationAction_decisionHandler(
            &self,
            _web_view: &WKWebView,
            action: &WKNavigationAction,
            decision_handler: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let request = unsafe { action.request() };
            let absolute = request
                .URL()
                .and_then(|url| url.absoluteString())
                .map(|url| url.to_string());
            let policy = if allows_extension_navigation(absolute.as_deref()) {
                WKNavigationActionPolicy::Allow
            } else {
                WKNavigationActionPolicy::Cancel
            };
            decision_handler.call((policy,));
        }
    }

    unsafe impl NSObjectProtocol for ExtensionNavigationDelegate {}
);

impl ExtensionNavigationDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavigationDelegateIvars);
        unsafe { msg_send![super(this), init] }
    }
}

pub(crate) struct ExtensionWebViewRegistry {
    compositor: Option<NativeViewCompositor>,
    sender: async_channel::Sender<ExtensionWebViewEvent>,
    surfaces: HashMap<String, ExtensionWebViewSurface>,
    focus_targets: Rc<RefCell<HashMap<String, ObjcWeak<WKWebView>>>>,
    focus_monitor: Option<Retained<AnyObject>>,
    lifecycle: ExtensionLifecycleCoordinator,
}

impl ExtensionWebViewRegistry {
    pub(crate) fn new(
        terminals: &crate::terminal::TerminalSurfaces,
    ) -> (Self, async_channel::Receiver<ExtensionWebViewEvent>) {
        let (sender, receiver) = async_channel::unbounded();
        let focus_targets = Rc::new(RefCell::new(HashMap::new()));
        let focus_monitor = match install_focus_monitor(focus_targets.clone(), sender.clone()) {
            Ok(monitor) => Some(monitor),
            Err(error) => {
                log::warn!("failed to install extension webview focus monitor: {error}");
                None
            }
        };
        (
            Self {
                compositor: terminals.native_view_compositor(),
                sender,
                surfaces: HashMap::new(),
                focus_targets,
                focus_monitor,
                lifecycle: ExtensionLifecycleCoordinator::default(),
            },
            receiver,
        )
    }

    pub(crate) fn reconcile(
        &mut self,
        descriptors: &[ExtensionWebViewDescriptor],
        visible: &HashSet<String>,
        focused: Option<&str>,
        theme: &Value,
        background: Rgba,
    ) {
        let known = descriptors
            .iter()
            .map(|descriptor| descriptor.instance_id.as_str())
            .collect::<HashSet<_>>();
        let removed = self
            .surfaces
            .keys()
            .filter(|surface_id| !known.contains(surface_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        for surface_id in removed {
            self.focus_targets.borrow_mut().remove(&surface_id);
            self.surfaces.remove(&surface_id);
            self.lifecycle.allow_surface(&surface_id);
        }
        for descriptor in descriptors {
            let recreate = self
                .surfaces
                .get(&descriptor.instance_id)
                .is_some_and(|surface| !surface.matches(descriptor));
            if recreate {
                self.focus_targets
                    .borrow_mut()
                    .remove(&descriptor.instance_id);
                self.surfaces.remove(&descriptor.instance_id);
                self.lifecycle.allow_surface(&descriptor.instance_id);
            }
            if !self.surfaces.contains_key(&descriptor.instance_id)
                && let Some(compositor) = self.compositor.as_ref()
            {
                match ExtensionWebViewSurface::new(
                    descriptor.clone(),
                    theme.clone(),
                    focused == Some(descriptor.instance_id.as_str()),
                    background,
                    compositor,
                    self.sender.clone(),
                ) {
                    Ok(surface) => {
                        self.focus_targets.borrow_mut().insert(
                            descriptor.instance_id.clone(),
                            ObjcWeak::from_retained(&surface.webview),
                        );
                        self.surfaces
                            .insert(descriptor.instance_id.clone(), surface);
                    }
                    Err(error) => {
                        log::warn!(
                            "failed to create extension webview {}: {error}",
                            descriptor.instance_id
                        );
                    }
                }
            }
            if let Some(surface) = self.surfaces.get_mut(&descriptor.instance_id) {
                surface.update(
                    descriptor,
                    theme,
                    focused == Some(descriptor.instance_id.as_str()),
                    visible.contains(&descriptor.instance_id),
                    background,
                );
            }
        }
    }

    pub(crate) fn element(&self, surface_id: &str, visible: bool) -> Option<AnyElement> {
        self.surfaces.get(surface_id)?.element(visible)
    }

    pub(crate) fn request_before_close(
        &mut self,
        surface_id: &str,
        reason: &str,
    ) -> Option<(
        String,
        async_channel::Receiver<super::ExtensionLifecycleVerdict>,
    )> {
        let surface = self.surfaces.get(surface_id)?;
        let (sender, receiver) = async_channel::bounded(1);
        let call_id = self
            .lifecycle
            .begin(surface_id, Instant::now(), move |verdict| {
                let _ = sender.try_send(verdict);
            });
        surface.request_before_close(&call_id, reason);
        Some((call_id, receiver))
    }

    pub(crate) fn expire_unacknowledged(&mut self, call_id: &str) -> bool {
        self.lifecycle
            .expire_unacknowledged(call_id, Instant::now())
    }

    pub(crate) fn handle_lifecycle_message(
        &mut self,
        surface_id: &str,
        request: &ExtensionApiRequest,
    ) -> Option<Result<Value, ExtensionApiError>> {
        super::handle_lifecycle_message(&mut self.lifecycle, surface_id, request)
    }

    pub(crate) fn dispatch_event(&self, extension_id: Option<&str>, name: &str, payload: &Value) {
        for surface in self.surfaces.values() {
            if extension_id
                .is_none_or(|extension_id| surface.descriptor.extension_id == extension_id)
            {
                surface.dispatch_event(name, payload);
            }
        }
    }

    pub(crate) fn deliver_modal_result(&self, surface_id: &str, request_id: &str, result: &Value) {
        if let Some(surface) = self.surfaces.get(surface_id) {
            evaluate(
                &surface.webview,
                &format!(
                    "globalThis.__muxiDeliverModalResult?.({}, {});",
                    json_literal(&Value::String(request_id.to_owned()), "\"\""),
                    json_literal(result, "null")
                ),
            );
        }
    }
}

impl Drop for ExtensionWebViewRegistry {
    fn drop(&mut self) {
        if let Some(monitor) = self.focus_monitor.take() {
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
    }
}

struct ExtensionWebViewSurface {
    descriptor: ExtensionWebViewDescriptor,
    webview: Retained<WKWebView>,
    registration: NativeViewRegistration,
    _asset_handler: Retained<ExtensionAssetSchemeHandler>,
    _bridge_handler: Retained<ExtensionBridgeHandler>,
    _navigation_delegate: Retained<ExtensionNavigationDelegate>,
    data: Value,
    theme: Value,
    focused: bool,
    visible: bool,
    background: Rgba,
}

impl ExtensionWebViewSurface {
    fn new(
        descriptor: ExtensionWebViewDescriptor,
        theme: Value,
        focused: bool,
        background: Rgba,
        compositor: &NativeViewCompositor,
        sender: async_channel::Sender<ExtensionWebViewEvent>,
    ) -> Result<Self, String> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| "extension webviews must be created on the main thread".to_owned())?;
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
        let asset_handler = ExtensionAssetSchemeHandler::new(
            mtm,
            descriptor.extension_id.clone(),
            descriptor.resource_root.clone(),
        );
        let asset_protocol: &ProtocolObject<dyn WKURLSchemeHandler> =
            ProtocolObject::from_ref(&*asset_handler);
        unsafe {
            configuration.setURLSchemeHandler_forURLScheme(
                Some(asset_protocol),
                &NSString::from_str(muxy_core::extensions::assets::EXTENSION_ASSET_SCHEME),
            );
        }
        let bridge_handler = ExtensionBridgeHandler::new(
            mtm,
            descriptor.extension_id.clone(),
            descriptor.instance_id.clone(),
            sender.clone(),
        );
        let bridge_protocol: &ProtocolObject<dyn WKScriptMessageHandlerWithReply> =
            ProtocolObject::from_ref(&*bridge_handler);
        let user_content = unsafe { configuration.userContentController() };
        unsafe {
            user_content.addScriptMessageHandlerWithReply_contentWorld_name(
                bridge_protocol,
                &WKContentWorld::pageWorld(mtm),
                &NSString::from_str(WEBVIEW_MESSAGE_HANDLER_NAME),
            );
        }
        let source = webview_bridge_script(&descriptor, &theme, focused);
        let user_script = unsafe {
            WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                WKUserScript::alloc(mtm),
                &NSString::from_str(&source),
                WKUserScriptInjectionTime::AtDocumentStart,
                true,
            )
        };
        unsafe { user_content.addUserScript(&user_script) };
        let webview = unsafe {
            WKWebView::initWithFrame_configuration(
                WKWebView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
                &configuration,
            )
        };
        unsafe { webview.setInspectable(cfg!(debug_assertions)) };
        apply_background(&webview, background);
        let navigation_delegate = ExtensionNavigationDelegate::new(mtm);
        let navigation_protocol: &ProtocolObject<dyn WKNavigationDelegate> =
            ProtocolObject::from_ref(&*navigation_delegate);
        unsafe { webview.setNavigationDelegate(Some(navigation_protocol)) };
        let native_view: &NSView = &webview;
        let registration = compositor
            .register_interactive(native_view, 0)
            .map_err(|error| error.to_string())?;
        let url = NSURL::URLWithString(&NSString::from_str(&descriptor.entry_url))
            .ok_or_else(|| "invalid extension entry URL".to_owned())?;
        let request = NSURLRequest::requestWithURL(&url);
        unsafe {
            webview.loadRequest(&request);
        }
        Ok(Self {
            data: descriptor.data.clone(),
            descriptor,
            webview,
            registration,
            _asset_handler: asset_handler,
            _bridge_handler: bridge_handler,
            _navigation_delegate: navigation_delegate,
            theme,
            focused,
            visible: false,
            background,
        })
    }

    fn matches(&self, descriptor: &ExtensionWebViewDescriptor) -> bool {
        self.descriptor.extension_id == descriptor.extension_id
            && self.descriptor.entry_url == descriptor.entry_url
            && self.descriptor.resource_root == descriptor.resource_root
            && self.descriptor.kind == descriptor.kind
    }

    fn update(
        &mut self,
        descriptor: &ExtensionWebViewDescriptor,
        theme: &Value,
        focused: bool,
        visible: bool,
        background: Rgba,
    ) {
        self.visible = visible;
        self.registration.set_visible(visible);
        if self.data != descriptor.data {
            self.data = descriptor.data.clone();
            evaluate(
                &self.webview,
                &format!(
                    "globalThis.__muxyApplyData?.({});",
                    json_literal(&self.data, "null")
                ),
            );
        }
        if self.theme != *theme {
            self.theme = theme.clone();
            evaluate(
                &self.webview,
                &format!(
                    "globalThis.__muxyApplyTheme?.({});",
                    json_literal(&self.theme, "{}")
                ),
            );
        }
        if self.focused != focused {
            self.focused = focused;
            evaluate(
                &self.webview,
                &format!("globalThis.__muxyApplyFocus?.({focused});"),
            );
        }
        if focused {
            self.registration.focus();
        }
        if self.background != background {
            self.background = background;
            apply_background(&self.webview, background);
        }
        self.descriptor.data = descriptor.data.clone();
    }

    fn element(&self, visible: bool) -> Option<AnyElement> {
        Some(
            self.registration
                .slot()?
                .visible(visible && self.visible)
                .size_full()
                .into_any_element(),
        )
    }

    fn request_before_close(&self, call_id: &str, reason: &str) {
        evaluate(
            &self.webview,
            &before_close_script(call_id, reason, &self.descriptor.instance_id),
        );
    }

    fn dispatch_event(&self, name: &str, payload: &Value) {
        evaluate(
            &self.webview,
            &format!(
                "globalThis.__muxyDispatchEvent?.({}, {});",
                json_literal(&Value::String(name.to_owned()), "\"\""),
                json_literal(payload, "null")
            ),
        );
    }
}

impl Drop for ExtensionWebViewSurface {
    fn drop(&mut self) {
        unsafe {
            self.webview.stopLoading();
            self.webview.setNavigationDelegate(None);
            let content = self.webview.configuration().userContentController();
            content.removeAllScriptMessageHandlers();
            content.removeAllUserScripts();
        }
    }
}

fn install_focus_monitor(
    focus_targets: Rc<RefCell<HashMap<String, ObjcWeak<WKWebView>>>>,
    sender: async_channel::Sender<ExtensionWebViewEvent>,
) -> Result<Retained<AnyObject>, String> {
    let block = RcBlock::new(move |event_pointer: NonNull<NSEvent>| -> *mut NSEvent {
        let event = unsafe { event_pointer.as_ref() };
        let surface_id = focus_targets
            .borrow()
            .iter()
            .find_map(|(surface_id, webview)| {
                webview
                    .load()
                    .filter(|webview| !webview.isHidden() && event_targets_view(event, webview))
                    .map(|_| surface_id.clone())
            });
        if let Some(surface_id) = surface_id {
            let _ = sender.try_send(ExtensionWebViewEvent::FocusRequested { surface_id });
        }
        event_pointer.as_ptr()
    });
    unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown,
            &block,
        )
    }
    .ok_or_else(|| "failed to install extension webview focus monitor".to_owned())
}

fn event_targets_view(event: &NSEvent, webview: &WKWebView) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(event_window) = event.window(mtm) else {
        return false;
    };
    let Some(view_window) = webview.window() else {
        return false;
    };
    if Retained::as_ptr(&event_window) != Retained::as_ptr(&view_window) {
        return false;
    }
    let point = webview.convertPoint_fromView(event.locationInWindow(), None);
    point_inside_rect(point, webview.bounds())
}

fn point_inside_rect(point: NSPoint, bounds: NSRect) -> bool {
    point.x >= bounds.origin.x
        && point.y >= bounds.origin.y
        && point.x < bounds.origin.x + bounds.size.width
        && point.y < bounds.origin.y + bounds.size.height
}

fn before_close_script(call_id: &str, reason: &str, instance_id: &str) -> String {
    format!(
        "if (typeof globalThis.__muxyBeforeClose === 'function') {{ globalThis.__muxyBeforeClose({}, {}, {}); }} else if (typeof globalThis.__muxyResolveBeforeClose === 'function') {{ globalThis.__muxyResolveBeforeClose({}, false); }}",
        json_literal(&Value::String(call_id.to_owned()), "null"),
        json_literal(&Value::String(reason.to_owned()), "null"),
        json_literal(&Value::String(instance_id.to_owned()), "null"),
        json_literal(&Value::String(call_id.to_owned()), "null"),
    )
}

fn webview_bridge_script(
    descriptor: &ExtensionWebViewDescriptor,
    theme: &Value,
    focused: bool,
) -> String {
    let bootstrap = r#"(() => {
    const handler = globalThis.webkit?.messageHandlers?.muxy;
    let nextRequestID = 1;
    globalThis.__muxyDispatch = (verb, args) => {
        if (!handler) return Promise.reject(new Error('extension bridge unavailable'));
        return handler.postMessage({ verb, args: args || {}, requestID: String(nextRequestID++) });
    };
})();"#;
    format!(
        "{bootstrap}\n{}",
        muxy_core::extensions::bridge::extension_webview_bridge_script(
            &descriptor.extension_id,
            &descriptor.instance_id,
            &descriptor.data,
            theme,
            focused,
        )
    )
}

fn evaluate(webview: &WKWebView, source: &str) {
    unsafe {
        webview.evaluateJavaScript_completionHandler(&NSString::from_str(source), None);
    }
}

fn apply_background(webview: &WKWebView, background: Rgba) {
    let color = NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(background.r),
        f64::from(background.g),
        f64::from(background.b),
        f64::from(background.a),
    );
    unsafe { webview.setUnderPageBackgroundColor(Some(&color)) };
}

fn foundation_to_json(object: &AnyObject) -> Result<Value, ()> {
    let data = unsafe {
        NSJSONSerialization::dataWithJSONObject_options_error(
            object,
            NSJSONWritingOptions::FragmentsAllowed,
        )
    }
    .map_err(|_| ())?;
    serde_json::from_slice(&data.to_vec()).map_err(|_| ())
}

fn json_to_foundation(value: &Value) -> Result<Retained<AnyObject>, ()> {
    let bytes = serde_json::to_vec(value).map_err(|_| ())?;
    NSJSONSerialization::JSONObjectWithData_options_error(
        &NSData::with_bytes(&bytes),
        NSJSONReadingOptions::FragmentsAllowed,
    )
    .map_err(|_| ())
}

fn reply_to_webview(reply_handler: &RcBlock<dyn Fn(*mut AnyObject, *mut NSString)>, reply: Value) {
    match json_to_foundation(&reply) {
        Ok(object) => {
            reply_handler.call((Retained::as_ptr(&object).cast_mut(), std::ptr::null_mut()))
        }
        Err(()) => {
            let error = NSString::from_str("extension response encoding failed");
            reply_handler.call((std::ptr::null_mut(), Retained::as_ptr(&error).cast_mut()));
        }
    }
}

fn scheme_task_id(task: &ProtocolObject<dyn WKURLSchemeTask>) -> usize {
    task as *const _ as *const () as usize
}

fn handle_asset_request(
    extension_id: &str,
    resource_root: &std::path::Path,
    active_tasks: &Mutex<HashSet<usize>>,
    request: AssetRequest,
) {
    let task =
        unsafe { Retained::from_raw(request.task as *mut ProtocolObject<dyn WKURLSchemeTask>) }
            .unwrap();
    let result = resolve_extension_asset(resource_root, extension_id, &request.absolute_url);
    let active = active_tasks.lock().unwrap().remove(&scheme_task_id(&task));
    if !active {
        return;
    }
    match result {
        Ok(asset) => {
            let Some(url) = NSURL::URLWithString(&NSString::from_str(&request.absolute_url)) else {
                fail_scheme_task(&task, ExtensionAssetError::InvalidUrl);
                return;
            };
            finish_scheme_task(&task, &url, asset);
        }
        Err(error) => fail_scheme_task(&task, error),
    }
}

fn fail_scheme_task_if_active(
    active_tasks: &Mutex<HashSet<usize>>,
    task: &ProtocolObject<dyn WKURLSchemeTask>,
    error: ExtensionAssetError,
) {
    if active_tasks.lock().unwrap().remove(&scheme_task_id(task)) {
        fail_scheme_task(task, error);
    }
}

fn finish_scheme_task(
    task: &ProtocolObject<dyn WKURLSchemeTask>,
    url: &NSURL,
    asset: muxy_core::extensions::assets::ExtensionAsset,
) {
    let keys = [
        NSString::from_str("Content-Type"),
        NSString::from_str("Content-Length"),
        NSString::from_str("Cache-Control"),
    ];
    let values = [
        NSString::from_str(asset.content_type),
        NSString::from_str(&asset.bytes.len().to_string()),
        NSString::from_str(asset.cache_control),
    ];
    let headers = NSDictionary::from_slices(
        &[&*keys[0], &*keys[1], &*keys[2]],
        &[&*values[0], &*values[1], &*values[2]],
    );
    let response = NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
        NSHTTPURLResponse::alloc(),
        url,
        200,
        Some(&NSString::from_str("HTTP/1.1")),
        Some(&headers),
    );
    let Some(response) = response else {
        fail_scheme_task(task, ExtensionAssetError::ReadFailed);
        return;
    };
    let data = NSData::from_vec(asset.bytes);
    unsafe {
        task.didReceiveResponse(&response);
        task.didReceiveData(&data);
        task.didFinish();
    }
}

fn fail_scheme_task(task: &ProtocolObject<dyn WKURLSchemeTask>, error: ExtensionAssetError) {
    let code = match error {
        ExtensionAssetError::InvalidUrl | ExtensionAssetError::HostMismatch(_) => NSURLErrorBadURL,
        ExtensionAssetError::OutsideRoot => NSURLErrorNoPermissionsToReadFile,
        ExtensionAssetError::NotFound | ExtensionAssetError::ReadFailed => {
            NSURLErrorFileDoesNotExist
        }
        ExtensionAssetError::TooLarge => NSURLErrorDataLengthExceedsMaximum,
    };
    let error = unsafe { NSError::errorWithDomain_code_userInfo(NSURLErrorDomain, code, None) };
    unsafe { task.didFailWithError(&error) };
}

fn json_literal(value: &Value, fallback: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| fallback.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::surfaces::ExtensionSurfaceKind;

    #[test]
    fn webview_script_installs_reply_bridge_before_window_muxy() {
        let descriptor = ExtensionWebViewDescriptor {
            extension_id: "fixture".to_owned(),
            instance_id: "surface".to_owned(),
            entry_url: "muxy-ext://fixture/index.html".to_owned(),
            resource_root: PathBuf::from("/fixture"),
            data: serde_json::json!({"phase": 6}),
            kind: ExtensionSurfaceKind::Modal {
                entry: "index.html".to_owned(),
                width: 480.0,
                height: 320.0,
                dismiss_on_outside_click: true,
                opener_surface_id: None,
            },
        };
        let script = webview_bridge_script(
            &descriptor,
            &serde_json::json!({"colorScheme": "dark"}),
            true,
        );
        assert!(
            script.find("globalThis.__muxyDispatch").unwrap() < script.find("const muxy").unwrap()
        );
        assert!(script.contains("handler.postMessage"));
        assert!(script.contains("let currentFocus = true"));
        assert!(script.contains("let currentData = {\"phase\":6}"));
    }

    #[test]
    fn before_close_script_invokes_the_swift_lifecycle_contract_with_safe_literals() {
        let script = before_close_script("call\"1", "tab", "surface\n1");
        assert!(
            script
                .contains("globalThis.__muxyBeforeClose(\"call\\\"1\", \"tab\", \"surface\\n1\")")
        );
        assert!(script.contains("globalThis.__muxyResolveBeforeClose(\"call\\\"1\", false)"));
    }

    #[test]
    fn point_hit_testing_includes_leading_and_excludes_trailing_edges() {
        let bounds = NSRect::new(NSPoint::new(10.0, 20.0), NSSize::new(30.0, 40.0));

        assert!(point_inside_rect(NSPoint::new(10.0, 20.0), bounds));
        assert!(point_inside_rect(NSPoint::new(39.999, 59.999), bounds));
        assert!(!point_inside_rect(NSPoint::new(9.999, 20.0), bounds));
        assert!(!point_inside_rect(NSPoint::new(40.0, 20.0), bounds));
        assert!(!point_inside_rect(NSPoint::new(10.0, 60.0), bounds));
    }

    #[test]
    fn point_hit_testing_rejects_empty_bounds() {
        let zero_width = NSRect::new(NSPoint::new(10.0, 20.0), NSSize::new(0.0, 40.0));
        let zero_height = NSRect::new(NSPoint::new(10.0, 20.0), NSSize::new(30.0, 0.0));

        assert!(!point_inside_rect(NSPoint::new(10.0, 20.0), zero_width));
        assert!(!point_inside_rect(NSPoint::new(10.0, 20.0), zero_height));
    }
}
