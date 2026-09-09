use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::panic::Location;
use std::rc::{Rc, Weak};

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Refineable as _, Rgba, Style, StyleRefinement, Styled, Window,
};
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{DefinedClass, MainThreadOnly, Message as _, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSRectFill, NSTrackingArea, NSTrackingAreaOptions, NSView,
    NSWindowOrderingMode,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NativeViewId(u64);

impl NativeViewId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for NativeViewId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Error)]
pub enum NativeViewCompositorError {
    #[error("GPUI did not provide a native window handle: {0:?}")]
    WindowHandle(raw_window_handle::HandleError),

    #[error("the GPUI window is not backed by an AppKit NSView")]
    UnsupportedWindowHandle,

    #[error("native AppKit views must be attached on the main thread")]
    NotMainThread,

    #[error("the GPUI NSView is not attached to an AppKit superview")]
    DetachedGpuiView,

    #[error("the NSView is already registered as native view {0}")]
    DuplicateView(NativeViewId),

    #[error("the GPUI or native container NSView cannot be registered")]
    ReservedView,

    #[error("native view identity space is exhausted")]
    IdentityExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NativeBackdrop {
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
}

impl NativeBackdrop {
    fn new(red: f64, green: f64, blue: f64, alpha: f64) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    fn is_opaque(self) -> bool {
        self.alpha >= 1.0
    }
}

impl From<Rgba> for NativeBackdrop {
    fn from(color: Rgba) -> Self {
        Self::new(
            color.r.into(),
            color.g.into(),
            color.b.into(),
            color.a.into(),
        )
    }
}

struct NativeContainerIvars {
    backdrop: Cell<NativeBackdrop>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "MuxyNativeViewContainer"]
    #[ivars = NativeContainerIvars]
    struct FlippedNativeViewContainer;

    impl FlippedNativeViewContainer {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            self.ivars().backdrop.get().is_opaque()
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty_rect: NSRect) {
            let backdrop = self.ivars().backdrop.get();
            NSColor::colorWithSRGBRed_green_blue_alpha(
                backdrop.red,
                backdrop.green,
                backdrop.blue,
                backdrop.alpha,
            )
            .setFill();
            NSRectFill(dirty_rect);
        }
    }

    unsafe impl NSObjectProtocol for FlippedNativeViewContainer {}
);

impl FlippedNativeViewContainer {
    fn new(mtm: MainThreadMarker, frame: NSRect, backdrop: NativeBackdrop) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NativeContainerIvars {
            backdrop: Cell::new(backdrop),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        this
    }

