mod categories;
mod commands;
mod json_editor;
pub mod theme_picker;

pub use muxy_core::settings_catalog::Category;

use muxy_core::settings_catalog;
use muxy_ui::controls;

use crate::quick_terminal::runtime::QuickTerminalRuntime;
use crate::views::shortcut_editor::{ShortcutEditor, ShortcutEditorEvent};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, BorrowAppContext, Bounds, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, Hsla, InteractiveElement, IntoElement, KeyBinding,
    KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, actions,
    div, px,
};
use muxy_core::prefs::{Prefs, settings};
use muxy_core::shortcuts::{KeyCombo, ShortcutMap};
use muxy_core::store::CommandShortcuts;
use muxy_ui::command_popover::{
    CommandPopover, CommandPopoverConfig, CommandPopoverDensity, CommandPopoverEvent,
    CommandPopoverItem, CommandPopoverPresentation, CommandPopoverRow, CommandPopoverStatus,
    CommandPopoverTab,
};
use muxy_ui::components::SymbolGlyph;
use muxy_ui::text_input::{self, InputEvent, InputStyle, TextInput};
use muxy_ui::theme::{Appearance, Metrics, Theme};
use serde_json::Value;
use std::collections::HashMap;
use theme_picker::{ThemeBrowser, ThemeBrowserEvent, ThemeMode};

const PANEL_WIDTH: f32 = 980.0;
const PANEL_HEIGHT: f32 = 680.0;
const SIDEBAR_WIDTH: f32 = 210.0;
const MINIMUM_PANEL_SIDE: f32 = 320.0;
const HEADER_HEIGHT: f32 = 56.0;
const ROUTE_KEY: &str = "muxy.settings.selectedRoute";

pub const KEY_CONTEXT: &str = "Settings";

pub fn category_supported(category: Category) -> bool {
    cfg!(target_os = "macos") || category != Category::QuickTerminal
}

fn normalized_category(category: Category) -> Category {
    if category_supported(category) {
        category
    } else {
        Category::General
    }
}

actions!(settings, [Dismiss]);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("cmd-w", Dismiss, Some(KEY_CONTEXT))]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Chrome,
    Scale,
    Theme,
    Shortcuts,
    CommandShortcuts,
    All,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsEvent {
    Dismiss,
    Applied(Effect),
    SetDesktopNotifications(bool),
    PreviewNotificationSound(String),
}

fn desktop_notification_event(enabled: bool, pending: bool) -> SettingsEvent {
    SettingsEvent::SetDesktopNotifications(if pending { false } else { !enabled })
}

fn notification_sound_event(name: &str) -> Option<SettingsEvent> {
    settings::NOTIFICATION_SOUNDS
        .contains(&name)
        .then(|| SettingsEvent::PreviewNotificationSound(name.to_owned()))
}

