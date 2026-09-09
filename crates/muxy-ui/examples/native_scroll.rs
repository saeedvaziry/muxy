#[allow(unsafe_code, unreachable_pub)]
mod adapter {
    include!("../src/native_scroll.rs");

    pub fn verify() -> Result<(), Box<dyn std::error::Error>> {
        use std::cell::RefCell;
        use std::io::{self, Write};
        use std::rc::Rc;

        use gpui::{point, px, size};
        use objc2_app_kit::{NSBackingStoreType, NSScrollerStyle, NSWindow, NSWindowStyleMask};
        use objc2_foundation::NSString;

        let mtm = MainThreadMarker::new().ok_or("probe requires main thread")?;
        let _app = NSApplication::sharedApplication(mtm);
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::ZERO, NSSize::new(640.0, 480.0)),
                NSWindowStyleMask::Titled,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        window.setTitle(&NSString::from_str("Muxy Native Scroll Probe"));
        let positions = Rc::new(RefCell::new(Vec::new()));
        let received = Rc::clone(&positions);
        let native = NativeScrollView::new("Muxy Native Scroll Probe", move |position| {
            received.borrow_mut().push(position);
        })
        .ok_or("native scroll view missing")?;
        native.view.setScrollerStyle(NSScrollerStyle::Legacy);
        let geometry = ScrollGeometry {
            bounds: Bounds::new(point(px(20.0), px(20.0)), size(px(600.0), px(400.0))),
            content_height: 80_400.0,
            from_bottom: 12.5,
            line_height: 16.0,
            revision: 1,
            dark: true,
        };
        assert!(native.forward_wheel().is_none());
        native.sync(geometry);
        native.set_visible(true);
        native.view.tile();
        let scrollbar = native
            .view
            .verticalScroller()
            .ok_or("native scrollbar missing")?;
        assert!(scrollbar.isKindOfClass(NSScroller::class()));
        assert!((native.view.position().from_bottom - 12.5).abs() < 0.01);
        assert!(native.reserved_width() > 0.0);
        let frame = native.view.frame();
        assert!(
            native
                .view
                .hitTest(NSPoint::new(frame.origin.x + 30.0, frame.origin.y + 30.0))
                .is_none()
        );
        let track = scrollbar.frame();
        let hit = native
            .view
            .hitTest(NSPoint::new(
                frame.origin.x + track.origin.x + track.size.width / 2.0,
                frame.origin.y + track.origin.y + track.size.height / 2.0,
            ))
            .ok_or("scrollbar is not hittable")?;
        assert!(hit.isKindOfClass(NSScroller::class()));
        assert!(positions.borrow().is_empty());
        let clip = native.view.contentView();
        let maximum = native.document.frame().size.height - clip.bounds().size.height;
        clip.scrollToPoint(NSPoint::new(0.0, maximum - 128.5));
        native.view.reflectScrolledClipView(&clip);
        let position = *positions.borrow().last().ok_or("missing native callback")?;
        assert_eq!(position.revision, 1);
        assert!(position.sequence > 0);
        assert!((position.from_bottom - 128.5).abs() < 0.01);
        native.sync(ScrollGeometry {
            content_height: 88_400.0,
            ..geometry
        });
        assert!((native.view.position().from_bottom - 128.5).abs() < 0.01);
        verify_wheel(&native)?;
        native.sync(ScrollGeometry {
            from_bottom: 0.0,
            revision: 2,
            ..geometry
        });
        assert!(native.view.position().from_bottom.abs() < 0.01);
        assert!(native.reserved_width() > 0.0);
        verify_scrollbar(&native)?;
        verify_empty_resize(&native)?;
        native.set_visible(false);
        assert!(native.view.isHidden());
        let retained = native.view.clone();
        drop(native);
        assert!(unsafe { retained.superview() }.is_none());
        window.close();
        writeln!(
            io::stdout(),
            "AppKit native probe: NSScrollView + NSScroller present; fractional positions 12.5 and 128.5 preserved; native wheel, thumb drag, and track click moved content; content growth kept position; native scrollbar hit testing, reserved width after reset, bottom reset, empty-content grow/shrink in both scrollbar styles, hide, and detach passed"
        )?;
        Ok(())
    }

    fn verify_empty_resize(native: &NativeScrollView) -> Result<(), Box<dyn std::error::Error>> {
        use gpui::{point, px, size};
        use objc2_app_kit::NSScrollerStyle;

        for style in [NSScrollerStyle::Legacy, NSScrollerStyle::Overlay] {
            native.view.setScrollerStyle(style);
            for height in [400.0, 700.0, 300.0, 550.0, 200.0, 400.0] {
                native.sync(ScrollGeometry {
                    bounds: Bounds::new(point(px(20.0), px(20.0)), size(px(600.0), px(height))),
                    content_height: f64::from(height),
                    from_bottom: 0.0,
                    line_height: 16.0,
                    revision: native.view.ivars().revision.get() + 1,
                    dark: true,
                });
                native.view.tile();
                let scrollbar = native.view.verticalScroller().ok_or("missing scrollbar")?;
                assert!(
                    scrollbar.isHidden(),
                    "empty content at height {height}, style {style:?}"
                );
                assert!(native.reserved_width().abs() < 0.01);
                assert!(native.view.position().from_bottom.abs() < 0.01);
                assert!((native.document.frame().size.height - f64::from(height)).abs() < 0.01);
            }
        }
        Ok(())
    }

    fn verify_wheel(native: &NativeScrollView) -> Result<(), Box<dyn std::error::Error>> {
        use objc2_app_kit::NSEvent;
        use std::ffi::c_void;

        #[repr(C)]
        struct Event {
            opaque: [u8; 0],
        }

        unsafe impl objc2::RefEncode for Event {
            const ENCODING_REF: objc2::Encoding =
                objc2::Encoding::Pointer(&objc2::Encoding::Struct("__CGEvent", &[]));
        }

        #[link(name = "ApplicationServices", kind = "framework")]
        unsafe extern "C" {
            fn CGEventCreateScrollWheelEvent2(
                source: *const c_void,
                units: u32,
                count: u32,
                wheel1: i32,
                wheel2: i32,
                wheel3: i32,
            ) -> *mut Event;
            fn CFRelease(value: *const c_void);
        }
        let event = unsafe { CGEventCreateScrollWheelEvent2(std::ptr::null(), 0, 1, 24, 0, 0) };
        if event.is_null() {
            return Err("could not create native wheel event".into());
        }
        let native_event: Option<Retained<NSEvent>> =
            unsafe { msg_send![NSEvent::class(), eventWithCGEvent: event] };
        unsafe {
            CFRelease(event.cast());
        }
        let native_event = native_event.ok_or("could not wrap native wheel event")?;
        let before = native.view.position().from_bottom;
        native.view.scrollWheel(&native_event);
        assert!(native.view.position().from_bottom > before);
        Ok(())
    }

    fn verify_scrollbar(native: &NativeScrollView) -> Result<(), Box<dyn std::error::Error>> {
        use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSScrollerPart};

        let app = NSApplication::sharedApplication(native.view.mtm());
        let window = native.view.window().ok_or("missing native window")?;
        window.makeKeyAndOrderFront(None);
        let scrollbar = native.view.verticalScroller().ok_or("missing scrollbar")?;
        let knob = scrollbar.rectForPart(NSScrollerPart::Knob);
        let start = NSPoint::new(
            knob.origin.x + knob.size.width / 2.0,
            knob.origin.y + knob.size.height / 2.0,
        );
        let end = NSPoint::new(start.x, scrollbar.bounds().size.height * 0.4);
        let event = |kind, point| {
            NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind,
                scrollbar.convertPoint_toView(point, None),
                NSEventModifierFlags::empty(),
                0.0,
                window.windowNumber(),
                None,
                1,
                1,
                1.0,
            ).ok_or("could not create native mouse event")
        };
        let before = native.view.position().from_bottom;
        app.postEvent_atStart(event(NSEventType::LeftMouseDragged, end)?.as_ref(), false);
        app.postEvent_atStart(event(NSEventType::LeftMouseUp, end)?.as_ref(), false);
        scrollbar.mouseDown(event(NSEventType::LeftMouseDown, start)?.as_ref());
        let dragged = native.view.position().from_bottom;
        assert!(
            (dragged - before).abs() > 1.0,
            "drag {before} -> {dragged}; knob {knob:?}; start {start:?}; end {end:?}; enabled {}; hidden {}; hit {:?}",
            scrollbar.isEnabled(),
            scrollbar.isHidden(),
            scrollbar.hitPart()
        );
        let track = NSPoint::new(start.x, scrollbar.bounds().size.height * 0.8);
        app.postEvent_atStart(event(NSEventType::LeftMouseUp, track)?.as_ref(), false);
        scrollbar.mouseDown(event(NSEventType::LeftMouseDown, track)?.as_ref());
        let class = objc2::runtime::AnyClass::get(c"NSRunLoop").ok_or("missing run loop class")?;
        let run_loop: Retained<objc2_foundation::NSObject> =
            unsafe { msg_send![class, currentRunLoop] };
        let class = objc2::runtime::AnyClass::get(c"NSDate").ok_or("missing date class")?;
        let date: Retained<objc2_foundation::NSObject> =
            unsafe { msg_send![class, dateWithTimeIntervalSinceNow: 0.2_f64] };
        unsafe {
            let _: () = msg_send![&*run_loop, runUntilDate: &*date];
        }
        assert!((native.view.position().from_bottom - dragged).abs() > 1.0);
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    adapter::verify()
}