    fn set_backdrop(&self, backdrop: NativeBackdrop) {
        if self.ivars().backdrop.replace(backdrop) != backdrop {
            self.setNeedsDisplay(true);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum NativeViewLayer {
    Underlay,
    Interactive,
}

#[derive(Clone)]
pub struct NativeViewCompositor {
    inner: Rc<CompositorInner>,
}

impl NativeViewCompositor {
    pub fn new(window: &Window, backdrop: Rgba) -> Result<Self, NativeViewCompositorError> {
        let mtm = MainThreadMarker::new().ok_or(NativeViewCompositorError::NotMainThread)?;
        let handle = HasWindowHandle::window_handle(window)
            .map_err(NativeViewCompositorError::WindowHandle)?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err(NativeViewCompositorError::UnsupportedWindowHandle);
        };

        let gpui_view = unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }
            .expect("a raw-window-handle AppKit NSView is non-null");

        let superview =
            unsafe { gpui_view.superview() }.ok_or(NativeViewCompositorError::DetachedGpuiView)?;

        track_mouse_moved(&gpui_view);

        let initial_frame = gpui_view.frame();
        let container =
            FlippedNativeViewContainer::new(mtm, initial_frame, NativeBackdrop::from(backdrop));
        container.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        superview.addSubview_positioned_relativeTo(
            &container,
            NSWindowOrderingMode::Below,
            Some(&gpui_view),
        );

        Ok(Self {
            inner: Rc::new(CompositorInner {
                container,
                gpui_view,
                superview,
                model: RefCell::new(RegistryModel::default()),
                views: RefCell::new(HashMap::new()),
                container_frame: Cell::new(NativeFrame::from_ns_rect(initial_frame)),
            }),
        })
    }

    pub fn register(
        &self,
        view: &NSView,
        z_index: i32,
    ) -> Result<NativeViewRegistration, NativeViewCompositorError> {
        self.register_in_layer(view, z_index, NativeViewLayer::Underlay)
    }

    pub fn register_interactive(
        &self,
        view: &NSView,
        z_index: i32,
    ) -> Result<NativeViewRegistration, NativeViewCompositorError> {
        self.register_in_layer(view, z_index, NativeViewLayer::Interactive)
    }

    fn register_in_layer(
        &self,
        view: &NSView,
        z_index: i32,
        layer: NativeViewLayer,
    ) -> Result<NativeViewRegistration, NativeViewCompositorError> {
        let view_ptr = view as *const NSView;
        if view_ptr == Retained::as_ptr(&self.inner.gpui_view)
            || view_ptr.cast::<FlippedNativeViewContainer>()
                == Retained::as_ptr(&self.inner.container)
        {
            return Err(NativeViewCompositorError::ReservedView);
        }

        if let Some(id) = self.inner.id_for_view(view_ptr) {
            return Err(NativeViewCompositorError::DuplicateView(id));
        }

        let registration = self
            .inner
            .model
            .borrow_mut()
            .register_in_layer(z_index, layer)
            .ok_or(NativeViewCompositorError::IdentityExhausted)?;
        self.inner
            .views
            .borrow_mut()
            .insert(registration.id, view.retain());
        view.setClipsToBounds(true);
        self.inner.apply_order(&registration.order);

        if view.isHidden() {
            view.setHidden(false);
        }

        Ok(NativeViewRegistration {
            id: registration.id,
            inner: Rc::downgrade(&self.inner),
        })
    }

    pub fn set_visible(&self, id: NativeViewId, visible: bool) -> bool {
        self.inner.set_visible(id, visible)
    }

    pub fn set_z_index(&self, id: NativeViewId, z_index: i32) -> bool {
        self.inner.set_z_index(id, z_index)
    }

    pub fn focus_gpui(&self) -> bool {
        self.inner.focus_gpui()
    }

    pub fn gpui_view(&self) -> Retained<NSView> {
        self.inner.gpui_view.clone()
    }

    pub fn set_backdrop(&self, backdrop: Rgba) {
        self.inner
            .container
            .set_backdrop(NativeBackdrop::from(backdrop));
    }
}

fn track_mouse_moved(view: &NSView) {
    use objc2::AnyThread;

    let options = NSTrackingAreaOptions::MouseMoved
        | NSTrackingAreaOptions::ActiveInKeyWindow
        | NSTrackingAreaOptions::InVisibleRect
        | NSTrackingAreaOptions::EnabledDuringMouseDrag;
    let area = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            view.bounds(),
            options,
            Some(view),
            None,
        )
    };
    view.addTrackingArea(&area);
}

struct CompositorInner {
    container: Retained<FlippedNativeViewContainer>,
    gpui_view: Retained<NSView>,
    superview: Retained<NSView>,
    model: RefCell<RegistryModel>,
    views: RefCell<HashMap<NativeViewId, Retained<NSView>>>,
    container_frame: Cell<NativeFrame>,
}

impl CompositorInner {
    fn id_for_view(&self, needle: *const NSView) -> Option<NativeViewId> {
        self.views
            .borrow()
            .iter()
            .find_map(|(id, view)| (Retained::as_ptr(view) == needle.cast_mut()).then_some(*id))
    }

    fn remove(&self, id: NativeViewId) -> bool {
        if !self.model.borrow_mut().remove(id) {
            return false;
        }

        if let Some(view) = self.views.borrow_mut().remove(&id) {
            view.removeFromSuperview();
        }
        true
    }

    fn set_visible(&self, id: NativeViewId, visible: bool) -> bool {
        if !self.model.borrow_mut().set_visible(id, visible) {
            return false;
        }

        if let Some(view) = self.views.borrow().get(&id) {
            view.setHidden(!visible);
        }
        true
    }

    fn set_z_index(&self, id: NativeViewId, z_index: i32) -> bool {
        let decision = self.model.borrow_mut().set_z_index(id, z_index);
        let Some(decision) = decision else {
            return false;
        };

        if let Some(order) = decision.order {
            self.apply_order(&order);
        }
        true
    }

