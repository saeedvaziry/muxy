use muxy_core::extensions::manifest::{
    ExtensionHomeView, ExtensionPanel, ExtensionPopover, ExtensionSidebar, ExtensionTabType,
};
use muxy_core::extensions::runtime::ExtensionRuntimeCatalog;
use muxy_core::workspace::TabKind;
use muxy_core::workspace_store::WorkspaceStore;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ExtensionSurfaceKind {
    Tab(ExtensionTabType),
    Panel(ExtensionPanel),
    Popover(ExtensionPopover),
    Modal {
        entry: String,
        width: f64,
        height: f64,
        dismiss_on_outside_click: bool,
        opener_surface_id: Option<String>,
    },
    Sidebar(ExtensionSidebar),
    Home(ExtensionHomeView),
}

impl ExtensionSurfaceKind {
    pub(crate) fn lifecycle_reason(&self) -> &'static str {
        match self {
            Self::Tab(_) => "tab",
            Self::Panel(_) => "panel",
            Self::Popover(_) => "popover",
            Self::Modal { .. } => "modal",
            Self::Sidebar(_) => "sidebar",
            Self::Home(_) => "homeView",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExtensionSurface {
    pub extension_id: String,
    pub instance_id: String,
    pub project_id: Option<String>,
    pub entry_url: String,
    pub resource_root: PathBuf,
    pub data: Value,
    pub kind: ExtensionSurfaceKind,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub(crate) enum ExtensionSurfaceError {
    #[error("extension '{0}' is not loaded")]
    ExtensionUnavailable(String),
    #[error("unknown tab type '{1}' for extension '{0}'")]
    UnknownTabType(String, String),
    #[error("unknown panel '{1}' for extension '{0}'")]
    UnknownPanel(String, String),
    #[error("unknown popover '{1}' for extension '{0}'")]
    UnknownPopover(String, String),
    #[error("extension asset '{1}' is invalid for extension '{0}'")]
    InvalidAsset(String, String),
    #[error("a project must be active to open an extension panel")]
    MissingProject,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ExtensionSurfaceChanges {
    pub opened: Vec<ExtensionSurface>,
    pub closed: Vec<ExtensionSurface>,
}

impl ExtensionSurfaceChanges {
    fn opened(surface: ExtensionSurface) -> Self {
        Self {
            opened: vec![surface],
            closed: Vec::new(),
        }
    }

    fn closed(surface: ExtensionSurface) -> Self {
        Self {
            opened: Vec::new(),
            closed: vec![surface],
        }
    }

    fn push_closed(&mut self, surface: ExtensionSurface) {
        self.closed.push(surface);
    }
}

#[derive(Debug, Default)]
pub(crate) struct ExtensionSurfaceRegistry {
    surfaces: BTreeMap<String, ExtensionSurface>,
    active_project_id: Option<String>,
    focused_surface_id: Option<String>,
    panel_dimensions: BTreeMap<String, f64>,
    next_instance: u64,
}

impl ExtensionSurfaceRegistry {
    pub(crate) fn sync_tabs(&mut self, store: &WorkspaceStore, catalog: &ExtensionRuntimeCatalog) {
        let mut present = BTreeSet::new();
        for workspace in store.states() {
            for tab in workspace.root.iter().flat_map(|root| root.tabs()) {
                if tab.kind != TabKind::ExtensionWebView {
                    continue;
                }
                let Some(extension_id) = tab.extension_id.as_deref() else {
                    continue;
                };
                let Some(tab_type_id) = tab.extension_web_view_id.as_deref() else {
                    continue;
                };
                let Some(record) = enabled_record(catalog, extension_id) else {
                    continue;
                };
                let Some(tab_type) = record.extension.manifest.tab_type(tab_type_id).cloned()
                else {
                    continue;
                };
                let Ok(surface) = declared_surface(
                    extension_id,
                    tab.id.clone(),
                    Some(workspace.project_id.clone()),
                    &record.extension.resource_root,
                    tab_type.entry.clone().as_str(),
                    tab.extension_data
                        .clone()
                        .or_else(|| tab_type.default_data.clone())
                        .unwrap_or(Value::Null),
                    ExtensionSurfaceKind::Tab(tab_type),
                ) else {
                    continue;
                };
                present.insert(tab.id.clone());
                self.surfaces.insert(tab.id.clone(), surface);
            }
        }
        self.surfaces.retain(|instance_id, surface| {
            !matches!(surface.kind, ExtensionSurfaceKind::Tab(_)) || present.contains(instance_id)
        });
    }

    pub(crate) fn set_active_project(
        &mut self,
        project_id: Option<&str>,
    ) -> ExtensionSurfaceChanges {
        if self.active_project_id.as_deref() == project_id {
            return ExtensionSurfaceChanges::default();
        }
        let previous_project_id = self.active_project_id.clone();
        self.active_project_id = project_id.map(str::to_owned);
        let mut changes = ExtensionSurfaceChanges::default();
        if let Some(previous_project_id) = previous_project_id.as_deref() {
            changes.closed.extend(
                self.surfaces
                    .values()
                    .filter(|surface| {
                        matches!(surface.kind, ExtensionSurfaceKind::Panel(_))
                            && surface.project_id.as_deref() == Some(previous_project_id)
                    })
                    .cloned(),
            );
        }
        if let Some(project_id) = project_id {
            changes.opened.extend(
                self.surfaces
                    .values()
                    .filter(|surface| {
                        matches!(surface.kind, ExtensionSurfaceKind::Panel(_))
                            && surface.project_id.as_deref() == Some(project_id)
                    })
                    .cloned(),
            );
        }
        let removed = self
            .surfaces
            .iter()
            .filter(|(_, surface)| {
                matches!(
                    surface.kind,
                    ExtensionSurfaceKind::Popover(_) | ExtensionSurfaceKind::Modal { .. }
                )
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in removed {
            if let Some(surface) = self.surfaces.remove(&id) {
                changes.push_closed(surface);
            }
        }
        if self.focused_surface_id.as_deref().is_some_and(|id| {
            self.surfaces.get(id).is_none_or(|surface| {
                matches!(surface.kind, ExtensionSurfaceKind::Panel(_))
                    && surface.project_id.as_deref() != self.active_project_id.as_deref()
            })
        }) {
            self.focused_surface_id = None;
        }
        changes
    }

    pub(crate) fn active_project_id(&self) -> Option<&str> {
        self.active_project_id.as_deref()
    }

    pub(crate) fn open_panel(
        &mut self,
        catalog: &ExtensionRuntimeCatalog,
        extension_id: &str,
        panel_id: &str,
        data: Option<Value>,
    ) -> Result<ExtensionSurfaceChanges, ExtensionSurfaceError> {
        let project_id = self
            .active_project_id
            .clone()
            .ok_or(ExtensionSurfaceError::MissingProject)?;
        let record = enabled_record(catalog, extension_id)
            .ok_or_else(|| ExtensionSurfaceError::ExtensionUnavailable(extension_id.to_owned()))?;
        let panel = record
            .extension
            .manifest
            .panel(panel_id)
            .cloned()
            .ok_or_else(|| {
                ExtensionSurfaceError::UnknownPanel(extension_id.to_owned(), panel_id.to_owned())
            })?;
        let instance_id = panel_instance_id(extension_id, panel_id, &project_id);
        if let Some(surface) = self.surfaces.get_mut(&instance_id) {
            if let Some(data) = data {
                surface.data = data;
            }
            return Ok(ExtensionSurfaceChanges::default());
        }
        let surface = declared_surface(
            extension_id,
            instance_id.clone(),
            Some(project_id),
            &record.extension.resource_root,
            panel.entry.clone().as_str(),
            data.or_else(|| panel.default_data.clone())
                .unwrap_or(Value::Null),
            ExtensionSurfaceKind::Panel(panel),
        )?;
        let changes = ExtensionSurfaceChanges::opened(surface.clone());
        self.surfaces.insert(instance_id, surface);
        self.focused_surface_id = changes
            .opened
            .first()
            .map(|surface| surface.instance_id.clone());
        Ok(changes)
    }

    pub(crate) fn toggle_panel(
        &mut self,
        catalog: &ExtensionRuntimeCatalog,
        extension_id: &str,
        panel_id: &str,
        data: Option<Value>,
    ) -> Result<ExtensionSurfaceChanges, ExtensionSurfaceError> {
        let project_id = self
            .active_project_id
            .as_deref()
            .ok_or(ExtensionSurfaceError::MissingProject)?;
        let instance_id = panel_instance_id(extension_id, panel_id, project_id);
        if let Some(surface) = self.surfaces.remove(&instance_id) {
            return Ok(ExtensionSurfaceChanges::closed(surface));
        }
        self.open_panel(catalog, extension_id, panel_id, data)
    }

    pub(crate) fn close_panel(
        &mut self,
        extension_id: &str,
        panel_id: &str,
    ) -> ExtensionSurfaceChanges {
        let Some(project_id) = self.active_project_id.as_deref() else {
            return ExtensionSurfaceChanges::default();
        };
        self.close(&panel_instance_id(extension_id, panel_id, project_id))
    }

    pub(crate) fn toggle_popover(
        &mut self,
        catalog: &ExtensionRuntimeCatalog,
        extension_id: &str,
        popover_id: &str,
        data: Option<Value>,
    ) -> Result<ExtensionSurfaceChanges, ExtensionSurfaceError> {
        let instance_id = popover_instance_id(extension_id, popover_id);
        if let Some(surface) = self.surfaces.remove(&instance_id) {
            return Ok(ExtensionSurfaceChanges::closed(surface));
        }
        let record = enabled_record(catalog, extension_id)
            .ok_or_else(|| ExtensionSurfaceError::ExtensionUnavailable(extension_id.to_owned()))?;
        let popover = record
            .extension
            .manifest
            .popover(popover_id)
            .cloned()
            .ok_or_else(|| {
                ExtensionSurfaceError::UnknownPopover(
                    extension_id.to_owned(),
                    popover_id.to_owned(),
                )
            })?;
        let previous = self
            .surfaces
            .iter()
            .find(|(_, surface)| matches!(surface.kind, ExtensionSurfaceKind::Popover(_)))
            .map(|(id, _)| id.clone());
        let surface = declared_surface(
            extension_id,
            instance_id.clone(),
            self.active_project_id.clone(),
            &record.extension.resource_root,
            popover.entry.clone().as_str(),
            data.or_else(|| popover.default_data.clone())
                .unwrap_or(Value::Null),
            ExtensionSurfaceKind::Popover(popover),
        )?;
        let mut changes = ExtensionSurfaceChanges::opened(surface.clone());
        if let Some(previous) = previous
            && let Some(surface) = self.surfaces.remove(&previous)
        {
            changes.push_closed(surface);
        }
        self.surfaces.insert(instance_id, surface);
        self.focused_surface_id = changes
            .opened
            .first()
            .map(|surface| surface.instance_id.clone());
        Ok(changes)
    }

    pub(crate) fn open_modal(
        &mut self,
        catalog: &ExtensionRuntimeCatalog,
        extension_id: &str,
        instance_id: Option<String>,
        entry: &str,
        width: Option<f64>,
        height: Option<f64>,
        dismiss_on_outside_click: bool,
        data: Option<Value>,
        opener_surface_id: Option<String>,
    ) -> Result<ExtensionSurfaceChanges, ExtensionSurfaceError> {
        let record = enabled_record(catalog, extension_id)
            .ok_or_else(|| ExtensionSurfaceError::ExtensionUnavailable(extension_id.to_owned()))?;
        let instance_id = instance_id.unwrap_or_else(|| self.next_id("modal", extension_id));
        let surface = declared_surface(
            extension_id,
            instance_id.clone(),
            self.active_project_id.clone(),
            &record.extension.resource_root,
            entry,
            data.unwrap_or(Value::Null),
            ExtensionSurfaceKind::Modal {
                entry: entry.to_owned(),
                width: finite_clamp(width.unwrap_or(480.0), 120.0, 900.0),
                height: finite_clamp(height.unwrap_or(320.0), 120.0, 760.0),
                dismiss_on_outside_click,
                opener_surface_id,
            },
        )?;
        let previous = self
            .surfaces
            .iter()
            .rev()
            .find(|(_, surface)| matches!(surface.kind, ExtensionSurfaceKind::Modal { .. }))
            .map(|(id, _)| id.clone());
        let mut changes = ExtensionSurfaceChanges::opened(surface.clone());
        if let Some(previous) = previous
            && let Some(surface) = self.surfaces.remove(&previous)
        {
            changes.push_closed(surface);
        }
        self.surfaces.insert(instance_id, surface.clone());
        self.focused_surface_id = Some(surface.instance_id.clone());
        Ok(changes)
    }

    pub(crate) fn open_sidebar(
        &mut self,
        catalog: &ExtensionRuntimeCatalog,
        extension_id: &str,
        data: Option<Value>,
    ) -> Result<ExtensionSurfaceChanges, ExtensionSurfaceError> {
        let record = enabled_record(catalog, extension_id)
            .ok_or_else(|| ExtensionSurfaceError::ExtensionUnavailable(extension_id.to_owned()))?;
        let sidebar = record.extension.manifest.sidebar.clone().ok_or_else(|| {
            ExtensionSurfaceError::InvalidAsset(extension_id.to_owned(), "sidebar".to_owned())
        })?;
        let instance_id = format!("extension-sidebar:{extension_id}:{}", sidebar.id);
        if let Some(surface) = self.surfaces.get_mut(&instance_id) {
            if let Some(data) = data {
                surface.data = data;
            }
            return Ok(ExtensionSurfaceChanges::default());
        }
        let surface = declared_surface(
            extension_id,
            instance_id.clone(),
            None,
            &record.extension.resource_root,
            sidebar.entry.clone().as_str(),
            data.or_else(|| sidebar.default_data.clone())
                .unwrap_or(Value::Null),
            ExtensionSurfaceKind::Sidebar(sidebar),
        )?;
        let previous = self
            .surfaces
            .iter()
            .find(|(_, surface)| matches!(surface.kind, ExtensionSurfaceKind::Sidebar(_)))
            .map(|(id, _)| id.clone());
        let mut changes = ExtensionSurfaceChanges::opened(surface.clone());
        if let Some(previous) = previous
            && let Some(surface) = self.surfaces.remove(&previous)
        {
            changes.push_closed(surface);
        }
        self.surfaces.insert(instance_id, surface);
        self.focused_surface_id = changes
            .opened
            .first()
            .map(|surface| surface.instance_id.clone());
        Ok(changes)
    }

    pub(crate) fn sync_sidebar_selection(
        &mut self,
        catalog: &ExtensionRuntimeCatalog,
        extension_id: Option<&str>,
    ) -> ExtensionSurfaceChanges {
        let current = self
            .surfaces
            .iter()
            .find(|(_, surface)| matches!(surface.kind, ExtensionSurfaceKind::Sidebar(_)))
            .map(|(id, surface)| (id.clone(), surface.extension_id.clone()));
        let selected = extension_id.filter(|extension_id| {
            enabled_record(catalog, extension_id)
                .is_some_and(|record| record.extension.manifest.sidebar.is_some())
        });
        if current
            .as_ref()
            .is_some_and(|(_, current_extension)| Some(current_extension.as_str()) == selected)
        {
            return ExtensionSurfaceChanges::default();
        }
        let mut changes = ExtensionSurfaceChanges::default();
        if let Some((id, _)) = current
            && let Some(surface) = self.surfaces.remove(&id)
        {
            changes.push_closed(surface);
        }
        if let Some(extension_id) = selected
            && let Ok(opened) = self.open_sidebar(catalog, extension_id, None)
        {
            changes.opened.extend(opened.opened);
            changes.closed.extend(opened.closed);
        }
        changes
    }

    pub(crate) fn open_home(
        &mut self,
        catalog: &ExtensionRuntimeCatalog,
        extension_id: &str,
        home_id: &str,
        data: Option<Value>,
    ) -> Result<ExtensionSurfaceChanges, ExtensionSurfaceError> {
        let record = enabled_record(catalog, extension_id)
            .ok_or_else(|| ExtensionSurfaceError::ExtensionUnavailable(extension_id.to_owned()))?;
        let home = record
            .extension
            .manifest
            .home_view(home_id)
            .cloned()
            .ok_or_else(|| {
                ExtensionSurfaceError::InvalidAsset(extension_id.to_owned(), home_id.to_owned())
            })?;
        let instance_id = format!("extension-home:{extension_id}:{home_id}");
        let surface = declared_surface(
            extension_id,
            instance_id.clone(),
            None,
            &record.extension.resource_root,
            home.entry.clone().as_str(),
            data.or_else(|| home.default_data.clone())
                .unwrap_or(Value::Null),
            ExtensionSurfaceKind::Home(home),
        )?;
        let previous = self
            .surfaces
            .iter()
            .find(|(_, surface)| matches!(surface.kind, ExtensionSurfaceKind::Home(_)))
            .map(|(id, _)| id.clone());
        let mut changes = ExtensionSurfaceChanges::opened(surface.clone());
        if let Some(previous) = previous
            && let Some(surface) = self.surfaces.remove(&previous)
        {
            changes.push_closed(surface);
        }
        self.surfaces.insert(instance_id, surface);
        self.focused_surface_id = changes
            .opened
            .first()
            .map(|surface| surface.instance_id.clone());
        Ok(changes)
    }

    pub(crate) fn close(&mut self, instance_id: &str) -> ExtensionSurfaceChanges {
        if self.focused_surface_id.as_deref() == Some(instance_id) {
            self.focused_surface_id = None;
        }
        self.surfaces
            .remove(instance_id)
            .map(ExtensionSurfaceChanges::closed)
            .unwrap_or_default()
    }

    pub(crate) fn resize_popover(
        &mut self,
        instance_id: &str,
        width: f64,
        height: f64,
    ) -> Result<(), ExtensionSurfaceError> {
        let surface = self.surfaces.get_mut(instance_id).ok_or_else(|| {
            ExtensionSurfaceError::InvalidAsset("surface".to_owned(), instance_id.to_owned())
        })?;
        let ExtensionSurfaceKind::Popover(popover) = &mut surface.kind else {
            return Err(ExtensionSurfaceError::InvalidAsset(
                surface.extension_id.clone(),
                instance_id.to_owned(),
            ));
        };
        popover.width = finite_clamp(width, 160.0, 720.0);
        popover.height = finite_clamp(height, 120.0, 760.0);
        Ok(())
    }

    pub(crate) fn panel_dimension(&self, instance_id: &str) -> Option<f64> {
        let surface = self.surfaces.get(instance_id)?;
        let ExtensionSurfaceKind::Panel(panel) = &surface.kind else {
            return None;
        };
        Some(
            self.panel_dimensions
                .get(instance_id)
                .copied()
                .unwrap_or(match panel.position {
                    muxy_core::extensions::manifest::PanelPosition::Right => 360.0,
                    muxy_core::extensions::manifest::PanelPosition::Bottom => 320.0,
                }),
        )
    }

    pub(crate) fn resize_panel(&mut self, instance_id: &str, dimension: f64) -> bool {
        let Some(surface) = self.surfaces.get(instance_id) else {
            return false;
        };
        let ExtensionSurfaceKind::Panel(panel) = &surface.kind else {
            return false;
        };
        let dimension = match panel.position {
            muxy_core::extensions::manifest::PanelPosition::Right => {
                finite_clamp(dimension, 240.0, 720.0)
            }
            muxy_core::extensions::manifest::PanelPosition::Bottom => {
                finite_clamp(dimension, 160.0, 600.0)
            }
        };
        self.panel_dimensions
            .insert(instance_id.to_owned(), dimension);
        true
    }

    pub(crate) fn move_panel(&mut self, instance_id: &str) -> ExtensionSurfaceChanges {
        let Some(surface) = self.surfaces.get(instance_id) else {
            return ExtensionSurfaceChanges::default();
        };
        let ExtensionSurfaceKind::Panel(panel) = &surface.kind else {
            return ExtensionSurfaceChanges::default();
        };
        let next = match panel.position {
            muxy_core::extensions::manifest::PanelPosition::Right => {
                muxy_core::extensions::manifest::PanelPosition::Bottom
            }
            muxy_core::extensions::manifest::PanelPosition::Bottom => {
                muxy_core::extensions::manifest::PanelPosition::Right
            }
        };
        self.reposition_panel(instance_id, Some(next), None)
    }

    pub(crate) fn toggle_panel_mode(&mut self, instance_id: &str) -> ExtensionSurfaceChanges {
        let Some(surface) = self.surfaces.get(instance_id) else {
            return ExtensionSurfaceChanges::default();
        };
        let ExtensionSurfaceKind::Panel(panel) = &surface.kind else {
            return ExtensionSurfaceChanges::default();
        };
        let next = match panel.mode {
            muxy_core::extensions::manifest::PanelMode::Pinned => {
                muxy_core::extensions::manifest::PanelMode::Floating
            }
            muxy_core::extensions::manifest::PanelMode::Floating => {
                muxy_core::extensions::manifest::PanelMode::Pinned
            }
        };
        self.reposition_panel(instance_id, None, Some(next))
    }

    fn reposition_panel(
        &mut self,
        instance_id: &str,
        position: Option<muxy_core::extensions::manifest::PanelPosition>,
        mode: Option<muxy_core::extensions::manifest::PanelMode>,
    ) -> ExtensionSurfaceChanges {
        let Some(mut surface) = self.surfaces.remove(instance_id) else {
            return ExtensionSurfaceChanges::default();
        };
        let ExtensionSurfaceKind::Panel(panel) = &mut surface.kind else {
            self.surfaces.insert(instance_id.to_owned(), surface);
            return ExtensionSurfaceChanges::default();
        };
        if let Some(position) = position {
            panel.position = position;
        }
        if let Some(mode) = mode {
            panel.mode = mode;
        }
        self.surfaces.insert(instance_id.to_owned(), surface);
        ExtensionSurfaceChanges::default()
    }

    pub(crate) fn close_extension(&mut self, extension_id: &str) -> ExtensionSurfaceChanges {
        let ids = self
            .surfaces
            .iter()
            .filter(|(_, surface)| surface.extension_id == extension_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let mut changes = ExtensionSurfaceChanges::default();
        for id in ids {
            if self.focused_surface_id.as_deref() == Some(id.as_str()) {
                self.focused_surface_id = None;
            }
            if let Some(surface) = self.surfaces.remove(&id) {
                changes.push_closed(surface);
            }
        }
        changes
    }

    pub(crate) fn surface(&self, instance_id: &str) -> Option<&ExtensionSurface> {
        self.surfaces.get(instance_id)
    }

    pub(crate) fn surfaces(&self) -> impl Iterator<Item = &ExtensionSurface> {
        self.surfaces.values()
    }

    pub(crate) fn focus(&mut self, instance_id: &str) -> bool {
        if self.surfaces.contains_key(instance_id) {
            self.focused_surface_id = Some(instance_id.to_owned());
            true
        } else {
            false
        }
    }

    pub(crate) fn focused_surface_id(&self) -> Option<&str> {
        self.focused_surface_id.as_deref()
    }

    pub(crate) fn webview_surfaces(&self) -> impl Iterator<Item = &ExtensionSurface> {
        let active_project_id = self.active_project_id.as_deref();
        self.surfaces
            .values()
            .filter(move |surface| match surface.kind {
                ExtensionSurfaceKind::Panel(_) => {
                    surface.project_id.as_deref() == active_project_id
                }
                _ => true,
            })
    }

    pub(crate) fn active_panels(&self) -> impl Iterator<Item = &ExtensionSurface> {
        let active_project_id = self.active_project_id.as_deref();
        self.surfaces.values().filter(move |surface| {
            active_project_id.is_some()
                && matches!(surface.kind, ExtensionSurfaceKind::Panel(_))
                && surface.project_id.as_deref() == active_project_id
        })
    }

    pub(crate) fn active_popover(&self) -> Option<&ExtensionSurface> {
        self.surfaces
            .values()
            .find(|surface| matches!(surface.kind, ExtensionSurfaceKind::Popover(_)))
    }

    pub(crate) fn active_sidebar(&self) -> Option<&ExtensionSurface> {
        self.surfaces
            .values()
            .find(|surface| matches!(surface.kind, ExtensionSurfaceKind::Sidebar(_)))
    }

    pub(crate) fn active_home(&self) -> Option<&ExtensionSurface> {
        self.surfaces
            .values()
            .find(|surface| matches!(surface.kind, ExtensionSurfaceKind::Home(_)))
    }

    pub(crate) fn active_modal(&self) -> Option<&ExtensionSurface> {
        self.surfaces
            .values()
            .rev()
            .find(|surface| matches!(surface.kind, ExtensionSurfaceKind::Modal { .. }))
    }

    fn next_id(&mut self, kind: &str, extension_id: &str) -> String {
        self.next_instance = self.next_instance.saturating_add(1).max(1);
        format!("extension-{kind}:{extension_id}:{}", self.next_instance)
    }
}

fn enabled_record<'a>(
    catalog: &'a ExtensionRuntimeCatalog,
    extension_id: &str,
) -> Option<&'a muxy_core::extensions::runtime::ExtensionRuntimeRecord> {
    catalog
        .records()
        .get(extension_id)
        .filter(|record| record.enabled)
}

fn declared_surface(
    extension_id: &str,
    instance_id: String,
    project_id: Option<String>,
    resource_root: &std::path::Path,
    entry: &str,
    data: Value,
    kind: ExtensionSurfaceKind,
) -> Result<ExtensionSurface, ExtensionSurfaceError> {
    let entry_url = muxy_core::extensions::assets::extension_asset_url(extension_id, entry)
        .map_err(|_| {
            ExtensionSurfaceError::InvalidAsset(extension_id.to_owned(), entry.to_owned())
        })?
        .to_string();
    Ok(ExtensionSurface {
        extension_id: extension_id.to_owned(),
        instance_id,
        project_id,
        entry_url,
        resource_root: resource_root.to_path_buf(),
        data,
        kind,
    })
}

fn panel_instance_id(extension_id: &str, panel_id: &str, project_id: &str) -> String {
    format!("extension-panel:{extension_id}:{panel_id}:{project_id}")
}

fn popover_instance_id(extension_id: &str, popover_id: &str) -> String {
    format!("extension-popover:{extension_id}:{popover_id}")
}

fn finite_clamp(value: f64, minimum: f64, maximum: f64) -> f64 {
    if value.is_finite() {
        value.clamp(minimum, maximum)
    } else {
        minimum
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_core::environment::{BuildMode, RuntimePathPolicy};
    use muxy_core::extensions::paths::ExtensionPaths;
    use muxy_core::extensions::state::ExtensionStateStore;
    use std::fs;

    fn catalog(root: &std::path::Path) -> ExtensionRuntimeCatalog {
        let paths = ExtensionPaths::new(RuntimePathPolicy::new(BuildMode::Production), root);
        let package = paths.packages.join("fixture");
        fs::create_dir_all(&package).unwrap();
        for entry in ["tab.html", "panel.html", "popover.html", "modal.html"] {
            fs::write(package.join(entry), entry).unwrap();
        }
        fs::write(
            package.join("package.json"),
            r#"{"name":"fixture","version":"1.0.0","muxy":{"tabTypes":[{"id":"tab","title":"Tab","entry":"tab.html"}],"panels":[{"id":"right","entry":"panel.html","position":"right","mode":"pinned"},{"id":"other","entry":"panel.html","position":"right","mode":"pinned"}],"popovers":[{"id":"menu","entry":"popover.html"}],"commands":[{"id":"modal","title":"Modal","action":{"kind":"openModal","entry":"modal.html"}}]}}"#,
        )
        .unwrap();
        let mut state = ExtensionStateStore::open(paths.state_file()).unwrap();
        state.set_enabled("fixture", Some(true)).unwrap();
        ExtensionRuntimeCatalog::load(paths).unwrap()
    }

    #[test]
    fn panel_registry_retains_surfaces_while_shared_host_owns_slot_displacement() {
        let root = tempfile::tempdir().unwrap();
        let catalog = catalog(root.path());
        let mut registry = ExtensionSurfaceRegistry::default();
        registry.set_active_project(Some("one"));
        let first = registry
            .open_panel(
                &catalog,
                "fixture",
                "right",
                Some(serde_json::json!({"a": 1})),
            )
            .unwrap();
        assert_eq!(first.opened.len(), 1);
        let first_id = first.opened[0].instance_id.clone();
        let second = registry
            .open_panel(&catalog, "fixture", "other", None)
            .unwrap();
        assert!(second.closed.is_empty());
        assert_eq!(registry.active_panels().count(), 2);
        assert!(registry.surface(&first_id).is_some());
        registry.set_active_project(Some("two"));
        assert_eq!(registry.active_panels().count(), 0);
        registry
            .open_panel(&catalog, "fixture", "right", None)
            .unwrap();
        let restored = registry.set_active_project(Some("one"));
        assert_eq!(restored.opened.len(), 2);
        assert_eq!(registry.active_panels().count(), 2);
        assert!(registry.surfaces().any(|surface| {
            surface.project_id.as_deref() == Some("two")
                && matches!(surface.kind, ExtensionSurfaceKind::Panel(_))
        }));
    }

    #[test]
    fn popovers_are_singleton_and_modal_dimensions_are_bounded() {
        let root = tempfile::tempdir().unwrap();
        let catalog = catalog(root.path());
        let mut registry = ExtensionSurfaceRegistry::default();
        registry.set_active_project(Some("project"));
        registry
            .toggle_popover(&catalog, "fixture", "menu", None)
            .unwrap();
        assert!(registry.active_popover().is_some());
        registry
            .toggle_popover(&catalog, "fixture", "menu", None)
            .unwrap();
        assert!(registry.active_popover().is_none());
        registry
            .open_modal(
                &catalog,
                "fixture",
                None,
                "modal.html",
                Some(f64::INFINITY),
                Some(10.0),
                true,
                None,
                None,
            )
            .unwrap();
        let ExtensionSurfaceKind::Modal { width, height, .. } =
            &registry.active_modal().unwrap().kind
        else {
            panic!()
        };
        assert_eq!((*width, *height), (120.0, 120.0));
    }
}
