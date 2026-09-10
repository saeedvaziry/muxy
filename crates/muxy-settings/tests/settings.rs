#![allow(
    clippy::float_cmp,
    reason = "Configuration values and integer zoom steps must round-trip exactly"
)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use muxy_settings::{Action, CellHeight, KeyChord, Keymap, Settings, TerminalSettings};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "muxy-settings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn write(&self, name: &str, source: &str) -> Result<PathBuf> {
        let path = self.0.join(name);
        fs::write(&path, source)?;
        Ok(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn missing_file_writes_editable_defaults_and_empty_file_loads_defaults() -> Result {
    let fixture = Fixture::new()?;
    let path = fixture.0.join("nested/settings.toml");
    assert_eq!(Settings::load(&path)?, Settings::default());
    let source = fs::read_to_string(&path)?;
    assert!(source.contains("new_tab = \"cmd-t\""));
    let stored: toml::Value = toml::from_str(&source)?;
    assert!(stored.get("terminal").is_none());
    assert_eq!(
        stored["keymap"]["terminal.close_find"].as_str(),
        Some("escape")
    );
    assert_eq!(Settings::load(&path)?, Settings::default());
    fs::write(&path, "")?;
    assert_eq!(Settings::load(&path)?, Settings::default());
    assert_eq!(fs::read_to_string(path)?, "");
    Ok(())
}

#[test]
fn partial_settings_keep_defaults_and_appearance_saves_preserve_bindings() -> Result {
    let fixture = Fixture::new()?;
    let path = fixture.write("settings.toml", "[appearance]\ndark_theme = 'Dracula'\n[window]\ndefault_size = [1000, 700]\n[keymap]\nnew_tab = 'cmd-n'\n")?;
    let settings = Settings::load(&path)?;
    assert_eq!(settings.window.default_size, [1000.0, 700.0]);
    assert_eq!(settings.appearance.light_theme, "Muxy Light");
    assert_eq!(
        settings.keymap.chord(Action::NewTab).map(KeyChord::as_str),
        Some("cmd-n")
    );
    assert_eq!(
        settings
            .keymap
            .chord(Action::CloseTab)
            .map(KeyChord::as_str),
        Some("cmd-shift-w")
    );
    let mut appearance = settings.appearance;
    appearance.sidebar_expanded = true;
    appearance.save(&path)?;
    let reloaded = Settings::load(&path)?;
    assert_eq!(reloaded.appearance, appearance);
    assert_eq!(reloaded.keymap, settings.keymap);
    assert_eq!(reloaded.window, settings.window);
    Ok(())
}

#[test]
fn close_confirmation_defaults_to_enabled_and_round_trips_without_replacing_other_settings()
-> Result {
    let fixture = Fixture::new()?;
    let path = fixture.write("settings.toml", "[window]\ndefault_size = [1000, 700]\n[keymap]\nnew_tab = 'cmd-n'\n[appearance]\ndark_theme = 'Dracula'\n")?;
    let mut settings = Settings::load(&path)?;
    assert!(settings.window.confirm_running_process);
    settings.set_confirm_running_process(false, &path)?;
    assert!(!settings.window.confirm_running_process);
    let mut reloaded = Settings::load(&path)?;
    assert_eq!(settings, reloaded);
    reloaded.appearance.sidebar_expanded = true;
    reloaded.appearance.save(&path)?;
    assert_eq!(Settings::load(&path)?, reloaded);
    reloaded.set_confirm_running_process(true, &path)?;
    assert!(Settings::load(&path)?.window.confirm_running_process);
    assert_eq!(reloaded.window.default_size, [1000.0, 700.0]);
    assert_eq!(
        reloaded.keymap.chord(Action::NewTab).map(KeyChord::as_str),
        Some("cmd-n")
    );
    assert_eq!(reloaded.appearance.dark_theme, "Dracula");
    Ok(())
}

#[test]
fn saving_close_confirmation_preserves_invalid_files_and_in_memory_preferences() -> Result {
    let fixture = Fixture::new()?;
    for source in ["not valid TOML", "window = 'invalid'"] {
        let path = fixture.write("settings.toml", source)?;
        let mut settings = Settings::default();
        assert!(settings.set_confirm_running_process(false, &path).is_err());
        assert!(settings.window.confirm_running_process);
        assert_eq!(fs::read_to_string(&path)?, source);
    }
    let blocked = fixture.write("blocked", "keep this file")?;
    let mut settings = Settings::default();
    assert!(
        settings
            .set_confirm_running_process(false, &blocked.join("settings.toml"))
            .is_err()
    );
    assert!(settings.window.confirm_running_process);
    assert_eq!(fs::read_to_string(blocked)?, "keep this file");
    Ok(())
}

#[test]
fn every_default_binding_round_trips_and_resolves_both_directions() -> Result {
    let keymap = Keymap::default();
    for action in Action::ALL {
        let chord: KeyChord = keymap
            .chord(action)
            .ok_or("default is unbound")?
            .to_string()
            .parse()?;
        assert_eq!(Some(&chord), keymap.chord(action));
        assert_eq!(keymap.action(&chord), Some(action));
    }
    assert_eq!(keymap.action(&"ctrl-alt-f24".parse()?), None);
    let settings: Settings = toml::from_str(&toml::to_string(&Settings::default())?)?;
    assert_eq!(settings, Settings::default());
    Ok(())
}

#[test]
fn chords_support_all_modifiers_named_keys_and_printable_characters() -> Result {
    for modifier in ["cmd", "ctrl", "alt", "shift", "fn"] {
        for name in [
            "enter",
            "escape",
            "space",
            "tab",
            "backspace",
            "delete",
            "insert",
            "home",
            "end",
            "pageup",
            "pagedown",
            "up",
            "down",
            "left",
            "right",
            "]",
            "[",
            "-",
            "+",
            "=",
            "a",
            "7",
            "é",
        ] {
            let value = format!("{modifier}-{name}");
            assert_eq!(value.parse::<KeyChord>()?.as_str(), value);
        }
        for number in 1..=24 {
            let value = format!("{modifier}-f{number}");
            assert_eq!(value.parse::<KeyChord>()?.as_str(), value);
        }
    }
    for (source, canonical) in [
        ("alt-CMD-ctrl-shift-fn-left", "cmd-ctrl-alt-shift-fn-left"),
        ("cmd-T", "cmd-shift-t"),
        ("cmd-plus", "cmd-+"),
        ("cmd-minus", "cmd--"),
        ("return", "enter"),
        ("esc", "escape"),
        ("CMD-ENTER", "cmd-enter"),
    ] {
        assert_eq!(source.parse::<KeyChord>()?.as_str(), canonical);
    }
    Ok(())
}

#[test]
fn invalid_chords_and_keymap_errors_name_the_problem() -> Result {
    for value in [
        "",
        "cmd",
        "cmd-",
        "cmd-cmd-t",
        "cmd-x-y",
        "ctrl--x",
        "cmd t",
        "cmd-unknown",
        "f0",
        "f25",
        "f01",
        "\n",
        "cmd- ",
        " cmd-t",
        "cmd---",
    ] {
        assert!(value.parse::<KeyChord>().is_err(), "{value:?}");
    }
    for (source, names) in [
        (
            "[keymap]\nnew_tab = 'cmd-nope'",
            vec!["new_tab", "cmd-nope"],
        ),
        (
            "[keymap]\nnew_tabb = 'cmd-t'",
            vec!["new_tabb", "unknown action"],
        ),
        (
            "[keymap]\nnew_tab = 'cmd-c'",
            vec!["new_tab", "copy", "cmd-c"],
        ),
        (
            "[keymap]\nnew_tab = 'shift-cmd-x'\nclose_tab = 'cmd-shift-x'",
            vec!["new_tab", "close_tab"],
        ),
    ] {
        let error = toml::from_str::<Settings>(source)
            .err()
            .ok_or("invalid settings accepted")?
            .to_string();
        for name in names {
            assert!(error.contains(name), "{error}");
        }
    }
    let settings: Settings = toml::from_str("[keymap]\nnew_tab = 'cmd-w'\nclose_tab = 'cmd-t'")?;
    assert_eq!(
        settings.keymap.action(&"cmd-w".parse()?),
        Some(Action::NewTab)
    );
    Ok(())
}

#[test]
fn invalid_settings_are_reported_without_overwriting_the_file() -> Result {
    let fixture = Fixture::new()?;
    for source in [
        "[broken",
        "[window]\ndefault_size = [0, 800]",
        "[window]\ndefault_size = [1200, inf]",
        "[window]\ndefault_size = [nan, 800]",
        "[terminal]\nfont_size = 16",
    ] {
        let path = fixture.write("settings.toml", source)?;
        assert!(Settings::load(&path).is_err());
        assert_eq!(fs::read_to_string(path)?, source);
    }
    Ok(())
}

#[test]
fn ghostty_defaults_seed_once_and_existing_config_is_preserved() -> Result {
    let fixture = Fixture::new()?;
    let path = fixture.0.join("ghostty.conf");
    assert_eq!(
        TerminalSettings::load_with_seed(&path, None)?,
        TerminalSettings::default()
    );
    let source = "font-family = Monaco\nfont-size = 16\n";
    let seed = fixture.write("seed", source)?;
    assert_eq!(
        TerminalSettings::load_with_seed(&path, Some(&seed))?,
        TerminalSettings::default()
    );
    fs::remove_file(&path)?;
    let settings = TerminalSettings::load_with_seed(&path, Some(&seed))?;
    assert_eq!(settings.font_size, 16.0);
    assert_eq!(settings.font_families, ["Monaco"]);
    assert_eq!(fs::read_to_string(path)?, source);
    Ok(())
}

#[test]
fn ghostty_values_support_comments_quotes_resets_fallbacks_and_height_adjustments() -> Result {
    let fixture = Fixture::new()?;
    let source = "# terminal settings\nfont-size = 16.5\nfont-family = Discarded\nfont-family = \"\"\nfont-family = \"Menlo\"\nfont-family = PingFang SC\nadjust-cell-height = 20%\nbackground = 112233\n";
    let path = fixture.write("ghostty.conf", source)?;
    let settings = TerminalSettings::load(&path)?;
    assert_eq!(settings.font_families, ["Menlo", "PingFang SC"]);
    assert_eq!(settings.font_size, 16.5);
    assert_eq!(settings.cell_height, CellHeight::Percent(20.0));
    assert_eq!(settings.cell_height.apply(20.0, 2.0), 24.0);
    assert_eq!(CellHeight::Pixels(4).apply(20.0, 2.0), 22.0);
    assert_eq!(CellHeight::Pixels(-100).apply(20.0, 2.0), 1.0);
    assert_eq!(fs::read_to_string(&path)?, source);
    fs::write(
        &path,
        "font-size = 20\nfont-size =\nadjust-cell-height = 10%\nadjust-cell-height =\nfont-family =\n",
    )?;
    assert_eq!(TerminalSettings::load(&path)?, TerminalSettings::default());
    Ok(())
}

#[test]
fn ghostty_includes_apply_last_and_detect_cycles() -> Result {
    let fixture = Fixture::new()?;
    fixture.write("fonts", "font-size = 17\n")?;
    let path = fixture.write(
        "ghostty.conf",
        "config-file = ?missing\nconfig-file = fonts\nfont-size = 15\n",
    )?;
    assert_eq!(TerminalSettings::load(&path)?.font_size, 17.0);
    fixture.write("fonts", "config-file = ghostty.conf\n")?;
    let error = TerminalSettings::load(&path)
        .err()
        .ok_or("cycle accepted")?
        .to_string();
    assert!(error.contains("cycle"), "{error}");
    Ok(())
}

#[test]
fn ghostty_ignores_bare_boolean_settings_outside_font_scope() -> Result {
    let fixture = Fixture::new()?;
    let path = fixture.write("ghostty.conf", "font-size = 16\nfont-thicken\n")?;
    assert_eq!(TerminalSettings::load(&path)?.font_size, 16.0);
    Ok(())
}

#[test]
fn ghostty_accepts_fractional_height_percentages() -> Result {
    let fixture = Fixture::new()?;
    let path = fixture.write("ghostty.conf", "adjust-cell-height = 12.5%\n")?;
    assert_eq!(
        TerminalSettings::load(&path)?.cell_height.apply(16.0, 1.0),
        18.0
    );
    Ok(())
}

#[test]
fn ghostty_loads_quoted_optional_paths() -> Result {
    let fixture = Fixture::new()?;
    fixture.write("font settings", "font-size = 17\n")?;
    let path = fixture.write(
        "ghostty.conf",
        "config-file = ?\"missing\"\nconfig-file = ?\"font settings\"\n",
    )?;
    assert_eq!(TerminalSettings::load(&path)?.font_size, 17.0);
    Ok(())
}

#[test]
fn ghostty_loads_nested_includes_in_discovery_order() -> Result {
    let fixture = Fixture::new()?;
    fixture.write("a", "config-file = c\n")?;
    fixture.write("b", "font-size = 18\n")?;
    fixture.write("c", "font-size = 17\n")?;
    let path = fixture.write("ghostty.conf", "config-file = a\nconfig-file = b\n")?;
    assert_eq!(TerminalSettings::load(&path)?.font_size, 17.0);
    Ok(())
}

#[test]
fn ghostty_errors_name_the_key_and_line_and_zoom_is_only_in_memory() -> Result {
    let fixture = Fixture::new()?;
    for (key, value) in [
        ("font-size", "nan"),
        ("font-size", "0"),
        ("font-size", "huge"),
        ("font-family", "\"Menlo"),
        ("adjust-cell-height", "-100%"),
        ("adjust-cell-height", "tall"),
    ] {
        let path = fixture.write("ghostty.conf", &format!("# test\n{key} = {value}"))?;
        let error = TerminalSettings::load(&path)
            .err()
            .ok_or("bad Ghostty setting accepted")?
            .to_string();
        assert!(error.contains(key) && error.contains(":2"), "{error}");
    }
    let path = fixture.write("ghostty.conf", "font-size = 16\n")?;
    let mut active = TerminalSettings::load(&path)?;
    let other = active.clone();
    active.zoom(1.0);
    assert_eq!(active.font_size, 17.0);
    assert_eq!(other.font_size, 16.0);
    active.zoom(-1.0);
    assert_eq!(active.font_size, 16.0);
    active.zoom(-1000.0);
    assert_eq!(active.font_size, 1.0);
    active.zoom(1000.0);
    assert_eq!(active.font_size, 256.0);
    assert_eq!(TerminalSettings::load(&path)?.font_size, 16.0);
    Ok(())
}

#[test]
fn project_search_location_round_trips_and_open_project_uses_command_o() -> Result {
    let fixture = Fixture::new()?;
    let path = fixture.write("settings.toml", "[keymap]\nnew_tab = 'cmd-n'\n")?;
    let mut settings = Settings::load(&path)?;
    assert_eq!(
        settings
            .keymap
            .chord(Action::AddProject)
            .map(KeyChord::as_str),
        Some("cmd-o")
    );
    assert!(settings.projects.search_root.is_none());
    settings.set_project_search_root(fixture.0.clone(), &path)?;
    assert_eq!(Settings::load(&path)?, settings);
    assert_eq!(
        settings.keymap.chord(Action::NewTab).map(KeyChord::as_str),
        Some("cmd-n")
    );
    Ok(())
}

#[test]
fn existing_bindings_take_precedence_over_new_project_defaults_and_round_trip() -> Result {
    let fixture = Fixture::new()?;
    for (project_action, chord) in [
        (Action::AddProject, "cmd-o"),
        (Action::PreviousProject, "cmd-alt-["),
        (Action::NextProject, "cmd-alt-]"),
    ] {
        for existing_action in [Action::NewTab, Action::Copy] {
            let source = format!("[keymap]\n{} = '{chord}'\n", existing_action.name());
            let path = fixture.write("settings.toml", &source)?;
            let mut settings = Settings::load(&path)?;
            let chord: KeyChord = chord.parse()?;
            assert_eq!(settings.keymap.chord(existing_action), Some(&chord));
            assert_eq!(settings.keymap.action(&chord), Some(existing_action));
            assert_eq!(settings.keymap.chord(project_action), None);
            assert_eq!(fs::read_to_string(&path)?, source);

            settings.set_project_search_root(fixture.0.clone(), &path)?;
            assert_eq!(Settings::load(&path)?, settings);
            fs::write(&path, toml::to_string(&settings)?)?;
            assert_eq!(Settings::load(&path)?, settings);
        }
    }
    Ok(())
}

#[test]
fn explicit_project_bindings_still_require_unique_chords() -> Result {
    for (project_action, chord) in [
        (Action::AddProject, "cmd-o"),
        (Action::PreviousProject, "cmd-alt-["),
        (Action::NextProject, "cmd-alt-]"),
    ] {
        let source = format!(
            "[keymap]\nnew_tab = '{chord}'\n{} = '{chord}'\n",
            project_action.name()
        );
        let error = toml::from_str::<Settings>(&source)
            .expect_err("explicit collision")
            .to_string();
        assert!(error.contains("new_tab"), "{error}");
        assert!(error.contains(project_action.name()), "{error}");

        let source = format!(
            "[keymap]\nnew_tab = '{chord}'\n{} = 'ctrl-alt-f24'\n",
            project_action.name()
        );
        let settings: Settings = toml::from_str(&source)?;
        assert_eq!(
            settings.keymap.action(&chord.parse()?),
            Some(Action::NewTab)
        );
        assert_eq!(
            settings.keymap.action(&"ctrl-alt-f24".parse()?),
            Some(project_action)
        );
    }
    Ok(())
}

#[test]
fn pane_directory_and_all_split_shortcuts_load_with_defaults_and_overrides() -> Result {
    let settings: Settings = toml::from_str("")?;
    assert_eq!(
        settings.panes.new_pane_directory,
        muxy_settings::NewPaneDirectory::Project
    );
    for (action, chord) in [
        (Action::SplitRight, "cmd-d"),
        (Action::SplitDown, "cmd-shift-d"),
        (Action::FocusPaneLeft, "cmd-alt-left"),
        (Action::FocusPaneRight, "cmd-alt-right"),
        (Action::FocusPaneUp, "cmd-alt-up"),
        (Action::FocusPaneDown, "cmd-alt-down"),
        (Action::ToggleZoomPane, "cmd-shift-enter"),
        (Action::ClosePane, "cmd-w"),
        (Action::CloseTab, "cmd-shift-w"),
    ] {
        assert_eq!(
            settings.keymap.chord(action).map(KeyChord::as_str),
            Some(chord)
        );
    }
    let customized: Settings = toml::from_str(
        "[panes]\nnew_pane_directory = 'current'\n[keymap]\nsplit_right = 'ctrl-alt-d'",
    )?;
    assert_eq!(
        customized.panes.new_pane_directory,
        muxy_settings::NewPaneDirectory::Current
    );
    assert_eq!(
        customized.keymap.action(&"ctrl-alt-d".parse()?),
        Some(Action::SplitRight)
    );
    assert_eq!(customized.keymap.action(&"cmd-d".parse()?), None);
    assert!(toml::from_str::<Settings>("[panes]\nnew_pane_directory = 'invalid'").is_err());
    assert_eq!(
        toml::from_str::<Settings>(&toml::to_string(&customized)?)?,
        customized
    );
    Ok(())
}

#[test]
fn new_pane_defaults_preserve_explicit_existing_shortcuts_without_collisions() -> Result {
    for (action, chord, displaced) in [
        (Action::CloseTab, "cmd-w", Action::ClosePane),
        (Action::NewTab, "cmd-d", Action::SplitRight),
        (Action::NextTab, "cmd-shift-d", Action::SplitDown),
        (Action::PreviousTab, "cmd-alt-left", Action::FocusPaneLeft),
        (Action::Copy, "cmd-shift-enter", Action::ToggleZoomPane),
        (Action::Paste, "cmd-shift-w", Action::CloseTab),
    ] {
        let settings: Settings =
            toml::from_str(&format!("[keymap]\n{} = '{chord}'", action.name()))?;
        assert_eq!(settings.keymap.action(&chord.parse()?), Some(action));
        assert!(settings.keymap.chord(displaced).is_none());
    }
    assert!(
        toml::from_str::<Settings>("[keymap]\nclose_tab = 'cmd-w'\nclose_pane = 'cmd-w'").is_err()
    );
    Ok(())
}

#[test]
fn every_module_shortcut_is_configurable_with_contexts_and_aliases() -> Result {
    use muxy_core::shortcuts::{ALL, ShortcutSettings};
    let defaults = Keymap::default();
    let mut names = std::collections::BTreeSet::new();
    for shortcut in ALL {
        assert!(names.insert(shortcut.id), "duplicate shortcut ID");
        assert_eq!(shortcut.keys.len(), shortcut.key_contexts.len());
        for key in shortcut.keys {
            let _: KeyChord = key.parse()?;
        }
        let settings = toml::from_str::<Settings>(&format!(
            "[keymap]\n\"{}\" = \"ctrl-alt-f24\"",
            shortcut.id
        ))?;
        for context in shortcut.contexts {
            assert_eq!(
                settings.keymap.keys(shortcut.id, *context),
                ["ctrl-alt-f24"]
            );
            let expected: Vec<_> = shortcut
                .keys
                .iter()
                .zip(shortcut.key_contexts)
                .filter(|(_, scopes)| scopes.contains(context))
                .map(|(key, _)| key.parse::<KeyChord>().map(|chord| chord.to_string()))
                .collect::<std::result::Result<_, _>>()?;
            assert_eq!(defaults.keys(shortcut.id, *context), expected);
        }
    }
    Ok(())
}

#[test]
fn aliases_yield_to_explicit_bindings_and_conflicts_are_scoped() -> Result {
    use muxy_core::shortcuts::ShortcutSettings;
    let settings = toml::from_str::<Settings>(
        "[keymap]\nnew_tab = 'ctrl-tab'\n'popover.dismiss' = 'ctrl-k'\n'menu.dismiss_menu' = 'ctrl-k'",
    )?;
    assert_eq!(
        settings.keymap.keys("next_tab", Some("WorkspaceTabs")),
        ["cmd-]"]
    );
    assert_eq!(
        settings
            .keymap
            .keys("popover.dismiss", Some("CommandPopover")),
        ["ctrl-k"]
    );
    assert!(
        toml::from_str::<Settings>(
            "[keymap]\n'popover.dismiss' = 'ctrl-k'\n'popover.confirm' = 'ctrl-k'"
        )
        .is_err()
    );
    assert!(
        toml::from_str::<Settings>("[keymap]\nquit = 'cmd-k'\n'popover.confirm' = 'cmd-k'")
            .is_err()
    );
    let remapped = toml::from_str::<Settings>("[keymap]\n'popover.secondary_confirm' = 'ctrl-k'")?;
    assert_eq!(
        remapped
            .keymap
            .keys("popover.secondary_confirm", Some("CommandPopover")),
        ["ctrl-k"]
    );
    Ok(())
}

#[test]
fn alias_overrides_only_displace_matching_contexts() -> Result {
    use muxy_core::shortcuts::ShortcutSettings;
    let settings: Settings = toml::from_str(
        r#"[keymap]
"text_input.submit" = "cmd-up"
"text_input.cancel" = "cmd-down"
"#,
    )?;
    assert!(
        settings
            .keymap
            .keys("text_input.document_start", Some("MultilineInput"))
            .contains(&"cmd-up".into())
    );
    assert!(
        settings
            .keymap
            .keys("text_input.document_end", Some("MultilineInput"))
            .contains(&"cmd-down".into())
    );
    assert_eq!(
        settings.keymap.keys("text_input.submit", Some("TextInput")),
        ["cmd-up"]
    );
    Ok(())
}

#[test]
fn clipboard_and_opener_preferences_load_without_losing_unavailable_ids() -> Result {
    let defaults: Settings = toml::from_str("")?;
    assert!(!defaults.clipboard.copy_on_select);
    assert_eq!(defaults.openers.file, "system.editor");
    assert_eq!(defaults.openers.url, "system.browser");
    let settings: Settings = toml::from_str(
        "[clipboard]\ncopy_on_select = true\n[openers]\nfile = 'extension:editor'\nurl = 'system.browser'\nproject_target = 'com.apple.finder'\n",
    )?;
    assert!(settings.clipboard.copy_on_select);
    assert_eq!(settings.openers.file, "extension:editor");
    assert_eq!(
        settings.openers.project_target.as_deref(),
        Some("com.apple.finder")
    );
    assert_eq!(
        toml::from_str::<Settings>(&toml::to_string(&settings)?)?,
        settings
    );
    assert!(toml::from_str::<Settings>("[clipboard]\ncopy_on_select = 'yes'").is_err());
    Ok(())
}
