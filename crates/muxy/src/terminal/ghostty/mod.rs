use crate::native_compositor::{NativeViewCompositor, NativeViewRegistration};
use crate::resources::AppResources;
use crate::terminal::surfaces::{AppSurfaceHandle, PaneLaunchContext, StandaloneLaunchContext};
use crate::terminal::{RoutedTerminalEvent, SurfaceIdentity, TerminalColorScheme};
use async_channel::Receiver;
use ghostty_host::{
    Action as RuntimeAction, ActionTarget, ClipboardContent, ClipboardLocation, ClipboardRequest,
    ClipboardRequestToken, ColorKind, ColorScheme, ConfigPaths, GhosttyApp, GhosttyConfig,
    Modifiers as RuntimeModifiers, MouseButton as RuntimeMouseButton,
    MouseButtonState as RuntimeMouseButtonState, MouseShape as RuntimeMouseShape, MouseVisibility,
    ProgressState, RuntimeEvent, SurfaceContext, SurfaceEnvironmentVariable, SurfaceOptions,
    SurfaceProcessState,
};
use gpui::{
    AnyElement, App, AppContext, Bounds, IntoElement, Styled, Task, Window, div, point, px,
};
use muxy_core::environment::BuildMode;
use muxy_core::shortcuts::{COMMAND, KeyCombo};
use muxy_core::workspace::TabId;
use muxy_terminal::backend::{
    LaunchCommand, PointerButton, PointerInput, PointerModifiers, SearchTotals, ShortcutGate,
    SurfaceAction, SurfaceMetadata, SurfaceProgress, SurfaceProgressKind, SurfaceSignal,
    TerminalSurfaceHandle, startup_shell_command, user_shell,
};
use muxy_terminal::confirmation::{
    ConfirmationDecision, ConfirmationId, ConfirmationKind, ConfirmationQueue,
};
use muxy_terminal::ghostty::cjk_font::{
    TemporaryConfigFile, config_text_for_user, resolve_system_font_family,
};
use muxy_terminal::ghostty::host_view::{GhosttyHostView, HostViewEvent};
use muxy_terminal::ghostty::pasteboard;
use muxy_terminal::ghostty::state::{
    ApplyActionResult, MouseShape, ProgressKind, RuntimeTarget, TerminalActionEvent, TerminalColor,
    TerminalColorSlot, TerminalProgress, TerminalStateAction, TerminalSurfaceState,
};
use muxy_terminal::scrollbar::ScrollbarMetrics;
use muxy_terminal::search::{SearchAction, SearchDirection};
use objc2::rc::Retained;
use objc2_app_kit::{NSBeep, NSView};
use objc2_foundation::MainThreadMarker;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;

type SharedState = Rc<RefCell<TerminalSurfaceState>>;
type SurfaceRegistry = Rc<RefCell<HashMap<u64, RoutedSurface>>>;
type SharedConfirmations = Rc<RefCell<ConfirmationQueue<PendingConfirmation>>>;

pub(crate) fn install_development_cli_environment(
    mode: BuildMode,
    socket_path: &Path,
) -> Result<(), String> {
    unsafe {
        std::env::remove_var("MUXY_HOOK_BIN");
        std::env::remove_var("MUXY_HOOK_SCRIPT");
    }
    if !mode.is_development() {
        return Ok(());
    }
    let resources = AppResources::discover().map_err(|error| error.to_string())?;
    let environment = development_cli_environment(
        mode,
        &resources.root,
        socket_path,
        std::env::var_os("PATH").as_deref(),
    )?;
    for variable in environment {
        unsafe { std::env::set_var(variable.key, variable.value) };
    }
    Ok(())
}

fn development_cli_environment(
    mode: BuildMode,
    resource_root: &Path,
    socket_path: &Path,
    inherited_path: Option<&OsStr>,
) -> Result<Vec<SurfaceEnvironmentVariable>, String> {
    if !mode.is_development() {
        return Ok(Vec::new());
    }
    let bin = resource_root.join("muxy-dev-bin");
    let launcher = bin.join("muxy");
    if !launcher.is_file() {
        return Err(format!(
            "development CLI launcher is missing at {}",
            launcher.display()
        ));
    }
    let app_path = resource_root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "resource root is not inside an app bundle".to_owned())?;
    let mut paths = vec![bin.clone()];
    if let Some(inherited_path) = inherited_path {
        paths.extend(std::env::split_paths(inherited_path).filter(|path| path != &bin));
    }
    let path = std::env::join_paths(paths)
        .map_err(|error| format!("failed to build development terminal PATH: {error}"))?;
    Ok(vec![
        SurfaceEnvironmentVariable::new("PATH", path.to_string_lossy().into_owned()),
        SurfaceEnvironmentVariable::new(
            "MUXY_DEVELOPMENT_CLI_BIN",
            bin.to_string_lossy().into_owned(),
        ),
        SurfaceEnvironmentVariable::new(
            "MUXY_DEVELOPMENT_APP_PATH",
            app_path.to_string_lossy().into_owned(),
        ),
        SurfaceEnvironmentVariable::new(
            "MUXY_DEVELOPMENT_SOCKET_PATH",
            socket_path.to_string_lossy().into_owned(),
        ),
        SurfaceEnvironmentVariable::new("MUXY_DEVELOPMENT_VERSION", env!("CARGO_PKG_VERSION")),
    ])
}

fn apply_pane_context(
    mut environment: Vec<SurfaceEnvironmentVariable>,
    context: &PaneLaunchContext,
) -> Vec<SurfaceEnvironmentVariable> {
    scrub_muxy_context(&mut environment);
    environment.extend(
        context
            .environment()
            .map(|(key, value)| SurfaceEnvironmentVariable::new(key, value)),
    );
    environment
}

fn apply_standalone_context(
    mut environment: Vec<SurfaceEnvironmentVariable>,
    context: &StandaloneLaunchContext,
) -> Vec<SurfaceEnvironmentVariable> {
    scrub_muxy_context(&mut environment);
    environment.push(SurfaceEnvironmentVariable::new(
        "MUXY_SOCKET_PATH",
        context.socket_path(),
    ));
    environment
}

const MUXY_CONTEXT_KEYS: [&str; 6] = [
    "MUXY_PANE_ID",
    "MUXY_PROJECT_ID",
    "MUXY_WORKTREE_ID",
    "MUXY_SOCKET_PATH",
    "MUXY_HOOK_BIN",
    "MUXY_HOOK_SCRIPT",
];

fn scrub_muxy_context(environment: &mut Vec<SurfaceEnvironmentVariable>) {
    environment.retain(|variable| !MUXY_CONTEXT_KEYS.contains(&variable.key.as_str()));
}