    fn sync_frame(&self, id: NativeViewId, frame: NativeFrame) -> bool {
        let Some(layer) = self.model.borrow().layer(id) else {
            return false;
        };
        let frame = match layer {
            NativeViewLayer::Underlay => frame,
            NativeViewLayer::Interactive => NativeFrame::from_ns_rect(
                self.container
                    .convertRect_toView(frame.to_ns_rect(), Some(&self.superview)),
            ),
        };
        if !self.model.borrow_mut().sync_frame(id, frame) {
            return false;
        }

        if let Some(view) = self.views.borrow().get(&id) {
            view.setFrame(frame.to_ns_rect());
        }
        true
    }

    fn sync_container_frame(&self) {
        let frame = NativeFrame::from_ns_rect(self.gpui_view.frame());
        if self.container_frame.get() == frame {
            return;
        }

        self.container.setFrame(frame.to_ns_rect());
        self.container_frame.set(frame);
    }

    fn focus(&self, id: NativeViewId) -> bool {
        if !self.model.borrow().is_visible(id) {
            return false;
        }

        let views = self.views.borrow();
        let Some(view) = views.get(&id) else {
            return false;
        };
        let Some(window) = view.window() else {
            return false;
        };

        if window.firstResponder().is_some_and(|responder| {
            Retained::as_ptr(&responder).cast::<()>() == Retained::as_ptr(view).cast::<()>()
        }) {
            return true;
        }

        window.makeFirstResponder(Some(view))
    }

    fn focus_gpui(&self) -> bool {
        let Some(window) = self.gpui_view.window() else {
            return false;
        };
        if window.firstResponder().is_some_and(|responder| {
            Retained::as_ptr(&responder).cast::<()>()
                == Retained::as_ptr(&self.gpui_view).cast::<()>()
        }) {
            return true;
        }
        window.makeFirstResponder(Some(&self.gpui_view))
    }

    fn apply_order(&self, order: &[NativeViewId]) {
        let model = self.model.borrow();
        let views = self.views.borrow();
        let mut underlay_views = Vec::new();
        let mut interactive_views = Vec::new();
        for id in order {
            let Some(view) = views.get(id).cloned() else {
                continue;
            };
            match model.layer(*id) {
                Some(NativeViewLayer::Underlay) => underlay_views.push(view),
                Some(NativeViewLayer::Interactive) => interactive_views.push(view),
                None => {}
            }
        }
        debug_assert_eq!(underlay_views.len() + interactive_views.len(), order.len());

        self.container
            .setSubviews(&NSArray::from_retained_slice(&underlay_views));
        let mut relative = self.gpui_view.clone();
        for view in interactive_views {
            self.superview.addSubview_positioned_relativeTo(
                &view,
                NSWindowOrderingMode::Above,
                Some(&relative),
            );
            relative = view;
        }
    }
}

impl Drop for CompositorInner {
    fn drop(&mut self) {
        self.container.removeFromSuperview();
        for view in self.views.get_mut().values() {
            view.removeFromSuperview();
        }
        self.views.get_mut().clear();
    }
}

pub struct NativeViewRegistration {
    id: NativeViewId,
    inner: Weak<CompositorInner>,
}

impl NativeViewRegistration {
    pub fn set_visible(&self, visible: bool) -> bool {
        self.inner
            .upgrade()
            .is_some_and(|inner| inner.set_visible(self.id, visible))
    }

    pub fn sync_frame(&self, bounds: Bounds<Pixels>) -> bool {
        self.inner.upgrade().is_some_and(|inner| {
            inner.sync_container_frame();
            inner.sync_frame(self.id, NativeFrame::from_gpui_bounds(bounds))
        })
    }

    pub fn focus(&self) -> bool {
        self.inner
            .upgrade()
            .is_some_and(|inner| inner.focus(self.id))
    }

    #[track_caller]
    pub fn slot(&self) -> Option<NativeViewSlot> {
        self.inner
            .upgrade()
            .map(|inner| NativeViewSlot::new(NativeViewCompositor { inner }, self.id))
    }
}

impl Drop for NativeViewRegistration {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner.remove(self.id);
        }
    }
}

pub struct NativeViewSlot {
    compositor: NativeViewCompositor,
    id: NativeViewId,
    style: StyleRefinement,
    visible: Option<bool>,
    z_index: Option<i32>,
    source: &'static Location<'static>,
}

