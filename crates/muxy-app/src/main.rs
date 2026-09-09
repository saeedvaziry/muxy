mod boot;
mod model;
mod navigation;
mod picker;
mod server;
mod theme;
mod views {
    pub(crate) mod confirm;
    pub(crate) mod disconnected;
    pub(crate) mod menu;
    pub(crate) mod overlays;
    pub(crate) mod project_editor;
    pub(crate) mod project_menu;
    pub(crate) mod project_picker;
    pub(crate) mod sidebar;
    pub(crate) mod splits;
    pub(crate) mod status_bar;
    pub(crate) mod tab_strip;
    pub(crate) mod theme_picker;
    pub(crate) mod titlebar;
    pub(crate) mod terminal {
        pub(crate) mod clipboard;
        pub(crate) mod colors;
        pub(crate) mod cursor;
        pub(crate) mod element;
        pub(crate) mod find;
        pub(crate) mod input;
        pub(crate) mod pane;
        pub(crate) mod scroll;
        pub(crate) mod selection;
    }
    pub(crate) mod workspace;
}

use std::io::{self, Write};
use std::process::ExitCode;

use gpui::{
    App, AppContext, Application, Bounds, Menu, MenuItem, OsAction, SystemMenuType,
    TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowOptions, point, px, size,
};

use model::AppModel;
use views::workspace::{
    AddProject, ClosePane, CloseTab, DecreaseFontSize, EndAllSessionsAndQuit, Find, FindNext,
    FindPrevious, FocusPaneDown, FocusPaneLeft, FocusPaneRight, FocusPaneUp, HideApp, HideOthers,
    IncreaseFontSize, Minimize, NewHomeTab, NewTab, NextProject, NextTab, OpenConfiguration,
    PreviousProject, PreviousTab, Quit, SelectTab, ShowAll, SplitDown, SplitRight,
    ToggleFullScreen, ToggleSidebar, ToggleThemePicker, ToggleZoomPane, Zoom, bind_keys,
};

fn main() -> ExitCode {
    let boot = match boot::Boot::load() {
        Ok(boot) => boot,
        Err(error) => {
            let _ = writeln!(io::stderr(), "muxy-app: {error}");
            return ExitCode::FAILURE;
        }
    };
    Application::new()
        .with_assets(muxy_ui::assets::Assets)
        .run(move |cx: &mut App| {
            bind_keys(&boot.settings.keymap, cx);
            let config_path = boot.state_path.with_file_name("ghostty.conf");
            cx.on_action(move |_: &OpenConfiguration, _| {
                if let Err(error) = std::process::Command::new("/usr/bin/open")
                    .args(["-a", "TextEdit"])
                    .arg(&config_path)
                    .status()
                    .and_then(|status| {
                        if status.success() {
                            Ok(())
                        } else {
                            Err(io::Error::other(format!("TextEdit exited with {status}")))
                        }
                    })
                {
                    let _ = writeln!(
                        io::stderr(),
                        "muxy-app: could not open configuration: {error}"
                    );
                }
            });
            cx.on_action(|_: &HideApp, cx| cx.hide());
            cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
            cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
            cx.set_menus(menus());
            let bounds = restored_bounds(&boot, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(640.0), px(400.0))),
                window_background: WindowBackgroundAppearance::Transparent,
                titlebar: Some(TitlebarOptions {
                    title: Some("Muxy Alpha".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(9.0), px(9.0))),
                }),
                ..WindowOptions::default()
            };
            if let Err(error) = cx.open_window(options, |window, cx| {
                let model = cx.new(|cx| AppModel::new(boot, window, cx));
                let weak = model.downgrade();
                window.on_window_should_close(cx, move |_, cx| {
                    if weak.update(cx, AppModel::quit).is_err() {
                        cx.quit();
                    }
                    false
                });
                model
            }) {
                let _ = writeln!(io::stderr(), "muxy-app: could not open window: {error}");
                cx.quit();
            }
            cx.activate(true);
        });
    ExitCode::SUCCESS
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Saved coordinates are checked against the GPUI pixel range"
)]
fn restored_bounds(boot: &boot::Boot, cx: &App) -> Bounds<gpui::Pixels> {
    if let Some(bounds) = boot.state.window().bounds
        && [bounds.x, bounds.y, bounds.width, bounds.height]
            .into_iter()
            .all(|value| value.abs() <= f64::from(f32::MAX))
    {
        return Bounds::new(
            point(px(bounds.x as f32), px(bounds.y as f32)),
            size(px(bounds.width as f32), px(bounds.height as f32)),
        );
    }
    let [width, height] = boot.settings.window.default_size;
    Bounds::centered(None, size(px(width), px(height)), cx)
}

fn menus() -> Vec<Menu> {
    use muxy_ui::text_input;
    let mut window_items = vec![
        MenuItem::action("Minimize", Minimize),
        MenuItem::action("Zoom", Zoom),
        MenuItem::separator(),
        MenuItem::action("Split Right", SplitRight),
        MenuItem::action("Split Down", SplitDown),
        MenuItem::action("Focus Pane Left", FocusPaneLeft),
        MenuItem::action("Focus Pane Right", FocusPaneRight),
        MenuItem::action("Focus Pane Up", FocusPaneUp),
        MenuItem::action("Focus Pane Down", FocusPaneDown),
        MenuItem::action("Toggle Pane Zoom", ToggleZoomPane),
        MenuItem::action("Close Pane", ClosePane),
        MenuItem::action("Close Tab", CloseTab),
        MenuItem::separator(),
        MenuItem::action("Next Tab", NextTab),
        MenuItem::action("Previous Tab", PreviousTab),
        MenuItem::separator(),
        MenuItem::action("Previous Project", PreviousProject),
        MenuItem::action("Next Project", NextProject),
        MenuItem::separator(),
    ];
    window_items.extend(
        (0..9).map(|index| MenuItem::action(format!("Tab {}", index + 1), SelectTab { index })),
    );
    vec![
        Menu {
            name: "Muxy Alpha".into(),
            items: vec![
                MenuItem::action("Open Configuration…", OpenConfiguration),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Muxy Alpha", HideApp),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Muxy Alpha", Quit),
                MenuItem::action("End All Sessions and Quit", EndAllSessionsAndQuit),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Cut", text_input::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", text_input::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", text_input::Paste, OsAction::Paste),
                MenuItem::os_action("Select All", text_input::SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find…", Find),
                MenuItem::action("Find Next", FindNext),
                MenuItem::action("Find Previous", FindPrevious),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Tab", NewTab),
                MenuItem::action("New Home Tab", NewHomeTab),
                MenuItem::action("Open Project…", AddProject),
                MenuItem::separator(),
                MenuItem::action("Close Tab", CloseTab),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Toggle Full Screen", ToggleFullScreen),
                MenuItem::separator(),
                MenuItem::action("Theme Picker", ToggleThemePicker),
                MenuItem::separator(),
                MenuItem::action("Increase Font Size", IncreaseFontSize),
                MenuItem::action("Decrease Font Size", DecreaseFontSize),
            ],
        },
        Menu {
            name: "Window".into(),
            items: window_items,
        },
    ]
}