const CHROME_KEYS: [&str; 13] = [
    "muxy.showStatusBar",
    "muxy.showTopBarActions",
    "muxy.showProjectSearch",
    "muxy.tabs.maxWidth",
    "muxy.sidebarCollapsedStyle",
    "muxy.sidebarExpandedStyle",
    "muxy.tips.visible",
    "muxy.showHomeProject",
    "muxy.projectSortMode",
    "muxy.projects.keepOpenWhenNoTabs",
    "muxy.projectPicker.defaultDirectory",
    "muxy.activeSidebar",
    "muxy.defaultFileOpener",
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtensionSettingsOptions {
    pub sidebars: Vec<(String, String)>,
    pub file_openers: Vec<(String, String)>,
}

#[derive(Clone, Copy)]
pub struct SliderSpec {
    pub key: &'static str,
    pub min: f32,
    pub max: f32,
    pub integral: bool,
    pub zero_at_maximum: bool,
}

impl SliderSpec {
    fn stored(self, value: f32) -> f32 {
        if self.zero_at_maximum && value >= self.max {
            return 0.0;
        }
        value.clamp(self.min, self.max)
    }
}

struct SliderDrag {
    spec: SliderSpec,
    bounds: Bounds<Pixels>,
    value: f32,
}

#[derive(Clone)]
pub enum SettingsPickerTarget {
    Setting,
    NotificationSound,
    Editor(String),
    Number(Vec<(String, f64)>),
}

pub struct Field {
    pub id: String,
    pub value: String,
    pub placeholder: String,
    pub monospaced: bool,
    pub multiline: bool,
}

pub struct SettingsModal {
    route: Category,
    search: Entity<TextInput>,
    query: String,
    picker: Option<(String, Entity<CommandPopover>)>,
    picker_subscription: Option<Subscription>,
    sidebar_scroll: ScrollHandle,
    content_scroll: ScrollHandle,
    theme: Theme,
    metrics: Metrics,
    fields: HashMap<String, Entity<TextInput>>,
    field_focus: Option<String>,
    selections: HashMap<String, String>,
    drag: Option<SliderDrag>,
    appearance: Appearance,
    browser: Option<(String, Entity<ThemeBrowser>)>,
    shortcuts: ShortcutMap,
    editor: Option<Entity<ShortcutEditor>>,
    commands: CommandShortcuts,
    armed_command: Option<String>,
    added_command: Option<String>,
    command_conflict: Option<String>,
    delete_all_countdown: Option<u8>,
    desktop_authorization_pending: bool,
    quick_terminal_recording: Option<crate::quick_terminal::ShortcutRecording>,
    quick_terminal_recording_generation: u64,
    extension_options: ExtensionSettingsOptions,
    errors: HashMap<String, String>,
    json_error: Option<String>,
    json_status: Option<String>,
    focus_handle: FocusHandle,
    focused: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsModal {}

impl Focusable for SettingsModal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SettingsModal {
    pub fn new(
        theme: Theme,
        metrics: Metrics,
        appearance: Appearance,
        extension_options: ExtensionSettingsOptions,
        cx: &mut Context<Self>,
    ) -> Self {
        let style = InputStyle::field(&theme, &metrics);
        let search = cx.new(|cx| {
            TextInput::new(style, cx)
                .with_key_context(text_input::BARE_CONTEXT)
                .with_placeholder("Search settings")
        });
        let subscription = cx.subscribe(&search, |modal: &mut Self, input, event, cx| {
            if matches!(event, InputEvent::Changed) {
                modal.query = input.read(cx).text().to_owned();
                modal.close_picker(cx);
                if !settings_catalog::category_matches(modal.route, &modal.query)
                    && let Some(first) = modal.visible_categories().first()
                {
                    modal.route = *first;
                }
                cx.notify();
            }
        });

        let route = normalized_category(
            Category::parse_route(&settings::string_value(ROUTE_KEY, "builtin.general"))
                .unwrap_or(Category::General),
        );

        Self {
            route,
            search,
            query: String::new(),
            picker: None,
            picker_subscription: None,
            sidebar_scroll: ScrollHandle::default(),
            content_scroll: ScrollHandle::default(),
            theme,
            metrics,
            fields: HashMap::new(),
            field_focus: None,
            selections: HashMap::new(),
            drag: None,
            appearance,
            browser: None,
            shortcuts: ShortcutMap::load(),
            editor: None,
            commands: CommandShortcuts::load(),
            armed_command: None,
            added_command: None,
            command_conflict: None,
            delete_all_countdown: None,
            desktop_authorization_pending: false,
            quick_terminal_recording: None,
            quick_terminal_recording_generation: 0,
            extension_options,
            errors: HashMap::new(),
            json_error: None,
            json_status: None,
            focus_handle: cx.focus_handle(),
            focused: false,
            _subscriptions: vec![subscription],
        }
    }

    pub fn style(&self) -> controls::Style<'_> {
        controls::Style {
            theme: &self.theme,
            metrics: &self.metrics,
        }
    }

    pub fn extension_options(&self) -> &ExtensionSettingsOptions {
        &self.extension_options
    }

    pub fn set_appearance(&mut self, theme: Theme, metrics: Metrics, cx: &mut Context<Self>) {
        self.theme = theme.clone();
        self.metrics = metrics;
        let style = self.input_style();
        self.search
            .update(cx, |input, cx| input.set_style(style, cx));
        for field in self.fields.values() {
            field.update(cx, |input, cx| input.set_style(style, cx));
        }
        if let Some((_, browser)) = &self.browser {
            browser.update(cx, |browser, cx| {
                browser.set_appearance(theme.clone(), metrics, cx);
            });
        }
        if let Some((_, picker)) = &self.picker {
            picker.update(cx, |picker, cx| {
                picker.set_appearance(theme.clone(), metrics, cx);
            });
        }
        if let Some(editor) = &self.editor {
            editor.update(cx, |editor, cx| {
                editor.set_appearance(theme, metrics, cx);
            });
        }
        cx.notify();
    }

    fn input_style(&self) -> InputStyle {
        InputStyle::field(&self.theme, &self.metrics)
    }

    pub fn field(&self, id: &str) -> Option<&Entity<TextInput>> {
        self.fields.get(id)
    }

    pub fn selection(&self, key: &str) -> Option<&str> {
        self.selections.get(key).map(String::as_str)
    }

    pub fn set_selection(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        self.selections.insert(key.to_owned(), value.to_owned());
        self.close_picker(cx);
        cx.notify();
    }

    pub fn field_text(&self, id: &str, cx: &App) -> String {
        self.fields
            .get(id)
            .map(|input| input.read(cx).text().to_owned())
            .unwrap_or_default()
    }

    fn prepare_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut specs = categories::fields(self, self.route);
        for spec in &specs {
            if self.fields.contains_key(&spec.id) {
                continue;
            }
            let style = self.input_style();
            let placeholder = spec.placeholder.clone();
            let monospaced = spec.monospaced;
            let rows = spec.multiline;
            let text = spec.value.clone();
            let input = cx.new(|cx| {
                let mut input = TextInput::new(style, cx)
                    .with_placeholder(placeholder)
                    .with_text(text);
                if monospaced {
                    input = input.with_font_family("Menlo");
                }
                if rows {
                    input = input.multiline();
                }
                input
            });
            let id = spec.id.clone();
            let subscription =
                cx.subscribe(
                    &input,
                    move |modal: &mut Self, input, event, cx| match event {
                        InputEvent::Submitted => {
                            let text = input.read(cx).text().to_owned();
                            categories::commit_field(modal, &id, &text, cx);
                        }
                        InputEvent::Changed => {
                            let text = input.read(cx).text().to_owned();
                            if categories::commits_on_change(&id) {
                                categories::commit_field(modal, &id, &text, cx);
                            }
                            modal.field_changed(&id, text, cx);
                        }
                        InputEvent::Cancelled => cx.notify(),
                    },
                );
            self._subscriptions.push(subscription);
            self.fields.insert(spec.id.clone(), input);
        }

        let focused = specs.iter().map(|spec| spec.id.clone()).find(|id| {
            self.fields
                .get(id)
                .is_some_and(|input| input.focus_handle(cx).is_focused(window))
        });
        if self.field_focus != focused
            && let Some(previous) = self.field_focus.take()
            && let Some(input) = self.fields.get(&previous)
        {
            let text = input.read(cx).text().to_owned();
            categories::commit_field(self, &previous, &text, cx);
            specs = categories::fields(self, self.route);
        }
        self.field_focus = focused;

        for spec in &specs {
            if spec.multiline || self.field_focus.as_deref() == Some(spec.id.as_str()) {
                continue;
            }
            let Some(input) = self.fields.get(&spec.id) else {
                continue;
            };
            if input.read(cx).text() != spec.value {
                let value = spec.value.clone();
                input.update(cx, |input, cx| input.set_text(value, cx));
            }
        }
    }

    fn prepare_editor(&mut self, cx: &mut Context<Self>) {
        if self.route != Category::Shortcuts || self.editor.is_some() {
            return;
        }
        let bindings = self.binding_list();
        let query = self.query.clone();
        let theme = self.theme.clone();
        let metrics = self.metrics;
        let editor = cx.new(|cx| ShortcutEditor::new(bindings, query, theme, metrics, cx));
        let subscription =
            cx.subscribe(&editor, |modal: &mut Self, editor, event, cx| match event {
                ShortcutEditorEvent::Save { action, combo } => {
                    if let Err(error) = modal.validate_quick_terminal_reverse_conflict(combo, cx) {
                        let bindings = modal.binding_list();
                        editor.update(cx, |editor, cx| {
                            editor.apply(bindings, cx);
                            editor.set_external_error(Some(error), cx);
                        });
                        return;
                    }
                    modal.shortcuts.set(*action, combo.clone());
                    editor.update(cx, |editor, cx| editor.set_external_error(None, cx));
                    modal.persist_shortcuts(cx);
                }
                ShortcutEditorEvent::ResetAll => {
                    let mut replacement = modal.shortcuts.clone();
                    replacement.reset_to_defaults();
                    if let Err(error) = modal.validate_shortcut_map(&replacement, cx) {
                        let bindings = modal.binding_list();
                        editor.update(cx, |editor, cx| {
                            editor.apply(bindings, cx);
                            editor.set_external_error(Some(error), cx);
                        });
                        return;
                    }
                    modal.shortcuts = replacement;
                    let bindings = modal.binding_list();
                    editor.update(cx, |editor, cx| {
                        editor.apply(bindings, cx);
                        editor.set_external_error(None, cx);
                    });
                    modal.persist_shortcuts(cx);
                }
                ShortcutEditorEvent::Dismiss => cx.emit(SettingsEvent::Dismiss),
            });
        self._subscriptions.push(subscription);
        self.editor = Some(editor);
    }

    fn binding_list(&self) -> Vec<(muxy_core::shortcuts::ShortcutAction, KeyCombo)> {
        muxy_core::shortcuts::modelled_actions()
            .into_iter()
            .map(|action| (action, self.shortcuts.combo(action).clone()))
            .collect()
    }

    fn persist_shortcuts(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.shortcuts.save() {
            log::warn!("failed to write keybindings.json: {error}");
            self.shortcuts = ShortcutMap::load();
            let bindings = self.binding_list();
            if let Some(editor) = self.editor.clone() {
                let message = format!("Failed to save app shortcuts: {error}");
                editor.update(cx, |editor, cx| {
                    editor.apply(bindings, cx);
                    editor.set_external_error(Some(message), cx);
                });
            }
            cx.notify();
            return;
        }
        if let Some(editor) = self.editor.clone() {
            editor.update(cx, |editor, cx| editor.set_external_error(None, cx));
        }
        cx.emit(SettingsEvent::Applied(Effect::Shortcuts));
        cx.notify();
    }

    pub fn editor(&self) -> Option<&Entity<ShortcutEditor>> {
        self.editor.as_ref()
    }

    pub fn commands(&self) -> &CommandShortcuts {
        &self.commands
    }

    pub fn command_query(&self) -> &str {
        self.selection("command.query").unwrap_or_default()
    }

    fn field_changed(&mut self, id: &str, text: String, cx: &mut Context<Self>) {
        if id == "command.search" {
            self.selections.insert("command.query".to_owned(), text);
        }
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.close_picker(cx);
        cx.notify();
    }

    pub fn request_quick_terminal_input_monitoring(&mut self, cx: &mut Context<Self>) {
        cx.update_global::<QuickTerminalRuntime, _>(|runtime, cx| {
            runtime.request_input_monitoring(cx)
        });
        cx.notify();
    }

    pub fn is_recording_quick_terminal_shortcut(&self) -> bool {
        self.quick_terminal_recording.is_some()
    }

    pub fn toggle_quick_terminal_recording(&mut self, cx: &mut Context<Self>) {
        self.quick_terminal_recording_generation =
            self.quick_terminal_recording_generation.wrapping_add(1);
        if self.quick_terminal_recording.take().is_some() {
            self.errors.remove("quick-terminal-shortcut");
            cx.notify();
            return;
        }
        let generation = self.quick_terminal_recording_generation;
        let (recording, events) = match crate::quick_terminal::start_shortcut_recording() {
            Ok(recording) => recording,
            Err(error) => {
                self.errors
                    .insert("quick-terminal-shortcut".to_owned(), error);
                cx.notify();
                return;
            }
        };
        self.quick_terminal_recording = Some(recording);
        self.errors.insert(
            "quick-terminal-shortcut".to_owned(),
            "Press Command, Control, or Option with a key. Press Escape to cancel.".to_owned(),
        );
        cx.spawn(async move |modal, cx| {
            while let Ok(event) = events.recv().await {
                let stop = modal
                    .update(
                        cx,
                        |modal, cx| match crate::quick_terminal::shortcut_recording_action(
                            modal.quick_terminal_recording_generation,
                            generation,
                            event,
                        ) {
                            crate::quick_terminal::ShortcutRecordingAction::Ignore => true,
                            crate::quick_terminal::ShortcutRecordingAction::Capture(capture) => {
                                modal.set_quick_terminal_shortcut(
                                    muxy_core::quick_terminal::QuickTerminalShortcut::KeyCombo {
                                        key_combo: capture.combo,
                                        virtual_key_code: capture.virtual_key_code,
                                    },
                                    cx,
                                );
                                if modal.error("quick-terminal-shortcut").is_none() {
                                    modal.quick_terminal_recording = None;
                                    true
                                } else {
                                    false
                                }
                            }
                            crate::quick_terminal::ShortcutRecordingAction::Cancel => {
                                modal.quick_terminal_recording = None;
                                modal.errors.remove("quick-terminal-shortcut");
                                cx.notify();
                                true
                            }
                            crate::quick_terminal::ShortcutRecordingAction::Reject(error) => {
                                modal
                                    .errors
                                    .insert("quick-terminal-shortcut".to_owned(), error);
                                cx.notify();
                                false
                            }
                        },
                    )
                    .unwrap_or(true);
                if stop {
                    return;
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub fn set_quick_terminal_shortcut(
        &mut self,
        shortcut: muxy_core::quick_terminal::QuickTerminalShortcut,
        cx: &mut Context<Self>,
    ) {
        let result = cx.update_global::<QuickTerminalRuntime, _>(|runtime, cx| {
            runtime.apply_shortcut_setting(shortcut, cx)
        });
        match result {
            Ok(()) => {
                self.errors.remove("quick-terminal-shortcut");
            }
            Err(error) => {
                self.errors
                    .insert("quick-terminal-shortcut".to_owned(), error);
            }
        }
        cx.notify();
    }

    fn validate_quick_terminal_reverse_conflict(
        &self,
        combo: &KeyCombo,
        cx: &Context<Self>,
    ) -> Result<(), String> {
        cx.global::<QuickTerminalRuntime>()
            .validate_reverse_conflict(combo)
    }

    fn validate_shortcut_map(
        &self,
        shortcuts: &ShortcutMap,
        cx: &Context<Self>,
    ) -> Result<(), String> {
        for action in muxy_core::shortcuts::modelled_actions() {
            self.validate_quick_terminal_reverse_conflict(shortcuts.combo(action), cx)?;
        }
        Ok(())
    }

    pub fn error(&self, key: &str) -> Option<&str> {
        self.errors.get(key).map(String::as_str)
    }

    pub fn set_error(&mut self, key: &str, message: Option<&str>, cx: &mut Context<Self>) {
        match message {
            Some(message) => {
                self.errors.insert(key.to_owned(), message.to_owned());
            }
            None => {
                self.errors.remove(key);
            }
        }
        cx.notify();
    }

    pub fn reset_field(&mut self, id: &str, text: &str, cx: &mut Context<Self>) {
        let Some(field) = self.fields.get(id) else {
            return;
        };
        let text = text.to_owned();
        field.update(cx, |input, cx| input.set_text(text, cx));
    }

    pub fn json_error(&self) -> Option<&str> {
        self.json_error.as_deref()
    }

    pub fn json_status(&self) -> Option<&str> {
        self.json_status.as_deref()
    }

    fn set_json_text(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(field) = self.fields.get(json_editor::EDITOR_FIELD) else {
            return;
        };
        field.update(cx, |input, cx| input.set_text(text, cx));
    }

    pub fn reload_json(&mut self, cx: &mut Context<Self>) {
        self.json_error = None;
        self.json_status = None;
        self.set_json_text(settings::load_user_text(), cx);
        cx.notify();
    }

    pub fn prettify_json(&mut self, cx: &mut Context<Self>) {
        let text = self.field_text(json_editor::EDITOR_FIELD, cx);
        match settings::prettify(&text) {
            Some(pretty) => {
                self.json_error = None;
                self.json_status = Some("JSON prettified".to_owned());
                self.set_json_text(pretty, cx);
            }
            None => {
                self.json_status = None;
                self.json_error = Some(settings::SettingsError::TopLevelObjectRequired.to_string());
            }
        }
        cx.notify();
    }

    pub fn reset_json(&mut self, cx: &mut Context<Self>) {
        settings::reset_user_file();
        self.json_error = None;
        self.json_status = None;
        self.set_json_text(settings::load_user_text(), cx);
        cx.notify();
    }

    pub fn apply_json(&mut self, cx: &mut Context<Self>) {
        let text = self.field_text(json_editor::EDITOR_FIELD, cx);
        let result = cx.update_global::<QuickTerminalRuntime, _>(|runtime, cx| {
            runtime.apply_json_settings(&text, cx)
        });
        match result {
            Ok(()) => {
                self.json_error = None;
                self.json_status = Some("Settings applied".to_owned());
                self.shortcuts = ShortcutMap::load();
                self.commands = CommandShortcuts::load();
                let bindings = self.binding_list();
                if let Some(editor) = self.editor.clone() {
                    editor.update(cx, |editor, cx| editor.apply(bindings, cx));
                }
                cx.emit(SettingsEvent::Applied(Effect::All));
            }
            Err(error) => {
                self.json_status = None;
                self.json_error = Some(error);
            }
        }
        cx.notify();
    }

    pub fn update_command(
        &mut self,
        id: &str,
        name: Option<String>,
        command: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.commands.update(id, name, command);
        self.persist_commands(cx);
    }

    pub fn armed_command(&self) -> Option<&str> {
        self.armed_command.as_deref()
    }

    pub fn command_conflict(&self) -> Option<&str> {
        self.command_conflict.as_deref()
    }

    pub fn delete_all_countdown(&self) -> Option<u8> {
        self.delete_all_countdown
    }

    pub fn arm_command(&mut self, target: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.armed_command = Some(target.to_owned());
        self.command_conflict = None;
        self.close_picker(cx);
        window.focus(&self.focus_handle);
        cx.notify();
    }

    pub fn persist_commands(&mut self, cx: &mut Context<Self>) -> bool {
        if let Err(error) = self.commands.save() {
            log::warn!("failed to write command-shortcuts.json: {error}");
            self.commands = CommandShortcuts::load();
            self.command_conflict = Some(format!("Failed to save command shortcuts: {error}"));
            cx.notify();
            return false;
        }
        cx.emit(SettingsEvent::Applied(Effect::CommandShortcuts));
        cx.notify();
        true
    }

    pub fn add_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let forbidden = cx
            .global::<QuickTerminalRuntime>()
            .settings_state()
            .shortcut
            .key_combo()
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
        let id = self.commands.add_avoiding(&forbidden);
        self.added_command = Some(id.clone());
        if self.persist_commands(cx) {
            self.arm_command(&id, window, cx);
        } else {
            self.added_command = None;
        }
    }

    pub fn remove_command(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.armed_command.as_deref() == Some(id) {
            self.armed_command = None;
        }
        self.fields.remove(&format!("command.name.{id}"));
        self.fields.remove(&format!("command.line.{id}"));
        self.commands.remove(id);
        self.persist_commands(cx);
    }

    pub fn remove_all_commands(&mut self, cx: &mut Context<Self>) {
        self.armed_command = None;
        self.delete_all_countdown = None;
        self.fields.retain(|id, _| !id.starts_with("command."));
        self.commands.remove_all();
        self.persist_commands(cx);
    }

    pub fn start_delete_all(&mut self, cx: &mut Context<Self>) {
        self.delete_all_countdown = Some(5);
        cx.notify();
        cx.spawn(async move |modal, cx| {
            for _ in 0..5 {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let stop = modal
                    .update(cx, |modal, cx| {
                        let Some(remaining) = modal.delete_all_countdown else {
                            return true;
                        };
                        modal.delete_all_countdown = remaining.checked_sub(1).filter(|it| *it > 0);
                        cx.notify();
                        modal.delete_all_countdown.is_none()
                    })
                    .unwrap_or(true);
                if stop {
                    return;
                }
            }
        })
        .detach();
    }

    fn record_command_key(&mut self, target: String, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            self.armed_command = None;
            self.command_conflict = None;
            if self.added_command.as_deref() == Some(target.as_str()) {
                self.added_command = None;
                self.remove_command(&target, cx);
            }
            cx.notify();
            return;
        }
        if matches!(key, "shift" | "control" | "alt" | "platform" | "function") {
            return;
        }
        let requires_modifier = target == "prefix";
        let Some(combo) = crate::views::shortcut_editor::capture(event, requires_modifier) else {
            return;
        };
        if let Err(error) = self.validate_quick_terminal_reverse_conflict(&combo, cx) {
            self.command_conflict = Some(format!(
                "{error} — press a different shortcut or Esc to cancel"
            ));
            cx.notify();
            return;
        }
        if target == "prefix" {
            self.armed_command = None;
            self.added_command = None;
            self.command_conflict = None;
            self.commands.set_prefix_combo(combo);
            self.persist_commands(cx);
            return;
        }
        if let Some(other) = self.commands.conflicting(&combo, &target) {
            self.command_conflict = Some(format!(
                "Conflicts with \"{}\" — press a different shortcut or Esc to cancel",
                other.display_name()
            ));
            cx.notify();
            return;
        }
        self.armed_command = None;
        self.added_command = None;
        self.command_conflict = None;
        self.commands.set_combo(&target, combo);
        self.persist_commands(cx);
    }

    pub fn slider_value(&self, spec: SliderSpec, stored: f32) -> f32 {
        match &self.drag {
            Some(drag) if drag.spec.key == spec.key => drag.value,
            _ => {
                if spec.zero_at_maximum && (stored <= 0.0 || stored >= spec.max) {
                    spec.max
                } else {
                    stored.clamp(spec.min, spec.max)
                }
            }
        }
    }

    pub fn begin_drag(&mut self, spec: SliderSpec, grab: &controls::Grab, cx: &mut Context<Self>) {
        let value = value_at(spec, grab.bounds, grab.position);
        self.drag = Some(SliderDrag {
            spec,
            bounds: grab.bounds,
            value,
        });
        self.close_picker(cx);
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.drag else {
            return;
        };
        drag.value = value_at(drag.spec, drag.bounds, event.position);
        cx.notify();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let stored = drag.spec.stored(drag.value);
        self.write(drag.spec.key, number(drag.spec, stored), cx);
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn picker(&self, key: &str) -> Option<&Entity<CommandPopover>> {
        self.picker
            .as_ref()
            .filter(|(open, _)| open == key)
            .map(|(_, picker)| picker)
    }

    pub fn theme_browser(&self, key: &str) -> Option<&Entity<ThemeBrowser>> {
        self.browser
            .as_ref()
            .filter(|(open, _)| open == key)
            .map(|(_, browser)| browser)
    }

    pub fn toggle_theme_browser(&mut self, key: &str, cx: &mut Context<Self>) {
        self.close_picker(cx);
        if self.theme_browser(key).is_some() {
            self.browser = None;
            cx.notify();
            return;
        }
        let mode = if key == "muxy.theme.dark" {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        let theme = self.theme.clone();
        let metrics = self.metrics;
        let appearance = self.appearance;
        let browser = cx.new(|cx| ThemeBrowser::new(mode, appearance, theme, metrics, cx));
        let subscription = cx.subscribe(&browser, |modal: &mut Self, _, event, cx| match event {
            ThemeBrowserEvent::Applied => {
                cx.emit(SettingsEvent::Applied(Effect::Theme));
                cx.notify();
            }
            ThemeBrowserEvent::Dismiss => {
                modal.browser = None;
                cx.notify();
            }
        });
        self._subscriptions.push(subscription);
        self.browser = Some((key.to_owned(), browser));
        cx.notify();
    }

    pub fn close_picker(&mut self, cx: &mut Context<Self>) {
        if self.picker.take().is_some() {
            self.picker_subscription = None;
            cx.notify();
        }
    }

    pub fn toggle_picker(
        &mut self,
        key: &str,
        choices: Vec<controls::Choice>,
        selected: String,
        target: SettingsPickerTarget,
        cx: &mut Context<Self>,
    ) {
        if self.picker(key).is_some() {
            self.close_picker(cx);
            return;
        }
        self.close_picker(cx);
        self.browser = None;
        let theme = self.theme.clone();
        let metrics = self.metrics;
        let picker = cx.new(|cx| {
            CommandPopover::new(
                CommandPopoverConfig {
                    id: format!("settings-popover-{key}").into(),
                    presentation: CommandPopoverPresentation::Popover,
                    density: CommandPopoverDensity::Compact,
                    tabs: vec![CommandPopoverTab::new("options", "Options")],
                    placeholder: "Search options…".into(),
                    footer_actions: Vec::new(),
                    footer_hints: Vec::new(),
                    width: Some(controls::CONTROL_WIDTH),
                    height: None,
                    max_height: Some(280.0),
                    completion_on_tab: false,
                    confirm_on_click: true,
                },
                theme,
                metrics,
                cx,
            )
        });
        sync_settings_picker(&picker, &choices, &selected, "", cx);
        let event_choices = choices.clone();
        let event_selected = selected.clone();
        let event_key = key.to_owned();
        let subscription =
            cx.subscribe(
                &picker,
                move |modal: &mut Self, picker, event, cx| match event {
                    CommandPopoverEvent::QueryChanged { query, .. } => {
                        sync_settings_picker(&picker, &event_choices, &event_selected, query, cx);
                    }
                    CommandPopoverEvent::Confirmed(selection)
                    | CommandPopoverEvent::SecondaryConfirmed(selection) => {
                        let Some(index) = selection
                            .id
                            .strip_prefix("settings-choice-")
                            .and_then(|index| index.parse::<usize>().ok())
                        else {
                            return;
                        };
                        let Some(choice) = event_choices.get(index) else {
                            return;
                        };
                        if !choice.enabled {
                            return;
                        }
                        match &target {
                            SettingsPickerTarget::Setting => {
                                modal.write(&event_key, Value::String(choice.value.clone()), cx)
                            }
                            SettingsPickerTarget::NotificationSound => {
                                modal.write_notification_sound(&choice.value, cx);
                            }
                            SettingsPickerTarget::Editor(name) => {
                                settings::set_editor_setting(
                                    name,
                                    Value::String(choice.value.clone()),
                                );
                                modal.close_picker(cx);
                                modal.refresh(cx);
                            }
                            SettingsPickerTarget::Number(values) => {
                                let Some(value) = values
                                    .iter()
                                    .find(|(label, _)| label == &choice.value)
                                    .map(|(_, value)| *value)
                                else {
                                    return;
                                };
                                modal.write(
                                    &event_key,
                                    serde_json::Number::from_f64(value)
                                        .map_or(Value::Null, Value::Number),
                                    cx,
                                );
                            }
                        }
                    }
                    CommandPopoverEvent::Dismissed => modal.close_picker(cx),
                    _ => {}
                },
            );
        self.picker = Some((key.to_owned(), picker));
        self.picker_subscription = Some(subscription);
        cx.notify();
    }

    pub fn write_ai_notification_provider(
        &mut self,
        provider: &str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let key = settings::provider_key(provider);
        self.write(&key, Value::Bool(enabled), cx);
    }

    pub fn request_desktop_notifications(&mut self, cx: &mut Context<Self>) {
        let enabled = settings::bool_value("muxy.notifications.desktopEnabled", false);
        let event = desktop_notification_event(enabled, self.desktop_authorization_pending);
        self.desktop_authorization_pending =
            matches!(event, SettingsEvent::SetDesktopNotifications(true));
        cx.emit(event);
        cx.notify();
    }

    pub fn set_desktop_authorization_pending(&mut self, pending: bool, cx: &mut Context<Self>) {
        self.desktop_authorization_pending = pending;
        self.refresh(cx);
    }

    pub fn desktop_authorization_pending(&self) -> bool {
        self.desktop_authorization_pending
    }

    pub fn write_notification_sound(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let Some(event) = notification_sound_event(name) else {
            return false;
        };
        self.close_picker(cx);
        Prefs::store_settings_value("muxy.notifications.sound", Value::String(name.to_owned()));
        cx.emit(event);
        cx.notify();
        true
    }

    pub fn write(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        self.close_picker(cx);
        if key.starts_with("muxy.quickTerminal.") {
            let result = if key == "muxy.quickTerminal.enabled" {
                cx.update_global::<QuickTerminalRuntime, _>(|runtime, cx| {
                    runtime.apply_enabled_setting(value, cx)
                })
            } else {
                settings::try_set(key, value)
                    .map_err(|error| format!("failed to persist {key}: {error}"))
            };
            match result {
                Ok(()) => {
                    self.errors.remove(key);
                }
                Err(error) => {
                    self.errors.insert(key.to_owned(), error);
                }
            }
            cx.notify();
            return;
        }
        Prefs::store_settings_value(key, value);
        if CHROME_KEYS.contains(&key) {
            cx.emit(SettingsEvent::Applied(Effect::Chrome));
        }
        cx.notify();
    }

    pub fn set_scale(&mut self, preset: muxy_core::prefs::ScalePreset, cx: &mut Context<Self>) {
        self.close_picker(cx);
        settings::set_ui_scale(preset);
        cx.emit(SettingsEvent::Applied(Effect::Scale));
        cx.notify();
    }

    fn visible_categories(&self) -> Vec<Category> {
        Category::ALL
            .into_iter()
            .filter(|category| category_supported(*category))
            .filter(|category| settings_catalog::category_matches(*category, &self.query))
            .collect()
    }

    pub(crate) fn select_category(&mut self, category: Category, cx: &mut Context<Self>) {
        let category = normalized_category(category);
        self.route = category;
        self.close_picker(cx);
        Prefs::store_default(ROUTE_KEY, Some(&category.route()));
        cx.notify();
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Dismiss);
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.armed_command.clone() {
            cx.stop_propagation();
            self.record_command_key(target, event, cx);
            return;
        }
        if event.keystroke.key != "escape" {
            return;
        }
        if let Some(editor) = self.editor.clone()
            && self.route == Category::Shortcuts
            && editor.update(cx, |editor, cx| editor.handle_escape(cx))
        {
            cx.stop_propagation();
            return;
        }
        if self.browser.take().is_some() || self.picker.take().is_some() {
            self.picker_subscription = None;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        cx.emit(SettingsEvent::Dismiss);
    }

    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let metrics = &self.metrics;
        let theme = &self.theme;
        let has_query = !self.query.trim().is_empty();

        let mut field = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.spacing3())
            .flex_grow()
            .min_w(px(0.0))
            .ml(metrics.spacing4())
            .mr(metrics.spacing5())
            .px(metrics.scaled(9.0))
            .py(metrics.scaled(7.0))
            .rounded(metrics.radius_lg())
            .bg(theme.surface)
            .border_1()
            .border_color(if has_query {
                theme.accent
            } else {
                theme.surface
            })
            .child(SymbolGlyph::new(
                "magnifyingglass",
                metrics.font_body(),
                theme.fg_muted,
            ))
            .child(muxy_ui::text_input::growing_input(&self.search));

        if has_query {
            field = field.child(
                div()
                    .id("settings-search-clear")
                    .flex()
                    .flex_none()
                    .cursor_pointer()
                    .child(SymbolGlyph::new(
                        "xmark.circle.fill",
                        metrics.font_body(),
                        theme.fg_muted,
                    ))
                    .on_click(cx.listener(|modal, _, _, cx| {
                        modal.search.update(cx, |input, cx| {
                            input.set_text(String::new(), cx);
                        });
                    })),
            );
        }

        div()
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .h(metrics.scaled(HEADER_HEIGHT))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(metrics.spacing6())
                    .flex_none()
                    .w(metrics.scaled(SIDEBAR_WIDTH))
                    .px(metrics.spacing7())
                    .child(SymbolGlyph::new(
                        "slider.horizontal.3",
                        metrics.font_emphasis(),
                        theme.fg_muted,
                    ))
                    .child(
                        div()
                            .text_size(metrics.font_title())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.fg)
                            .child(SharedString::from("Settings")),
                    ),
            )
            .child(
                div()
                    .w(px(1.0))
                    .h(metrics.scaled(HEADER_HEIGHT))
                    .flex_none()
                    .bg(theme.border),
            )
            .child(field)
            .child(
                div()
                    .id("settings-close")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(metrics.scaled(28.0))
                    .mr(metrics.spacing6())
                    .rounded(metrics.radius_sm())
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.hover))
                    .child(SymbolGlyph::new(
                        "xmark",
                        metrics.font_body(),
                        theme.fg_muted,
                    ))
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(SettingsEvent::Dismiss);
                    })),
            )
            .into_any_element()
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let metrics = &self.metrics;
        let theme = &self.theme;
        let categories = self.visible_categories();

        let mut list = div().flex().flex_col().gap(metrics.spacing2());
        if categories.is_empty() {
            list = list.child(
                div()
                    .p(metrics.spacing6())
                    .text_size(metrics.font_body())
                    .text_color(theme.fg_muted)
                    .child(SharedString::from("No settings found")),
            );
        }
        for category in categories {
            list = list.child(self.sidebar_row(category, cx));
        }

        div()
            .id("settings-sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .w(metrics.scaled(SIDEBAR_WIDTH))
            .p(metrics.spacing5())
            .overflow_y_scroll()
            .track_scroll(&self.sidebar_scroll)
            .bg(sidebar_background(theme))
            .child(list)
            .into_any_element()
    }

    fn sidebar_row(&self, category: Category, cx: &mut Context<Self>) -> AnyElement {
        let metrics = &self.metrics;
        let theme = &self.theme;
        let selected = self.route == category;
        let count = settings_catalog::match_count_summary(category, &self.query);

        let mut labels = div().flex().flex_col().gap(metrics.spacing1()).child(
            div()
                .text_size(metrics.font_body())
                .font_weight(if selected {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(theme.fg)
                .truncate()
                .child(SharedString::from(category.title())),
        );
        if let Some(count) = count {
            labels = labels.child(
                div()
                    .text_size(metrics.font_caption())
                    .text_color(theme.fg_muted)
                    .child(SharedString::from(count)),
            );
        }

        div()
            .id(SharedString::from(format!(
                "settings-route-{}",
                category.raw()
            )))
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.spacing4())
            .px(metrics.spacing5())
            .py(metrics.scaled(7.0))
            .rounded(metrics.radius_lg())
            .cursor_pointer()
            .when(selected, |element| element.bg(theme.accent_soft))
            .when(!selected, |element| {
                element.hover(|style| style.bg(theme.hover))
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .w(metrics.scaled(16.0))
                    .child(SymbolGlyph::new(
                        category.symbol(),
                        metrics.font_body(),
                        if selected {
                            theme.accent
                        } else {
                            theme.fg_muted
                        },
                    )),
            )
            .child(labels)
            .on_click(cx.listener(move |modal, _, _, cx| modal.select_category(category, cx)))
            .into_any_element()
    }

    fn content(&self, cx: &mut Context<Self>) -> AnyElement {
        let metrics = &self.metrics;
        let theme = &self.theme;
        if self.visible_categories().is_empty() {
            return div()
                .flex()
                .flex_grow()
                .items_center()
                .justify_center()
                .text_size(metrics.font_body())
                .text_color(theme.fg_muted)
                .child(SharedString::from("No settings found"))
                .into_any_element();
        }

        div()
            .id("settings-content")
            .flex()
            .flex_col()
            .flex_grow()
            .min_w(px(0.0))
            .overflow_y_scroll()
            .track_scroll(&self.content_scroll)
            .children(categories::content(self, self.route, cx))
            .into_any_element()
    }
}

fn sync_settings_picker(
    picker: &Entity<CommandPopover>,
    choices: &[controls::Choice],
    selected: &str,
    query: &str,
    cx: &mut Context<SettingsModal>,
) {
    let query = query.trim().to_lowercase();
    let items = choices
        .iter()
        .enumerate()
        .filter(|(_, choice)| {
            query.is_empty()
                || choice.label.to_lowercase().contains(&query)
                || choice.value.to_lowercase().contains(&query)
        })
        .map(|(index, choice)| {
            let mut row =
                CommandPopoverRow::new(format!("settings-choice-{index}"), choice.label.clone());
            row.current = choice.value == selected;
            row.disabled = !choice.enabled;
            CommandPopoverItem::Row(row)
        })
        .collect::<Vec<_>>();
    let status = if items.is_empty() {
        CommandPopoverStatus::Empty("No matching options".into())
    } else {
        CommandPopoverStatus::Ready
    };
    let selected_id = choices
        .iter()
        .position(|choice| choice.value == selected)
        .map(|index| format!("settings-choice-{index}"));
    picker.update(cx, |picker, cx| {
        picker.set_items(items, cx);
        picker.set_status(status, cx);
        if let Some(selected_id) = selected_id {
            let _ = picker.select_row(&selected_id, cx);
        }
    });
}

fn value_at(spec: SliderSpec, bounds: Bounds<Pixels>, position: gpui::Point<Pixels>) -> f32 {
    let fraction = controls::fraction_at(bounds, position);
    (spec.min + (spec.max - spec.min) * fraction).round()
}

fn number(spec: SliderSpec, value: f32) -> Value {
    if spec.integral {
        return Value::Number(serde_json::Number::from(value.round() as i64));
    }
    serde_json::Number::from_f64(f64::from(value)).map_or(Value::Null, Value::Number)
}

fn sidebar_background(theme: &Theme) -> Hsla {
    Hsla {
        l: theme.bg.l * 0.92,
        ..theme.bg
    }
}

impl Render for SettingsModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = self.metrics;
        let theme = self.theme.clone();
        if !self.focused {
            self.focused = true;
            window.focus(&self.search.focus_handle(cx));
        }
        self.prepare_fields(window, cx);
        self.prepare_editor(cx);

        let viewport = window.viewport_size();
        let margin = metrics.spacing8();
        let top_inset = metrics.title_bar_height();
        let floor = metrics.scaled(MINIMUM_PANEL_SIDE);
        let width = metrics
            .scaled(PANEL_WIDTH)
            .min((viewport.width - margin * 2.0).max(floor));
        let height = metrics
            .scaled(PANEL_HEIGHT)
            .min((viewport.height - top_inset - margin).max(floor));

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .pt(top_inset)
            .pb(margin)
            .px(margin)
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .key_context(KEY_CONTEXT)
                    .track_focus(&self.focus_handle)
                    .on_action(cx.listener(Self::dismiss))
                    .on_key_down(cx.listener(Self::on_key_down))
                    .on_mouse_move(cx.listener(Self::on_mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|modal: &mut Self, _, _, cx| {
                            if modal.browser.take().is_some() || modal.picker.take().is_some() {
                                modal.picker_subscription = None;
                                cx.notify();
                            }
                            cx.stop_propagation();
                        }),
                    )
                    .occlude()
                    .flex()
                    .flex_col()
                    .w(width)
                    .h(height)
                    .rounded(metrics.radius_xl())
                    .bg(theme.bg)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .overflow_hidden()
                    .child(self.header(cx))
                    .child(div().h(px(1.0)).flex_none().bg(theme.border))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_grow()
                            .min_h(px(0.0))
                            .child(self.sidebar(cx))
                            .child(div().w(px(1.0)).flex_none().bg(theme.border))
                            .child(self.content(cx)),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_desktop_toggle_emits_exact_events_and_pending_click_disables() {
        assert_eq!(
            desktop_notification_event(false, false),
            SettingsEvent::SetDesktopNotifications(true)
        );
        assert_eq!(
            desktop_notification_event(true, false),
            SettingsEvent::SetDesktopNotifications(false)
        );
        assert_eq!(
            desktop_notification_event(false, true),
            SettingsEvent::SetDesktopNotifications(false)
        );
    }

    #[test]
    fn settings_sound_picker_accepts_only_catalog_names_and_emits_exact_preview() {
        for name in settings::NOTIFICATION_SOUNDS {
            assert_eq!(
                notification_sound_event(name),
                Some(SettingsEvent::PreviewNotificationSound(name.to_owned()))
            );
        }
        assert_eq!(notification_sound_event("unknown"), None);
        assert_eq!(notification_sound_event(""), None);
    }

    #[test]
    fn quick_terminal_settings_category_matches_platform_support() {
        assert_eq!(
            category_supported(Category::QuickTerminal),
            cfg!(target_os = "macos")
        );
        assert_eq!(
            normalized_category(Category::QuickTerminal),
            if cfg!(target_os = "macos") {
                Category::QuickTerminal
            } else {
                Category::General
            }
        );
        assert!(category_supported(Category::General));
    }

    #[test]
    fn settings_sound_preview_path_has_no_history_or_toast_effect() {
        let source = include_str!("../window/overlays.rs");
        let start = source.find("fn preview_notification_sound").unwrap();
        let end = source[start..]
            .find("pub(crate) fn open_theme_picker")
            .unwrap()
            + start;
        let preview = &source[start..end];
        assert!(preview.contains("notification_coordinator.play_sound(name)"));
        assert!(!preview.contains("feedback("));
        assert!(!preview.contains("show_toast"));
        assert!(!preview.contains("notification_store"));
    }
}