impl NativeViewSlot {
    #[track_caller]
    fn new(compositor: NativeViewCompositor, id: NativeViewId) -> Self {
        Self {
            compositor,
            id,
            style: StyleRefinement::default(),
            visible: None,
            z_index: None,
            source: Location::caller(),
        }
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = Some(visible);
        self
    }
}

impl IntoElement for NativeViewSlot {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for NativeViewSlot {
    type RequestLayoutState = Style;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::NamedInteger(
            "native-view-slot".into(),
            self.id.get(),
        ))
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        Some(self.source)
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
        self.compositor.inner.sync_container_frame();
        if let Some(z_index) = self.z_index {
            self.compositor.set_z_index(self.id, z_index);
        }
        if let Some(visible) = self.visible {
            self.compositor.set_visible(self.id, visible);
        }
        self.compositor
            .inner
            .sync_frame(self.id, NativeFrame::from_gpui_bounds(bounds));
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
}

impl Styled for NativeViewSlot {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct NativeFrame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl NativeFrame {
    fn from_gpui_bounds(bounds: Bounds<Pixels>) -> Self {
        Self {
            x: f64::from(f32::from(bounds.origin.x)),
            y: f64::from(f32::from(bounds.origin.y)),
            width: f64::from(f32::from(bounds.size.width)),
            height: f64::from(f32::from(bounds.size.height)),
        }
    }

    fn from_ns_rect(rect: NSRect) -> Self {
        Self {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        }
    }