struct ProcessEnvironmentGuard {
    values: Vec<(&'static str, Option<OsString>)>,
}

impl ProcessEnvironmentGuard {
    fn standalone(socket_path: &str) -> Self {
        let values = MUXY_CONTEXT_KEYS
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect();
        for key in MUXY_CONTEXT_KEYS {
            unsafe { std::env::remove_var(key) };
        }
        unsafe { std::env::set_var("MUXY_SOCKET_PATH", socket_path) };
        Self { values }
    }
}

impl Drop for ProcessEnvironmentGuard {
    fn drop(&mut self) {
        for (key, value) in &self.values {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

fn with_standalone_process_environment<T>(socket_path: &str, operation: impl FnOnce() -> T) -> T {
    let guard = ProcessEnvironmentGuard::standalone(socket_path);
    let result = operation();
    drop(guard);
    result
}

struct PendingConfirmation {
    identity: SurfaceIdentity,
    payload: ConfirmationPayload,
}

enum ConfirmationPayload {
    ClipboardRead {
        token: ClipboardRequestToken,
        content: String,
    },
    ClipboardWrite(String),
    ActiveProcessClose,
}

struct RoutedSurface {
    identity: SurfaceIdentity,
    state: SharedState,
    host: Retained<GhosttyHostView>,
}

struct SurfaceSpawn {
    identity: SurfaceIdentity,
    working_directory: PathBuf,
    command: Option<String>,
    environment: Vec<SurfaceEnvironmentVariable>,
    standalone_socket_path: Option<String>,
}

pub struct GhosttyBackend {
    app: Option<GhosttyApp>,
    compositor: Option<NativeViewCompositor>,
    resources: Option<AppResources>,
    config: Option<GhosttyConfig>,
    cjk_overlay: Option<TemporaryConfigFile>,
    surfaces: SurfaceRegistry,
    confirmations: SharedConfirmations,
    gate: Rc<ShortcutGate>,
    launch_environment: Vec<SurfaceEnvironmentVariable>,
    transparent_surface: bool,
    color_scheme: Option<TerminalColorScheme>,
    navigation_events: async_channel::Sender<muxy_core::navigation::Direction>,
    navigation_event_receiver: Receiver<muxy_core::navigation::Direction>,
    external_drops: async_channel::Sender<(SurfaceIdentity, muxy_terminal::backend::ExternalDrop)>,
    external_drop_receiver: Receiver<(SurfaceIdentity, muxy_terminal::backend::ExternalDrop)>,
    standalone_shortcuts: async_channel::Sender<()>,
    standalone_shortcut_receiver: Receiver<()>,
}

impl Default for GhosttyBackend {
    fn default() -> Self {
        Self::new()
    }
}

fn standalone_shortcut_combos() -> Vec<KeyCombo> {
    vec![KeyCombo::new("w", COMMAND)]
}

fn ghostty_color_scheme(scheme: TerminalColorScheme) -> ColorScheme {
    match scheme {
        TerminalColorScheme::Light => ColorScheme::Light,
        TerminalColorScheme::Dark => ColorScheme::Dark,
    }
}

impl GhosttyBackend {
    pub fn new() -> Self {
        let (navigation_events, navigation_event_receiver) = async_channel::bounded(32);
        let (external_drops, external_drop_receiver) = async_channel::bounded(32);
        let (standalone_shortcuts, standalone_shortcut_receiver) = async_channel::bounded(8);
        Self {
            app: None,
            compositor: None,
            resources: None,
            config: None,
            cjk_overlay: None,
            surfaces: Rc::new(RefCell::new(HashMap::new())),
            confirmations: Rc::new(RefCell::new(ConfirmationQueue::new())),
            gate: Rc::new(ShortcutGate::new(Vec::new())),
            launch_environment: Vec::new(),
            transparent_surface: false,
            color_scheme: None,
            navigation_events,
            navigation_event_receiver,
            external_drops,
            external_drop_receiver,
            standalone_shortcuts,
            standalone_shortcut_receiver,
        }
    }

    pub fn attach(
        &mut self,
        combos: Vec<KeyCombo>,
        mode: BuildMode,
        socket_path: &Path,
        backdrop: gpui::Rgba,
        window: &mut Window,
    ) -> Result<(), String> {
        self.attach_with(combos, mode, socket_path, backdrop, false, window)
    }

    pub fn attach_standalone(
        &mut self,
        mode: BuildMode,
        socket_path: &Path,
        window: &mut Window,
    ) -> Result<(), String> {
        self.attach_with(
            standalone_shortcut_combos(),
            mode,
            socket_path,
            gpui::transparent_black().into(),
            true,
            window,
        )
    }

    fn attach_with(
        &mut self,
        combos: Vec<KeyCombo>,
        mode: BuildMode,
        socket_path: &Path,
        backdrop: gpui::Rgba,
        transparent_surface: bool,
        window: &mut Window,
    ) -> Result<(), String> {
        if self.app.is_some() {
            return Ok(());
        }
        MainThreadMarker::new()
            .ok_or_else(|| "ghostty must attach on the main thread".to_owned())?;
        let resources = AppResources::discover().map_err(|error| error.to_string())?;
        let launch_environment = development_cli_environment(
            mode,
            &resources.root,
            socket_path,
            std::env::var_os("PATH").as_deref(),
        )?;
        unsafe { std::env::set_var("GHOSTTY_RESOURCES_DIR", &resources.ghostty) };
        let (config, cjk_overlay) =
            load_config(&resources, transparent_surface).map_err(|error| error.to_string())?;
        let owned = config.try_clone().map_err(|error| error.to_string())?;
        let app = GhosttyApp::new(owned).map_err(|error| error.to_string())?;
        if let Some(scheme) = self.color_scheme {
            app.set_color_scheme(ghostty_color_scheme(scheme));
        }
        let compositor =
            NativeViewCompositor::new(window, backdrop).map_err(|error| error.to_string())?;

        self.gate = Rc::new(ShortcutGate::new(combos));
        self.app = Some(app);
        self.compositor = Some(compositor);
        self.resources = Some(resources);
        self.config = Some(config);
        self.cjk_overlay = cjk_overlay;
        self.launch_environment = launch_environment;
        self.transparent_surface = transparent_surface;
        Ok(())
    }

    pub(crate) fn compositor(&self) -> Option<NativeViewCompositor> {
        self.compositor.clone()
    }

    pub fn set_shortcut_combos(&mut self, combos: Vec<KeyCombo>) {
        self.gate = Rc::new(ShortcutGate::new(combos));
        for surface in self.surfaces.borrow().values() {
            surface.host.set_shortcut_gate(self.gate.clone());
        }
    }

    pub fn set_backdrop(&self, backdrop: gpui::Rgba) {
        if let Some(compositor) = &self.compositor {
            compositor.set_backdrop(backdrop);
        }
    }

    pub fn set_color_scheme(&mut self, scheme: TerminalColorScheme) {
        self.color_scheme = Some(scheme);
        let scheme = ghostty_color_scheme(scheme);
        if let Some(app) = &self.app {
            app.set_color_scheme(scheme);
        }
        for surface in self.surfaces.borrow().values() {
            surface.host.set_color_scheme_override(Some(scheme));
        }
    }

    pub fn wakeup_receiver(&self) -> Option<Receiver<()>> {
        self.app.as_ref().map(GhosttyApp::wakeup_receiver)
    }

    pub fn event_receiver(&self) -> Option<Receiver<RuntimeEvent>> {
        self.app.as_ref().map(GhosttyApp::event_receiver)
    }

    pub fn standalone_shortcut_receiver(&self) -> Receiver<()> {
        self.standalone_shortcut_receiver.clone()
    }

    pub fn navigation_event_receiver(&self) -> Receiver<muxy_core::navigation::Direction> {
        self.navigation_event_receiver.clone()
    }

    pub(crate) fn external_drop_receiver(
        &self,
    ) -> Receiver<(SurfaceIdentity, muxy_terminal::backend::ExternalDrop)> {
        self.external_drop_receiver.clone()
    }

    pub(crate) fn inject_staged_external_drop(
        &self,
        identity: SurfaceIdentity,
        dropped: muxy_terminal::backend::ExternalDrop,
    ) -> bool {
        muxy_core::prefs::is_test_process()
            && std::env::var("MUXY_TEST_P7_COMPOSER_CASE").ok().as_deref() == Some("phase-7")
            && std::env::var_os("MUXY_TEST_APPLICATION_SUPPORT_DIRECTORY").is_some()
            && self.external_drops.try_send((identity, dropped)).is_ok()
    }

    pub fn tick(&self) {
        if let Some(app) = &self.app {
            app.tick();
        }
    }

    pub fn reload_config(&mut self) {
        let Some(resources) = &self.resources else {
            return;
        };
        let (config, cjk_overlay) = match load_config(resources, self.transparent_surface) {
            Ok(loaded) => loaded,
            Err(error) => {
                log::error!("Ghostty config reload failed; preserving prior config: {error}");
                return;
            }
        };
        if let Some(app) = &self.app
            && let Ok(owned) = config.try_clone()
        {
            app.replace_config(owned);
        }
        for surface in self.surfaces.borrow().values() {
            surface.host.update_config(&config);
            apply_background_blur_to(self.app.as_ref(), &surface.host);
        }
        self.config = Some(config);
        self.cjk_overlay = cjk_overlay;
    }

    pub fn active_confirmation(&self) -> Option<(TabId, ConfirmationId, ConfirmationKind)> {
        let confirmations = self.confirmations.borrow();
        let request = confirmations.active()?;
        let SurfaceIdentity::Workspace(tab_id) = &request.payload.identity else {
            return None;
        };
        Some((tab_id.clone(), request.id, request.kind))
    }

    pub fn set_overlay_active(&self, active: bool) {
        for surface in self.surfaces.borrow().values() {
            surface.host.set_overlay_active(active);
        }
        if active && let Some(compositor) = &self.compositor {
            compositor.focus_gpui();
        }
    }

    pub fn route(&mut self, event: RuntimeEvent, cx: &mut App) -> Option<RoutedTerminalEvent> {
        match event {
            RuntimeEvent::Action(action) => self.route_action(action, cx),
            RuntimeEvent::ClipboardRead {
                surface_id, token, ..
            } => {
                let surfaces = self.surfaces.borrow();
                let surface = surfaces.get(&surface_id.get())?;
                let text = pasteboard::read_text().unwrap_or_default();
                let _ = surface
                    .host
                    .resolve_clipboard_request(token, Some(&text), false);
                None
            }
            RuntimeEvent::ClipboardReadConfirmation {
                surface_id,
                content,
                token,
                request,
            } => {
                let surfaces = self.surfaces.borrow();
                let surface = surfaces.get(&surface_id.get())?;
                let Some(kind) = confirmation_kind(request) else {
                    let _ = surface.host.resolve_clipboard_request(token, None, true);
                    return None;
                };
                let content = content
                    .map(|content| String::from_utf8_lossy(&content).into_owned())
                    .unwrap_or_default();
                let id = self.confirmations.borrow_mut().enqueue(
                    kind,
                    PendingConfirmation {
                        identity: surface.identity.clone(),
                        payload: ConfirmationPayload::ClipboardRead { token, content },
                    },
                );
                Some(surface.identity.route(SurfaceSignal::Confirm { id, kind }))
            }
            RuntimeEvent::ClipboardWrite {
                surface_id,
                location,
                contents,
                confirm,
            } => {
                if !matches!(
                    location,
                    ClipboardLocation::Standard | ClipboardLocation::Selection
                ) {
                    return None;
                }
                let surfaces = self.surfaces.borrow();
                let surface = surfaces.get(&surface_id.get())?;
                let text = first_plain_text(&contents)?;
                if !confirm {
                    pasteboard::write_text(&text);
                    return None;
                }
                let kind = ConfirmationKind::Osc52Write;
                let id = self.confirmations.borrow_mut().enqueue(
                    kind,
                    PendingConfirmation {
                        identity: surface.identity.clone(),
                        payload: ConfirmationPayload::ClipboardWrite(text),
                    },
                );
                Some(surface.identity.route(SurfaceSignal::Confirm { id, kind }))
            }
            RuntimeEvent::Close {
                surface_id,
                process,
            } => {
                let surfaces = self.surfaces.borrow();
                let surface = surfaces.get(&surface_id.get())?;
                if matches!(process, SurfaceProcessState::Exited) {
                    return Some(surface.identity.route(SurfaceSignal::Exited));
                }
                let kind = ConfirmationKind::ActiveProcessClose;
                let id = self.confirmations.borrow_mut().enqueue(
                    kind,
                    PendingConfirmation {
                        identity: surface.identity.clone(),
                        payload: ConfirmationPayload::ActiveProcessClose,
                    },
                );
                Some(surface.identity.route(SurfaceSignal::Confirm { id, kind }))
            }
        }
    }

    fn route_action(
        &mut self,
        event: ghostty_host::ActionEvent,
        _cx: &mut App,
    ) -> Option<RoutedTerminalEvent> {
        if matches!(&event.action, RuntimeAction::ReloadConfig { .. }) {
            self.reload_config();
            return None;
        }
        if matches!(&event.action, RuntimeAction::DesktopNotification(_)) {
            let surfaces = self.surfaces.borrow();
            return route_desktop_notification(
                runtime_target(event.target),
                &event.action,
                |surface_id| {
                    surfaces
                        .get(&surface_id)
                        .map(|surface| surface.identity.clone())
                },
            );
        }
        if event.target == ActionTarget::App {
            return None;
        }
        let target = runtime_target(event.target);
        let RuntimeTarget::Surface(Some(surface_id)) = target else {
            return None;
        };
        let surfaces = self.surfaces.borrow();
        let surface = surfaces.get(&surface_id)?;
        if let RuntimeAction::Scrollbar(scrollbar) = &event.action {
            let metrics = ScrollbarMetrics::new(scrollbar.total, scrollbar.offset, scrollbar.len);
            let result = surface.state.borrow_mut().apply_event(TerminalActionEvent {
                target,
                action: TerminalStateAction::Scrollbar(metrics),
            });
            if matches!(result, ApplyActionResult::Changed) {
                surface.host.update_scrollbar(metrics);
            }
            return None;
        }
        let result = surface.state.borrow_mut().apply_event(TerminalActionEvent {
            target,
            action: terminal_state_action(&event.action),
        });
        if !matches!(result, ApplyActionResult::Changed) {
            return None;
        }
        match &event.action {
            RuntimeAction::Bell => NSBeep(),
            RuntimeAction::MouseShape(shape) => {
                surface.host.apply_runtime_mouse_cursor(*shape);
            }
            RuntimeAction::MouseVisibility(visibility) => {
                surface.host.apply_runtime_mouse_visibility(*visibility);
            }
            RuntimeAction::OpenUrl(open) if !open.url.is_empty() => {
                return Some(surface.identity.route_open_url(open.url.clone()));
            }
            _ => {}
        }
        let metadata = metadata_of(&surface.state.borrow());
        Some(surface.identity.route(SurfaceSignal::Metadata(metadata)))
    }

    pub fn spawn(
        &mut self,
        tab_id: &TabId,
        directory: PathBuf,
        command: Option<LaunchCommand>,
        context: &PaneLaunchContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Box<dyn AppSurfaceHandle>> {
        let (environment, launch) = self.surface_environment(command)?;
        let environment = apply_pane_context(environment, context);
        self.spawn_surface(
            SurfaceSpawn {
                identity: SurfaceIdentity::Workspace(tab_id.clone()),
                working_directory: directory,
                command: launch,
                environment,
                standalone_socket_path: None,
            },
            window,
            cx,
        )
    }

    pub fn spawn_standalone(
        &mut self,
        context: &StandaloneLaunchContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Box<dyn AppSurfaceHandle>, String> {
        let (environment, launch) = self
            .surface_environment(None)
            .ok_or_else(|| "standalone terminal backend is not attached".to_owned())?;
        let environment = apply_standalone_context(environment, context);
        self.spawn_surface(
            SurfaceSpawn {
                identity: SurfaceIdentity::Standalone,
                working_directory: context.working_directory().to_path_buf(),
                command: launch,
                environment,
                standalone_socket_path: Some(context.socket_path().to_owned()),
            },
            window,
            cx,
        )
        .ok_or_else(|| "failed to create the standalone Ghostty surface".to_owned())
    }

    fn surface_environment(
        &self,
        command: Option<LaunchCommand>,
    ) -> Option<(Vec<SurfaceEnvironmentVariable>, Option<String>)> {
        let resources = self.resources.as_ref()?;
        let mut environment = self.launch_environment.clone();
        environment.push(SurfaceEnvironmentVariable::new(
            "TERMINFO_DIRS",
            resources.terminfo.to_string_lossy().into_owned(),
        ));
        let launch = command.map(|command| {
            environment.push(SurfaceEnvironmentVariable::new(
                "MUXY_STARTUP_COMMAND",
                command.command.clone(),
            ));
            startup_shell_command(&user_shell(), command.keeps_shell_open)
        });
        Some((environment, launch))
    }

    fn spawn_surface(
        &mut self,
        spawn: SurfaceSpawn,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Box<dyn AppSurfaceHandle>> {
        let SurfaceSpawn {
            identity,
            working_directory,
            command,
            environment,
            standalone_socket_path,
        } = spawn;
        let mtm = MainThreadMarker::new()?;
        let app = self.app.as_ref()?;
        let compositor = self.compositor.as_ref()?;
        let host = GhosttyHostView::new(mtm);
        if let Some(scheme) = self.color_scheme {
            host.set_color_scheme_override(Some(ghostty_color_scheme(scheme)));
        }
        let native_view: &NSView = &host;
        let registration = compositor.register(native_view, 0).ok()?;
        registration.sync_frame(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: window.viewport_size(),
        });
        let scheme = host.color_scheme();
        app.set_color_scheme(scheme);
        let options = SurfaceOptions {
            context: SurfaceContext::Window,
            working_directory,
            command,
            environment,
            ..SurfaceOptions::default()
        };
        let attach = || host.attach_surface(app, options);
        let surface_id = match standalone_socket_path.as_deref() {
            Some(socket_path) => with_standalone_process_environment(socket_path, attach),
            None => attach(),
        }
        .ok()?;
        host.set_window_active(window.is_window_active());
        host.set_shortcut_gate(self.gate.clone());
        host.set_app_view(compositor.gpui_view());
        if let Some(config) = &self.config {
            host.update_config(config);
        }
        apply_background_blur(app, &host);
        let shared = Rc::new(RefCell::new(TerminalSurfaceState::new(surface_id.get())));
        self.surfaces.borrow_mut().insert(
            surface_id.get(),
            RoutedSurface {
                identity: identity.clone(),
                state: shared.clone(),
                host: host.clone(),
            },
        );
        let host_events = host.event_receiver();
        let navigation_events = self.navigation_events.clone();
        let external_drops = self.external_drops.clone();
        let standalone_shortcuts = self.standalone_shortcuts.clone();
        let routes_navigation = matches!(identity, SurfaceIdentity::Workspace(_));
        let drop_identity = identity.clone();
        let task = cx.background_spawn(async move {
            while let Ok(event) = host_events.recv().await {
                if event == HostViewEvent::AppShortcut {
                    if !routes_navigation && standalone_shortcuts.send(()).await.is_err() {
                        return;
                    }
                    continue;
                }
                if let HostViewEvent::ExternalDrop(dropped) = &event {
                    if external_drops
                        .send((drop_identity.clone(), dropped.clone()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    continue;
                }
                if !routes_navigation {
                    continue;
                }
                let direction = match event {
                    HostViewEvent::NavigateBack => muxy_core::navigation::Direction::Back,
                    HostViewEvent::NavigateForward => muxy_core::navigation::Direction::Forward,
                    HostViewEvent::ContextMenu(_)
                    | HostViewEvent::Appearance(_)
                    | HostViewEvent::AppShortcut
                    | HostViewEvent::ExternalDrop(_) => continue,
                };
                if navigation_events.send(direction).await.is_err() {
                    return;
                }
            }
        });
        Some(Box::new(GhosttySurfaceHandle {
            registration,
            host,
            identity,
            surface_id: surface_id.get(),
            metadata: SurfaceMetadata::default(),
            state: shared,
            surfaces: self.surfaces.clone(),
            confirmations: self.confirmations.clone(),
            _host_task: task,
        }))
    }
}

pub struct GhosttySurfaceHandle {
    registration: NativeViewRegistration,
    host: Retained<GhosttyHostView>,
    identity: SurfaceIdentity,
    surface_id: u64,
    metadata: SurfaceMetadata,
    state: SharedState,
    surfaces: SurfaceRegistry,
    confirmations: SharedConfirmations,
    _host_task: Task<()>,
}

impl Drop for GhosttySurfaceHandle {
    fn drop(&mut self) {
        self.surfaces.borrow_mut().remove(&self.surface_id);
        let identity = self.identity.clone();
        self.confirmations
            .borrow_mut()
            .discard(|request| request.payload.identity == identity);
    }
}

impl AppSurfaceHandle for GhosttySurfaceHandle {
    fn element(&self, visible: bool) -> AnyElement {
        match self.registration.slot() {
            Some(slot) => slot.visible(visible).size_full().into_any_element(),
            None => div().size_full().into_any_element(),
        }
    }
}

impl TerminalSurfaceHandle for GhosttySurfaceHandle {
    fn set_focused(&self, focused: bool) {
        if focused {
            self.registration.focus();
        }
    }

    fn set_occluded(&self, occluded: bool) {
        self.registration.set_visible(!occluded);
    }

    fn set_pointer_inside(&self, inside: bool) {
        self.host.set_pointer_inside(inside);
    }

    fn set_input_transaction_active(&self, active: bool) {
        self.host.set_input_transaction_active(active);
    }

    fn cancel_input_transaction(&self) {
        self.host.cancel_input_transaction();
    }

    fn has_native_scrollbar(&self) -> bool {
        true
    }

    fn has_selection(&self) -> bool {
        self.host.has_selection()
    }

    fn send_text(&self, text: &str) -> bool {
        self.host.send_text(text)
    }

    fn send_bytes(&self, bytes: &[u8]) -> bool {
        self.host.send_bytes(bytes)
    }

    fn read_screen_text(&self, last_lines: usize) -> Option<String> {
        self.host.read_screen_text(last_lines)
    }

    fn foreground_pid(&self) -> Option<u64> {
        self.host.foreground_pid()
    }

    fn metadata(&self) -> &SurfaceMetadata {
        &self.metadata
    }

    fn perform(&self, action: SurfaceAction) -> bool {
        match action {
            SurfaceAction::Copy => {
                if self.host.binding_action("copy_to_clipboard") {
                    return true;
                }
                match self.host.read_selection() {
                    Ok(Some(text)) => pasteboard::write_text(&text),
                    _ => false,
                }
            }
            SurfaceAction::Paste => self.host.binding_action("paste_from_clipboard"),
            SurfaceAction::SearchStart => {
                let started = self.host.binding_action(&SearchAction::Start.encode());
                if !started {
                    self.state
                        .borrow_mut()
                        .apply_action(TerminalStateAction::SearchStart(None));
                }
                true
            }
            SurfaceAction::SearchEnd => {
                self.host.binding_action(&SearchAction::End.encode());
                self.state
                    .borrow_mut()
                    .apply_action(TerminalStateAction::SearchEnd);
                true
            }
            SurfaceAction::SearchNext => self
                .host
                .binding_action(&SearchAction::Navigate(SearchDirection::Next).encode()),
            SurfaceAction::SearchPrevious => self
                .host
                .binding_action(&SearchAction::Navigate(SearchDirection::Previous).encode()),
            SurfaceAction::SearchQuery(query) => self
                .host
                .binding_action(&SearchAction::Query(query).encode()),
            SurfaceAction::ScrollToRow(row) => {
                self.host.binding_action(&format!("scroll_to_row:{row}"))
            }
            SurfaceAction::ClipboardDecision { id, approved } => {
                self.resolve_confirmation(id, approved)
            }
        }
    }

    fn forward_pointer(&self, input: PointerInput) -> bool {
        match input {
            PointerInput::Moved { x, y, modifiers } => {
                self.host
                    .forward_pointer_position(x, y, runtime_modifiers(modifiers));
                true
            }
            PointerInput::Down {
                x,
                y,
                button,
                modifiers,
                ..
            } => {
                if !self.claims_press(button, modifiers) {
                    return false;
                }
                self.host.forward_pointer_button(
                    x,
                    y,
                    RuntimeMouseButtonState::Press,
                    runtime_button(button),
                    runtime_modifiers(modifiers),
                )
            }
            PointerInput::Up {
                x,
                y,
                button,
                modifiers,
            } => self.host.forward_pointer_button(
                x,
                y,
                RuntimeMouseButtonState::Release,
                runtime_button(button),
                runtime_modifiers(modifiers),
            ),
        }
    }

    fn apply(&mut self, signal: SurfaceSignal) -> bool {
        match signal {
            SurfaceSignal::Metadata(metadata) => {
                if metadata == self.metadata {
                    return false;
                }
                self.metadata = metadata;
                true
            }
            SurfaceSignal::DesktopNotification { .. } => false,
            SurfaceSignal::Exited => true,
            SurfaceSignal::Confirm { .. } => false,
        }
    }

    fn refresh(&mut self) -> bool {
        let metadata = metadata_of(&self.state.borrow());
        if metadata == self.metadata {
            return false;
        }
        self.metadata = metadata;
        true
    }

    fn needs_confirm_close(&self) -> bool {
        self.host.needs_confirm_quit()
    }

    fn request_close(&self) {
        self.host.request_close();
    }
}

impl GhosttySurfaceHandle {
    fn claims_press(&self, button: PointerButton, modifiers: PointerModifiers) -> bool {
        match button {
            PointerButton::Left if modifiers.platform => {
                self.state.borrow().link_under_pointer.is_some()
            }
            PointerButton::Right => self.host.mouse_captured() && !modifiers.shift,
            _ => true,
        }
    }

    fn resolve_confirmation(&self, id: ConfirmationId, approved: bool) -> bool {
        let decision = if approved {
            ConfirmationDecision::Approve
        } else {
            ConfirmationDecision::Deny
        };
        let Ok(resolved) = self.confirmations.borrow_mut().decide(id, decision) else {
            return false;
        };
        match resolved.payload.payload {
            ConfirmationPayload::ClipboardRead { token, content } => {
                let content = approved.then_some(content.as_str());
                let surfaces = self.surfaces.borrow();
                let Some(surface) = surfaces.get(&token.surface_id().get()) else {
                    return true;
                };
                let _ = surface.host.resolve_clipboard_request(token, content, true);
            }
            ConfirmationPayload::ClipboardWrite(text) => {
                if approved {
                    pasteboard::write_text(&text);
                }
            }
            ConfirmationPayload::ActiveProcessClose => {}
        }
        true
    }
}

fn load_config(
    resources: &AppResources,
    transparent_surface: bool,
) -> Result<(GhosttyConfig, Option<TemporaryConfigFile>), ghostty_host::ConfigError> {
    let paths = ConfigPaths::new(&resources.defaults_config)
        .with_user_override(Some(muxy_core::store::ghostty_conf::path()));
    let user_config = paths
        .user_override()
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    let overlay = config_text_for_user(&user_config, resolve_system_font_family).and_then(|text| {
        match TemporaryConfigFile::create(&text) {
            Ok(file) => Some(file),
            Err(error) => {
                log::warn!("could not create temporary CJK fallback config: {error}");
                None
            }
        }
    });
    let paths = paths.with_generated_overlay(overlay.as_ref().map(|file| file.path().to_owned()));
    let config = GhosttyConfig::load(paths)?;
    let config = if transparent_surface {
        config.with_overlay_file(&resources.transparent_surface_config)?
    } else {
        config
    };
    for diagnostic in config.diagnostics() {
        log::warn!("Ghostty config: {diagnostic}");
    }
    Ok((config, overlay))
}

fn confirmation_kind(request: ClipboardRequest) -> Option<ConfirmationKind> {
    match request {
        ClipboardRequest::Paste => Some(ConfirmationKind::Paste),
        ClipboardRequest::Osc52Read => Some(ConfirmationKind::Osc52Read),
        ClipboardRequest::Osc52Write => Some(ConfirmationKind::Osc52Write),
        ClipboardRequest::Unknown(_) => None,
    }
}

fn first_plain_text(contents: &[ClipboardContent]) -> Option<String> {
    contents.iter().find_map(|content| {
        let mime = content.mime.as_deref()?;
        if !mime.eq_ignore_ascii_case(b"text/plain") {
            return None;
        }
        content
            .data
            .as_deref()
            .map(|data| String::from_utf8_lossy(data).into_owned())
    })
}

fn metadata_of(state: &TerminalSurfaceState) -> SurfaceMetadata {
    SurfaceMetadata {
        title: Some(state.effective_title().to_owned()),
        working_directory: state.working_directory.clone(),
        bell_generation: state.bell_count,
        progress: progress_of(state.progress.as_ref()),
        search_totals: SearchTotals {
            active: state.search.visible,
            total: state.search.total,
            selected: state.search.selected,
        },
        scrollbar: state.scrollbar,
    }
}

fn progress_of(progress: Option<&TerminalProgress>) -> SurfaceProgress {
    match progress {
        None => SurfaceProgress::default(),
        Some(progress) => SurfaceProgress {
            kind: Some(match progress.kind {
                ProgressKind::Set => SurfaceProgressKind::Set,
                ProgressKind::Error => SurfaceProgressKind::Error,
                ProgressKind::Indeterminate => SurfaceProgressKind::Indeterminate,
                ProgressKind::Paused => SurfaceProgressKind::Paused,
            }),
            value: progress.percent.map(|percent| f32::from(percent) / 100.0),
        },
    }
}

fn runtime_button(button: PointerButton) -> RuntimeMouseButton {
    match button {
        PointerButton::Left => RuntimeMouseButton::Left,
        PointerButton::Right => RuntimeMouseButton::Right,
        PointerButton::Middle => RuntimeMouseButton::Middle,
        PointerButton::Other => RuntimeMouseButton::Unknown,
    }
}

fn runtime_modifiers(modifiers: PointerModifiers) -> RuntimeModifiers {
    let mut value = RuntimeModifiers::NONE;
    if modifiers.shift {
        value |= RuntimeModifiers::SHIFT;
    }
    if modifiers.control {
        value |= RuntimeModifiers::CONTROL;
    }
    if modifiers.alt {
        value |= RuntimeModifiers::ALT;
    }
    if modifiers.platform {
        value |= RuntimeModifiers::SUPER;
    }
    value
}

fn runtime_target(target: ActionTarget) -> RuntimeTarget {
    match target {
        ActionTarget::App => RuntimeTarget::App,
        ActionTarget::Surface(id) => RuntimeTarget::Surface(id.map(|id| id.get())),
        ActionTarget::Unknown(tag) => RuntimeTarget::Unknown(tag),
    }
}

fn terminal_mouse_shape(shape: RuntimeMouseShape) -> MouseShape {
    match shape {
        RuntimeMouseShape::Default => MouseShape::Default,
        RuntimeMouseShape::ContextMenu => MouseShape::ContextMenu,
        RuntimeMouseShape::Help => MouseShape::Help,
        RuntimeMouseShape::Pointer => MouseShape::Pointer,
        RuntimeMouseShape::Progress => MouseShape::Progress,
        RuntimeMouseShape::Wait => MouseShape::Wait,
        RuntimeMouseShape::Cell => MouseShape::Cell,
        RuntimeMouseShape::Crosshair => MouseShape::Crosshair,
        RuntimeMouseShape::Text => MouseShape::Text,
        RuntimeMouseShape::VerticalText => MouseShape::VerticalText,
        RuntimeMouseShape::Alias => MouseShape::Alias,
        RuntimeMouseShape::Copy => MouseShape::Copy,
        RuntimeMouseShape::Move => MouseShape::Move,
        RuntimeMouseShape::NoDrop => MouseShape::NoDrop,
        RuntimeMouseShape::NotAllowed => MouseShape::NotAllowed,
        RuntimeMouseShape::Grab => MouseShape::Grab,
        RuntimeMouseShape::Grabbing => MouseShape::Grabbing,
        RuntimeMouseShape::AllScroll => MouseShape::AllScroll,
        RuntimeMouseShape::ColumnResize => MouseShape::ColumnResize,
        RuntimeMouseShape::RowResize => MouseShape::RowResize,
        RuntimeMouseShape::NorthResize => MouseShape::NorthResize,
        RuntimeMouseShape::EastResize => MouseShape::EastResize,
        RuntimeMouseShape::SouthResize => MouseShape::SouthResize,
        RuntimeMouseShape::WestResize => MouseShape::WestResize,
        RuntimeMouseShape::NorthEastResize => MouseShape::NorthEastResize,
        RuntimeMouseShape::NorthWestResize => MouseShape::NorthWestResize,
        RuntimeMouseShape::SouthEastResize => MouseShape::SouthEastResize,
        RuntimeMouseShape::SouthWestResize => MouseShape::SouthWestResize,
        RuntimeMouseShape::EastWestResize => MouseShape::EastWestResize,
        RuntimeMouseShape::NorthSouthResize => MouseShape::NorthSouthResize,
        RuntimeMouseShape::NorthEastSouthWestResize => MouseShape::NorthEastSouthWestResize,
        RuntimeMouseShape::NorthWestSouthEastResize => MouseShape::NorthWestSouthEastResize,
        RuntimeMouseShape::ZoomIn => MouseShape::ZoomIn,
        RuntimeMouseShape::ZoomOut => MouseShape::ZoomOut,
    }
}

fn route_desktop_notification(
    target: RuntimeTarget,
    action: &RuntimeAction,
    identity_for_surface: impl Fn(u64) -> Option<SurfaceIdentity>,
) -> Option<RoutedTerminalEvent> {
    let RuntimeTarget::Surface(Some(surface_id)) = target else {
        return None;
    };
    let RuntimeAction::DesktopNotification(notification) = action else {
        return None;
    };
    Some(
        identity_for_surface(surface_id)?.route(SurfaceSignal::DesktopNotification {
            title: notification.title.clone(),
            body: notification.body.clone(),
        }),
    )
}

fn terminal_state_action(action: &RuntimeAction) -> TerminalStateAction {
    match action {
        RuntimeAction::SetTitle(title) => TerminalStateAction::SetTitle(title.clone()),
        RuntimeAction::SetTabTitle(title) => TerminalStateAction::SetTabTitle(title.clone()),
        RuntimeAction::WorkingDirectory(path) => {
            TerminalStateAction::WorkingDirectory(path.to_string_lossy().into_owned())
        }
        RuntimeAction::Bell => TerminalStateAction::Bell,
        RuntimeAction::MouseShape(shape) => {
            TerminalStateAction::MouseShape(terminal_mouse_shape(*shape))
        }
        RuntimeAction::MouseVisibility(visibility) => {
            TerminalStateAction::MouseVisibility(matches!(visibility, MouseVisibility::Visible))
        }
        RuntimeAction::MouseOverLink(link) => TerminalStateAction::MouseOverLink(link.clone()),
        RuntimeAction::OpenUrl(open) => TerminalStateAction::OpenUrl(open.url.clone()),
        RuntimeAction::SearchStart(needle) => {
            TerminalStateAction::SearchStart((!needle.is_empty()).then(|| needle.clone()))
        }
        RuntimeAction::SearchEnd => TerminalStateAction::SearchEnd,
        RuntimeAction::SearchTotal(total) => TerminalStateAction::SearchTotal(*total),
        RuntimeAction::SearchSelected(selected) => TerminalStateAction::SearchSelected(*selected),
        RuntimeAction::Progress(report) => TerminalStateAction::Progress(match report.state {
            ProgressState::Remove => None,
            ProgressState::Set => Some(TerminalProgress::new(ProgressKind::Set, report.percent)),
            ProgressState::Error => {
                Some(TerminalProgress::new(ProgressKind::Error, report.percent))
            }
            ProgressState::Indeterminate => Some(TerminalProgress::new(
                ProgressKind::Indeterminate,
                report.percent,
            )),
            ProgressState::Pause => {
                Some(TerminalProgress::new(ProgressKind::Paused, report.percent))
            }
        }),
        RuntimeAction::Scrollbar(scrollbar) => TerminalStateAction::Scrollbar(
            ScrollbarMetrics::new(scrollbar.total, scrollbar.offset, scrollbar.len),
        ),
        RuntimeAction::ColorChange(change) => TerminalStateAction::ColorChange {
            slot: match change.kind {
                ColorKind::Foreground => TerminalColorSlot::Foreground,
                ColorKind::Background => TerminalColorSlot::Background,
                ColorKind::Cursor => TerminalColorSlot::Cursor,
                ColorKind::Palette(index) => TerminalColorSlot::Palette(index),
            },
            color: TerminalColor {
                red: change.color.red,
                green: change.color.green,
                blue: change.color.blue,
            },
        },
        RuntimeAction::Unsupported { tag } => TerminalStateAction::Unsupported { tag: *tag },
        RuntimeAction::DesktopNotification(_) => {
            unreachable!("desktop notifications are routed before terminal state")
        }
        RuntimeAction::ReloadConfig { .. } => {
            unreachable!("config reloads are handled before the surface state")
        }
    }
}

fn apply_background_blur(app: &GhosttyApp, host: &GhosttyHostView) {
    let Some(window) = host.window() else {
        return;
    };
    let pointer = NonNull::from(&*window).cast();
    unsafe { app.set_window_background_blur(pointer) };
}

fn apply_background_blur_to(app: Option<&GhosttyApp>, host: &GhosttyHostView) {
    if let Some(app) = app {
        apply_background_blur(app, host);
    }
}

impl GhosttyBackend {
    pub fn set_window_active(&self, active: bool) {
        for surface in self.surfaces.borrow().values() {
            surface.host.set_window_active(active);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_terminal_standalone_surface_reserves_command_w_for_the_app() {
        assert_eq!(
            standalone_shortcut_combos(),
            vec![KeyCombo::new("w", COMMAND)]
        );
    }

    #[test]
    fn quick_terminal_backend_retains_its_explicit_color_scheme_before_attachment() {
        let mut backend = GhosttyBackend::new();
        backend.set_color_scheme(TerminalColorScheme::Dark);
        assert_eq!(backend.color_scheme, Some(TerminalColorScheme::Dark));
        backend.set_color_scheme(TerminalColorScheme::Light);
        assert_eq!(backend.color_scheme, Some(TerminalColorScheme::Light));
    }

    #[test]
    fn desktop_notification_routes_only_exact_live_surface_targets_without_metadata() {
        let action = RuntimeAction::DesktopNotification(ghostty_host::DesktopNotification {
            title: "Done".to_owned(),
            body: "Ready".to_owned(),
        });
        let routed =
            route_desktop_notification(RuntimeTarget::Surface(Some(7)), &action, |surface_id| {
                (surface_id == 7).then(|| SurfaceIdentity::Workspace("PANE".to_owned()))
            });
        assert_eq!(
            routed,
            Some(RoutedTerminalEvent::Workspace(
                "PANE".to_owned(),
                SurfaceSignal::DesktopNotification {
                    title: "Done".to_owned(),
                    body: "Ready".to_owned(),
                }
            ))
        );
        assert_eq!(
            route_desktop_notification(RuntimeTarget::Surface(Some(7)), &action, |_| {
                Some(SurfaceIdentity::Standalone)
            }),
            Some(RoutedTerminalEvent::Standalone(
                SurfaceSignal::DesktopNotification {
                    title: "Done".to_owned(),
                    body: "Ready".to_owned(),
                }
            ))
        );
        for target in [
            RuntimeTarget::App,
            RuntimeTarget::Surface(None),
            RuntimeTarget::Surface(Some(8)),
            RuntimeTarget::Unknown(99),
        ] {
            assert!(
                route_desktop_notification(target, &action, |surface_id| {
                    (surface_id == 7).then(|| SurfaceIdentity::Workspace("PANE".to_owned()))
                })
                .is_none()
            );
        }
        assert!(
            route_desktop_notification(
                RuntimeTarget::Surface(Some(7)),
                &RuntimeAction::Bell,
                |_| Some(SurfaceIdentity::Workspace("PANE".to_owned()))
            )
            .is_none()
        );
    }

    #[test]
    fn development_cli_environment_targets_the_current_bundle_and_socket() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("MuxyTests.app");
        let resources = app.join("Contents/Resources");
        let bin = resources.join("muxy-dev-bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("muxy"), b"launcher").unwrap();
        let socket = directory.path().join(
            muxy_core::environment::RuntimePathPolicy::new(BuildMode::Development)
                .main_socket_filename(),
        );
        let inherited = OsStr::new("/usr/local/bin:/usr/bin:/bin");
        let environment = development_cli_environment(
            BuildMode::Development,
            &resources,
            &socket,
            Some(inherited),
        )
        .unwrap();
        let value = |key: &str| {
            environment
                .iter()
                .find(|variable| variable.key == key)
                .map(|variable| variable.value.as_str())
                .unwrap()
        };
        let paths = std::env::split_paths(OsStr::new(value("PATH"))).collect::<Vec<_>>();
        assert_eq!(paths[0], bin);
        assert_eq!(
            paths[1..],
            [
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
            ]
        );
        assert_eq!(value("MUXY_DEVELOPMENT_CLI_BIN"), bin.to_string_lossy());
        assert_eq!(value("MUXY_DEVELOPMENT_APP_PATH"), app.to_string_lossy());
        assert_eq!(
            value("MUXY_DEVELOPMENT_SOCKET_PATH"),
            socket.to_string_lossy()
        );
        assert_eq!(value("MUXY_DEVELOPMENT_VERSION"), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn pane_context_overrides_stale_values_and_removes_hook_variables() {
        let context = PaneLaunchContext::new(
            "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE",
            "11111111-2222-4333-8444-555555555555",
            "66666666-7777-4888-8999-AAAAAAAAAAAA",
            "/tmp/selected.socket",
        );
        let environment = apply_pane_context(
            vec![
                SurfaceEnvironmentVariable::new("MUXY_PANE_ID", "stale"),
                SurfaceEnvironmentVariable::new("MUXY_SOCKET_PATH", "/tmp/stale.sock"),
                SurfaceEnvironmentVariable::new("MUXY_HOOK_BIN", "/tmp/hook"),
                SurfaceEnvironmentVariable::new("MUXY_HOOK_SCRIPT", "/tmp/hook.sh"),
                SurfaceEnvironmentVariable::new("TERMINFO_DIRS", "/tmp/terminfo"),
            ],
            &context,
        );
        let value = |key: &str| {
            environment
                .iter()
                .find(|variable| variable.key == key)
                .map(|variable| variable.value.as_str())
        };

        assert_eq!(
            value("MUXY_PANE_ID"),
            Some("AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE")
        );
        assert_eq!(value("MUXY_SOCKET_PATH"), Some("/tmp/selected.socket"));
        assert_eq!(value("TERMINFO_DIRS"), Some("/tmp/terminfo"));
        assert_eq!(value("MUXY_HOOK_BIN"), None);
        assert_eq!(value("MUXY_HOOK_SCRIPT"), None);
    }

    #[test]
    fn standalone_context_scrubs_workspace_and_hook_values() {
        let context = StandaloneLaunchContext::new(
            PathBuf::from("/tmp/home"),
            Path::new("/tmp/selected.socket"),
        );
        let environment = apply_standalone_context(
            vec![
                SurfaceEnvironmentVariable::new("MUXY_PANE_ID", "stale-pane"),
                SurfaceEnvironmentVariable::new("MUXY_PROJECT_ID", "stale-project"),
                SurfaceEnvironmentVariable::new("MUXY_WORKTREE_ID", "stale-worktree"),
                SurfaceEnvironmentVariable::new("MUXY_SOCKET_PATH", "/tmp/stale.sock"),
                SurfaceEnvironmentVariable::new("MUXY_HOOK_BIN", "/tmp/hook"),
                SurfaceEnvironmentVariable::new("MUXY_HOOK_SCRIPT", "/tmp/hook.sh"),
                SurfaceEnvironmentVariable::new("TERMINFO_DIRS", "/tmp/terminfo"),
            ],
            &context,
        );
        let value = |key: &str| {
            environment
                .iter()
                .find(|variable| variable.key == key)
                .map(|variable| variable.value.as_str())
        };

        assert_eq!(context.working_directory(), Path::new("/tmp/home"));
        assert_eq!(value("MUXY_SOCKET_PATH"), Some("/tmp/selected.socket"));
        assert_eq!(value("TERMINFO_DIRS"), Some("/tmp/terminfo"));
        for key in [
            "MUXY_PANE_ID",
            "MUXY_PROJECT_ID",
            "MUXY_WORKTREE_ID",
            "MUXY_HOOK_BIN",
            "MUXY_HOOK_SCRIPT",
        ] {
            assert_eq!(value(key), None);
        }
    }

    #[test]
    fn production_terminal_environment_does_not_inject_the_development_cli() {
        assert!(
            development_cli_environment(
                BuildMode::Production,
                Path::new("/missing/resources"),
                Path::new("/missing/socket"),
                Some(OsStr::new("/usr/bin")),
            )
            .unwrap()
            .is_empty()
        );
    }
}
