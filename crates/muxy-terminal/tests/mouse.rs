use std::error::Error;

use muxy_terminal::{
    InputModes, Modifiers, MouseAction, MouseButton, MouseEvent, ScrollDirection, Size, Terminal,
};

fn event(action: MouseAction, button: Option<MouseButton>) -> MouseEvent {
    MouseEvent {
        action,
        button,
        column: 2,
        row: 3,
        scroll: None,
        modifiers: Modifiers::default(),
    }
}

fn terminal() -> Result<Terminal, muxy_terminal::TerminalError> {
    Terminal::new(Size { cols: 80, rows: 24 }, 1024 * 1024)
}

#[test]
fn input_modes_follow_tracking_focus_and_the_active_screen() -> Result<(), Box<dyn Error>> {
    let mut terminal = terminal()?;
    assert_eq!(terminal.input_modes()?, InputModes::default());
    terminal.feed(b"\x1b[?1000h\x1b[?1004h");
    assert_eq!(
        terminal.input_modes()?,
        InputModes {
            mouse_tracking: true,
            alternate_scroll: false,
            focus_events: true,
        }
    );
    terminal.feed(b"\x1b[?1049h");
    assert!(terminal.input_modes()?.alternate_scroll);
    terminal.feed(b"\x1b[?1007l");
    assert!(!terminal.input_modes()?.alternate_scroll);
    terminal.feed(b"\x1b[?1007h\x1b[?1049l\x1b[?1000l\x1b[?1004l");
    assert_eq!(terminal.input_modes()?, InputModes::default());
    Ok(())
}

#[test]
fn sgr_press_release_wheel_and_tracking_modes() -> Result<(), Box<dyn Error>> {
    for mode in [1000, 1002, 1003] {
        let mut terminal = terminal()?;
        terminal.feed(format!("\x1b[?{mode}h\x1b[?1006h").as_bytes());
        assert_eq!(
            terminal.encode_mouse(&event(MouseAction::Press, Some(MouseButton::Left)))?,
            b"\x1b[<0;3;4M"
        );
        let mut motion = event(MouseAction::Motion, Some(MouseButton::Left));
        motion.column = 4;
        assert_eq!(
            terminal.encode_mouse(&motion)?,
            if mode == 1000 {
                b"".as_slice()
            } else {
                b"\x1b[<32;5;4M"
            }
        );
        assert_eq!(
            terminal.encode_mouse(&event(MouseAction::Release, Some(MouseButton::Left)))?,
            b"\x1b[<0;3;4m"
        );
        let mut wheel = event(MouseAction::Scroll, None);
        for (direction, bytes) in [
            (ScrollDirection::Up, b"\x1b[<64;3;4M"),
            (ScrollDirection::Down, b"\x1b[<65;3;4M"),
        ] {
            wheel.scroll = Some(direction);
            assert_eq!(terminal.encode_mouse(&wheel)?, bytes);
        }
    }
    Ok(())
}

#[test]
fn button_motion_requires_a_button_and_any_motion_does_not() -> Result<(), Box<dyn Error>> {
    let mut terminal = terminal()?;
    terminal.feed(b"\x1b[?1002h\x1b[?1006h");
    let mut motion = event(MouseAction::Motion, None);
    assert!(terminal.encode_mouse(&motion)?.is_empty());
    terminal.encode_mouse(&event(MouseAction::Press, Some(MouseButton::Left)))?;
    motion.button = Some(MouseButton::Left);
    motion.column = 4;
    assert_eq!(terminal.encode_mouse(&motion)?, b"\x1b[<32;5;4M");
    terminal.encode_mouse(&event(MouseAction::Release, Some(MouseButton::Left)))?;
    motion.button = None;
    motion.column = 6;
    assert!(terminal.encode_mouse(&motion)?.is_empty());
    terminal.feed(b"\x1b[?1003h");
    motion.button = None;
    assert_eq!(terminal.encode_mouse(&motion)?, b"\x1b[<35;7;4M");
    Ok(())
}

#[test]
fn buttons_modifiers_and_coordinates_use_the_current_size() -> Result<(), Box<dyn Error>> {
    let mut terminal = terminal()?;
    terminal.feed(b"\x1b[?1000h\x1b[?1006h");
    for (button, code) in [
        (MouseButton::Left, 0),
        (MouseButton::Middle, 1),
        (MouseButton::Right, 2),
        (MouseButton::Back, 128),
        (MouseButton::Forward, 129),
    ] {
        let mut press = event(MouseAction::Press, Some(button));
        press.modifiers = Modifiers {
            shift: true,
            alt: true,
            ctrl: true,
        };
        assert_eq!(
            terminal.encode_mouse(&press)?,
            format!("\x1b[<{};3;4M", code + 28).as_bytes()
        );
    }
    let mut press = event(MouseAction::Press, Some(MouseButton::Left));
    press.column = u16::MAX;
    press.row = u16::MAX;
    assert_eq!(terminal.encode_mouse(&press)?, b"\x1b[<0;80;24M");
    terminal.resize(Size { cols: 10, rows: 5 })?;
    assert_eq!(terminal.encode_mouse(&press)?, b"\x1b[<0;10;5M");
    Ok(())
}

#[test]
fn alternate_scroll_is_three_cursor_presses_and_only_on_the_alternate_screen()
-> Result<(), Box<dyn Error>> {
    let mut terminal = terminal()?;
    let mut wheel = event(MouseAction::Scroll, None);
    wheel.scroll = Some(ScrollDirection::Up);
    assert!(terminal.encode_mouse(&wheel)?.is_empty());
    terminal.feed(b"\x1b[?1049h");
    assert_eq!(terminal.encode_mouse(&wheel)?, b"\x1b[A\x1b[A\x1b[A");
    wheel.scroll = Some(ScrollDirection::Down);
    assert_eq!(terminal.encode_mouse(&wheel)?, b"\x1b[B\x1b[B\x1b[B");
    terminal.feed(b"\x1b[?1h");
    assert_eq!(terminal.encode_mouse(&wheel)?, b"\x1bOB\x1bOB\x1bOB");
    wheel.scroll = Some(ScrollDirection::Up);
    assert_eq!(terminal.encode_mouse(&wheel)?, b"\x1bOA\x1bOA\x1bOA");
    terminal.feed(b"\x1b[?1007l");
    assert!(terminal.encode_mouse(&wheel)?.is_empty());
    terminal.feed(b"\x1b[?1007h\x1b[?1049l");
    assert!(terminal.encode_mouse(&wheel)?.is_empty());
    for action in [
        MouseAction::Press,
        MouseAction::Motion,
        MouseAction::Release,
    ] {
        assert!(
            terminal
                .encode_mouse(&event(action, Some(MouseButton::Left)))?
                .is_empty()
        );
    }
    Ok(())
}

#[test]
fn encoding_follows_terminal_format_changes() -> Result<(), Box<dyn Error>> {
    let mut terminal = terminal()?;
    terminal.feed(b"\x1b[?1000h");
    let press = event(MouseAction::Press, Some(MouseButton::Left));
    assert_eq!(terminal.encode_mouse(&press)?, b"\x1b[M #$");
    terminal.feed(b"\x1b[?1006h");
    assert_eq!(terminal.encode_mouse(&press)?, b"\x1b[<0;3;4M");
    terminal.feed(b"\x1b[?1006l");
    assert_eq!(terminal.encode_mouse(&press)?, b"\x1b[M #$");
    Ok(())
}
