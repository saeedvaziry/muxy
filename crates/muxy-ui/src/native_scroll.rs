use std::cell::Cell;
use std::fmt;

use gpui::{Bounds, Pixels};
use objc2::rc::Retained;
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSApplication, NSBorderType, NSClipView, NSEventType, NSScrollElasticity, NSScrollView,
    NSScroller, NSScrollerStyle, NSView,
};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize};

#[derive(Clone, Copy, Debug)]
pub struct ScrollPosition {
    pub revision: u64,
    pub sequence: u64,
    pub from_bottom: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct ScrollGeometry {
    pub bounds: Bounds<Pixels>,
    pub content_height: f64,
    pub from_bottom: f64,
    pub line_height: f64,
    pub revision: u64,
    pub dark: bool,
}

struct ScrollState {
    callback: Box<dyn Fn(ScrollPosition)>,
    updating: Cell<bool>,
    revision: Cell<u64>,
    sequence: Cell<u64>,
    document_height: Cell<f64>,
}

impl fmt::Debug for ScrollState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScrollState").finish_non_exhaustive()
    }
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    #[name = "MuxyScrollDocument"]
    struct Document;

    unsafe impl NSObjectProtocol for Document {}

    impl Document {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
    }
);

define_class!(
    #[unsafe(super = NSScrollView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ScrollState]
    #[derive(Debug)]
    #[name = "MuxyScrollView"]
    struct ScrollView;

    unsafe impl NSObjectProtocol for ScrollView {}

    impl ScrollView {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
            hit.filter(|view| view.isKindOfClass(NSScroller::class()))
        }

        #[unsafe(method(reflectScrolledClipView:))]
        fn reflected(&self, clip: &NSClipView) {
            unsafe { let _: () = msg_send![super(self), reflectScrolledClipView: clip]; }
            let state = self.ivars();
            if !state.updating.get() {
                state.sequence.set(state.sequence.get().wrapping_add(1));
                (state.callback)(self.position());
            }
        }
    }
);

impl ScrollView {
    fn position(&self) -> ScrollPosition {
        let bounds = self.contentView().bounds();
        ScrollPosition {
            revision: self.ivars().revision.get(),
            sequence: self.ivars().sequence.get(),
            from_bottom: (self.ivars().document_height.get() - bounds.size.height).max(0.0)
                - bounds.origin.y,
        }
    }
}

#[derive(Debug)]
pub struct NativeScrollView {
    view: Retained<ScrollView>,
    document: Retained<Document>,
    parent: Retained<NSView>,
}

impl NativeScrollView {
    pub fn new(window_title: &str, callback: impl Fn(ScrollPosition) + 'static) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let app = NSApplication::sharedApplication(mtm);
        let windows = app.windows();
        let window = (0..windows.count())
            .map(|index| windows.objectAtIndex(index))
            .find(|window| window.title().to_string() == window_title)?;
        let parent = window.contentView()?;
        let allocated = ScrollView::alloc(mtm).set_ivars(ScrollState {
            callback: Box::new(callback),
            updating: Cell::new(true),
            revision: Cell::new(u64::MAX),
            sequence: Cell::new(0),
            document_height: Cell::new(0.0),
        });
        let view: Retained<ScrollView> =
            unsafe { msg_send![super(allocated), initWithFrame: NSRect::ZERO] };
        let allocated = Document::alloc(mtm).set_ivars(());
        let document: Retained<Document> =
            unsafe { msg_send![super(allocated), initWithFrame: NSRect::ZERO] };
        view.setBorderType(NSBorderType::NoBorder);
        view.setDrawsBackground(false);
        view.contentView().setDrawsBackground(false);
        view.setHasVerticalScroller(true);
        view.setHasHorizontalScroller(false);
        view.setAutohidesScrollers(true);
        view.setScrollerStyle(NSScrollerStyle::Overlay);
        view.setHorizontalScrollElasticity(NSScrollElasticity::None);
        view.setVerticalScrollElasticity(NSScrollElasticity::Automatic);
        view.setUsesPredominantAxisScrolling(true);
        view.setDocumentView(Some(&document));
        view.setWantsLayer(true);
        view.setHidden(true);
        parent.addSubview(&view);
        view.ivars().updating.set(false);
        Some(Self {
            view,
            document,
            parent,
        })
    }

    pub fn set_visible(&self, visible: bool) {
        self.view.setHidden(!visible);
    }

    pub fn sync(&self, geometry: ScrollGeometry) {
        let state = self.view.ivars();
        state.updating.set(true);
        let old_offset = self.view.position().from_bottom;
        let old_frame = self.view.frame();
        let old_height = state.document_height.get();
        let bounds = geometry.bounds;
        let top = f64::from(f32::from(bounds.top()));
        let height = f64::from(f32::from(bounds.size.height));
        let frame = NSRect::new(
            NSPoint::new(
                f64::from(f32::from(bounds.left())),
                if self.parent.isFlipped() {
                    top
                } else {
                    self.parent.bounds().size.height - top - height
                },
            ),
            NSSize::new(f64::from(f32::from(bounds.size.width)), height),
        );
        let changed = state.revision.replace(geometry.revision) != geometry.revision;
        self.view.setFrame(frame);
        let appearance = NSAppearance::appearanceNamed(unsafe {
            if geometry.dark {
                NSAppearanceNameDarkAqua
            } else {
                NSAppearanceNameAqua
            }
        });
        self.view.setAppearance(appearance.as_deref());
        self.view.setVerticalLineScroll(geometry.line_height);
        let content_height = geometry.content_height.max(self.view.contentSize().height);
        state.document_height.set(content_height);
        self.document
            .setFrameSize(NSSize::new(self.view.contentSize().width, content_height));
        if changed || old_frame != frame || (old_height - content_height).abs() > 0.01 {
            let from_bottom = if changed {
                geometry.from_bottom
            } else {
                old_offset
            };
            let maximum = (content_height - self.view.contentSize().height).max(0.0);
            self.view.contentView().scrollToPoint(NSPoint::new(
                0.0,
                (maximum - from_bottom).clamp(0.0, maximum),
            ));
            self.view.reflectScrolledClipView(&self.view.contentView());
        }
        state.updating.set(false);
    }

    #[allow(clippy::cast_possible_truncation)]
    pub fn reserved_width(&self) -> f32 {
        (self.view.frame().size.width - self.view.contentSize().width).max(0.0) as f32
    }

    pub fn forward_wheel(&self) -> Option<ScrollPosition> {
        let event = NSApplication::sharedApplication(self.view.mtm()).currentEvent()?;
        if event.r#type() != NSEventType::ScrollWheel || self.view.isHidden() {
            return None;
        }
        self.view.scrollWheel(&event);
        Some(self.view.position())
    }
}

impl Drop for NativeScrollView {
    fn drop(&mut self) {
        self.view.ivars().updating.set(true);
        self.view.removeFromSuperview();
    }
}