    fn to_ns_rect(self) -> NSRect {
        NSRect::new(
            NSPoint::new(self.x, self.y),
            NSSize::new(self.width, self.height),
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct RegistryEntry {
    z_index: i32,
    sequence: u64,
    layer: NativeViewLayer,
    visible: bool,
    frame: Option<NativeFrame>,
}

#[derive(Debug, Default)]
struct RegistryModel {
    next_id: u64,
    next_sequence: u64,
    entries: HashMap<NativeViewId, RegistryEntry>,
    order: Vec<NativeViewId>,
}

#[derive(Debug, PartialEq)]
struct RegisterDecision {
    id: NativeViewId,
    order: Vec<NativeViewId>,
}

#[derive(Debug, PartialEq)]
struct ZIndexDecision {
    order: Option<Vec<NativeViewId>>,
}

impl RegistryModel {
    #[cfg(test)]
    fn register(&mut self, z_index: i32) -> Option<RegisterDecision> {
        self.register_in_layer(z_index, NativeViewLayer::Underlay)
    }

    fn register_in_layer(
        &mut self,
        z_index: i32,
        layer: NativeViewLayer,
    ) -> Option<RegisterDecision> {
        let id_value = self.next_id.checked_add(1)?;
        let sequence = self.next_sequence;
        let next_sequence = sequence.checked_add(1)?;

        self.next_id = id_value;
        self.next_sequence = next_sequence;
        let id = NativeViewId(id_value);
        self.entries.insert(
            id,
            RegistryEntry {
                z_index,
                sequence,
                layer,
                visible: true,
                frame: None,
            },
        );
        self.order = self.sorted_ids();

        Some(RegisterDecision {
            id,
            order: self.order.clone(),
        })
    }

    fn remove(&mut self, id: NativeViewId) -> bool {
        if self.entries.remove(&id).is_none() {
            return false;
        }
        self.order.retain(|candidate| *candidate != id);
        true
    }

    fn set_visible(&mut self, id: NativeViewId, visible: bool) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        if entry.visible == visible {
            return false;
        }
        entry.visible = visible;
        true
    }

    fn set_z_index(&mut self, id: NativeViewId, z_index: i32) -> Option<ZIndexDecision> {
        let entry = self.entries.get_mut(&id)?;
        if entry.z_index == z_index {
            return None;
        }
        entry.z_index = z_index;

        let new_order = self.sorted_ids();
        let order = (new_order != self.order).then(|| new_order.clone());
        self.order = new_order;
        Some(ZIndexDecision { order })
    }

    fn sync_frame(&mut self, id: NativeViewId, frame: NativeFrame) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        if entry.frame == Some(frame) {
            return false;
        }
        entry.frame = Some(frame);
        true
    }

    fn is_visible(&self, id: NativeViewId) -> bool {
        self.entries.get(&id).is_some_and(|entry| entry.visible)
    }

    fn layer(&self, id: NativeViewId) -> Option<NativeViewLayer> {
        self.entries.get(&id).map(|entry| entry.layer)
    }

    fn sorted_ids(&self) -> Vec<NativeViewId> {
        let mut ordered: Vec<_> = self.entries.iter().collect();
        ordered.sort_by_key(|(_, entry)| (entry.layer, entry.z_index, entry.sequence));
        ordered.into_iter().map(|(id, _)| *id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{NativeBackdrop, NativeFrame, NativeViewLayer, RegistryModel};

    fn frame(x: f64, y: f64, width: f64, height: f64) -> NativeFrame {
        NativeFrame {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn orders_by_z_index_then_registration_sequence() {
        let mut registry = RegistryModel::default();
        let first = registry.register(10).unwrap().id;
        let second = registry.register(-2).unwrap().id;
        let third = registry.register(10).unwrap().id;
        let fourth = registry.register(-2).unwrap().id;

        assert_eq!(registry.order, vec![second, fourth, first, third]);
    }

    #[test]
    fn interactive_views_are_ordered_above_underlay_views() {
        let mut registry = RegistryModel::default();
        let interactive = registry
            .register_in_layer(-100, NativeViewLayer::Interactive)
            .unwrap()
            .id;
        let underlay = registry.register(100).unwrap().id;

        assert_eq!(registry.order, vec![underlay, interactive]);
        assert_eq!(registry.layer(underlay), Some(NativeViewLayer::Underlay));
        assert_eq!(
            registry.layer(interactive),
            Some(NativeViewLayer::Interactive)
        );
    }

    #[test]
    fn hide_show_only_emits_real_transitions() {
        let mut registry = RegistryModel::default();
        let id = registry.register(0).unwrap().id;

        assert!(!registry.set_visible(id, true));
        assert!(registry.set_visible(id, false));
        assert!(!registry.set_visible(id, false));
        assert!(registry.set_visible(id, true));
        assert!(!registry.set_visible(id, true));
    }

    #[test]
    fn unchanged_frame_is_a_no_op() {
        let mut registry = RegistryModel::default();
        let id = registry.register(0).unwrap().id;
        let initial = frame(12.0, 24.0, 640.0, 480.0);

        assert!(registry.sync_frame(id, initial));
        assert!(!registry.sync_frame(id, initial));
        assert!(registry.sync_frame(id, frame(12.0, 24.0, 641.0, 480.0)));
    }

    #[test]
    fn z_index_reorders_only_when_total_order_changes() {
        let mut registry = RegistryModel::default();
        let first = registry.register(0).unwrap().id;
        let second = registry.register(1).unwrap().id;
        let third = registry.register(2).unwrap().id;

        let changed_without_reorder = registry.set_z_index(second, 0).unwrap();
        assert_eq!(changed_without_reorder.order, None);
        assert_eq!(registry.order, vec![first, second, third]);

        let reordered = registry.set_z_index(third, -1).unwrap();
        assert_eq!(reordered.order, Some(vec![third, first, second]));
        assert_eq!(registry.order, vec![third, first, second]);

        assert_eq!(registry.set_z_index(third, -1), None);
    }

    #[test]
    fn stale_removal_and_updates_are_no_ops() {
        let mut registry = RegistryModel::default();
        let removed = registry.register(0).unwrap().id;
        let survivor = registry.register(0).unwrap().id;

        assert!(registry.remove(removed));
        assert!(!registry.remove(removed));
        assert!(!registry.set_visible(removed, false));
        assert_eq!(registry.set_z_index(removed, 10), None);
        assert!(!registry.sync_frame(removed, frame(0.0, 0.0, 1.0, 1.0)));
        assert_eq!(registry.order, vec![survivor]);

        let new_id = registry.register(0).unwrap().id;
        assert!(new_id > removed, "removed ids must never be reused");
        assert_eq!(registry.order, vec![survivor, new_id]);
    }

    #[test]
    fn native_container_backdrop_preserves_opacity() {
        let opaque = NativeBackdrop::new(0.1, 0.2, 0.3, 1.0);
        let transparent = NativeBackdrop::new(0.1, 0.2, 0.3, 0.0);

        assert_eq!(opaque.red, 0.1);
        assert_eq!(opaque.green, 0.2);
        assert_eq!(opaque.blue, 0.3);
        assert_eq!(opaque.alpha, 1.0);
        assert!(opaque.is_opaque());
        assert!(!transparent.is_opaque());
    }
}
