use std::time::Duration;

use gpui::{Context, Task, Window};

use super::pane::{PaneState, TerminalPane};

const BLINK_INTERVAL: Duration = Duration::from_millis(530);

pub(crate) struct CursorBlink {
    pub(crate) enabled: bool,
    pub(crate) visible: bool,
    task: Option<Task<()>>,
}

impl Default for CursorBlink {
    fn default() -> Self {
        Self {
            enabled: true,
            visible: true,
            task: None,
        }
    }
}

impl CursorBlink {
    pub(crate) fn reset(&mut self) {
        self.task = None;
        self.visible = true;
    }
}

impl TerminalPane {
    fn should_blink_cursor(&self, window: &Window) -> bool {
        self.cursor_blink.enabled
            && self.state == PaneState::Live
            && self.focused
            && self.native_visible
            && self.focus.is_focused(window)
            && window.is_window_active()
            && self.scroll.view.is_none()
            && self.grid.as_ref().is_some_and(|grid| {
                grid.cursor.visible
                    && grid.cursor.col < grid.size.cols
                    && grid.cursor.row < self.viewport().unwrap_or(grid.size).rows
            })
    }

    pub(crate) fn sync_cursor_blink(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.should_blink_cursor(window) {
            self.cursor_blink.reset();
        } else if self.cursor_blink.task.is_none() {
            self.cursor_blink.task = Some(cx.spawn_in(window, async move |pane, cx| {
                loop {
                    cx.background_executor().timer(BLINK_INTERVAL).await;
                    let keep_blinking = pane.update_in(cx, |pane, window, cx| {
                        let blinking = pane.should_blink_cursor(window);
                        if blinking {
                            pane.cursor_blink.visible = !pane.cursor_blink.visible;
                        } else {
                            pane.cursor_blink.reset();
                        }
                        cx.notify();
                        blinking
                    });
                    if !matches!(keep_blinking, Ok(true)) {
                        break;
                    }
                }
            }));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use gpui::{Entity, TestAppContext, VisualTestContext};
    use muxy_client::{Attachment, RunGrid};
    use muxy_protocol::{ChannelId, Cursor, MetadataEvent, Modes, ScreenFrame, ServerPath, Size};

    use crate::views::terminal::colors::Palette;

    fn attachment() -> Attachment {
        Attachment {
            channel: ChannelId(1),
            grid: RunGrid {
                size: Size { cols: 20, rows: 3 },
                rows: vec![vec![]; 3],
                cursor: Cursor {
                    row: 0,
                    col: 0,
                    visible: true,
                },
                modes: Modes::default(),
                history: std::collections::VecDeque::new(),
                history_cursor: None,
                history_total: 0,
                history_fresh: true,
            },
            title: String::new(),
            directory: ServerPath(b"/tmp".to_vec()),
            process: None,
        }
    }

    fn setup(cx: &mut TestAppContext) -> (Entity<TerminalPane>, &mut VisualTestContext) {
        let result = cx.add_window_view(|window, cx| {
            let mut pane = TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            );
            pane.attach(attachment(), cx);
            pane.focus.focus(window);
            window.activate_window();
            pane
        });
        result.1.run_until_parked();
        result
    }

    fn tick(cx: &mut VisualTestContext) {
        cx.executor().advance_clock(BLINK_INTERVAL);
        cx.run_until_parked();
    }

    #[gpui::test]
    fn cursor_blinks_at_idle_and_restarts_on_input_and_cursor_movement(cx: &mut TestAppContext) {
        let (pane, cx) = setup(cx);
        assert!(pane.read_with(cx, |pane, _| pane.cursor_blink.task.is_some()));
        assert!(pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        tick(cx);
        assert!(!pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        tick(cx);
        assert!(pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        tick(cx);
        cx.simulate_keystrokes("a");
        assert!(pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        tick(cx);
        assert!(!pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        pane.update(cx, |pane, cx| {
            let grid = pane.grid.as_ref().unwrap();
            pane.apply(
                &ScreenFrame {
                    seq: 1,
                    reset: false,
                    rows: vec![],
                    cursor: Cursor {
                        col: 1,
                        ..grid.cursor
                    },
                    modes: grid.modes,
                },
                cx,
            );
        });
        cx.run_until_parked();
        assert!(pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        tick(cx);
        assert!(!pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        pane.update(cx, |pane, cx| {
            let grid = pane.grid.as_ref().unwrap();
            pane.apply(
                &ScreenFrame {
                    seq: 2,
                    reset: false,
                    rows: vec![],
                    cursor: grid.cursor,
                    modes: grid.modes,
                },
                cx,
            );
        });
        cx.run_until_parked();
        assert!(
            !pane.read_with(cx, |pane, _| pane.cursor_blink.visible),
            "unrelated output must not restart blinking"
        );
    }

    #[gpui::test]
    fn cursor_steady_mode_cancels_blink_and_reattach_restores_the_default(cx: &mut TestAppContext) {
        let (pane, cx) = setup(cx);
        tick(cx);
        pane.update(cx, |pane, cx| {
            pane.metadata(MetadataEvent::CursorBlinking(false), cx);
        });
        cx.run_until_parked();
        assert!(pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        assert!(pane.read_with(cx, |pane, _| pane.cursor_blink.task.is_none()));
        tick(cx);
        assert!(pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        pane.update(cx, |pane, cx| {
            pane.metadata(MetadataEvent::CursorBlinking(true), cx);
        });
        cx.run_until_parked();
        tick(cx);
        assert!(!pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        pane.update(cx, |pane, cx| {
            pane.metadata(MetadataEvent::CursorBlinking(false), cx);
            pane.attach(attachment(), cx);
        });
        cx.run_until_parked();
        tick(cx);
        assert!(
            !pane.read_with(cx, |pane, _| pane.cursor_blink.visible),
            "old servers fall back to blinking"
        );
    }

    #[gpui::test]
    fn cursor_blink_stops_for_inactive_windows_and_other_focus(cx: &mut TestAppContext) {
        let (pane, cx) = setup(cx);
        tick(cx);
        cx.deactivate_window();
        tick(cx);
        assert!(pane.read_with(cx, |pane, _| pane.cursor_blink.task.is_none()));
        assert!(pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        tick(cx);
        assert!(!pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        let other = cx.update(|window, cx| {
            let focus = cx.focus_handle();
            focus.focus(window);
            focus
        });
        tick(cx);
        assert!(pane.read_with(cx, |pane, _| pane.cursor_blink.task.is_none()));
        assert!(pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
        drop(other);
    }

    #[gpui::test]
    fn cursor_blink_stops_when_hidden_scrolled_or_disconnected(cx: &mut TestAppContext) {
        let (pane, cx) = setup(cx);
        for reason in 0..5 {
            pane.update(cx, |pane, cx| {
                pane.attach(attachment(), cx);
                pane.set_focused(true, cx);
                pane.native_visible = true;
            });
            cx.run_until_parked();
            tick(cx);
            assert!(!pane.read_with(cx, |pane, _| pane.cursor_blink.visible));
            pane.update(cx, |pane, cx| {
                match reason {
                    0 => pane.set_focused(false, cx),
                    1 => pane.native_visible = false,
                    2 => pane.grid.as_mut().unwrap().cursor.visible = false,
                    3 => pane.scroll.view = pane.grid.clone(),
                    _ => pane.set_state(PaneState::Disconnected, cx),
                }
                cx.notify();
            });
            cx.run_until_parked();
            tick(cx);
            assert!(
                pane.read_with(cx, |pane, _| pane.cursor_blink.task.is_none()),
                "reason {reason}"
            );
        }
    }
}
