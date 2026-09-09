use super::*;

pub(super) fn content(modal: &SettingsModal, cx: &mut Context<SettingsModal>) -> Vec<AnyElement> {
    let category = Category::Appearance;
    let style = modal.style();
    let project_focused = settings::string_value(APP_LAYOUT, "projectFocused") == "projectFocused";
    let mut sections = Vec::new();

    let localization = settings::string_value(LOCALIZATION, "");
    let language_footer = if localization.is_empty() {
        "English is built in. Additional languages can be provided by enabled extensions."
    } else {
        "The selected language extension is unavailable, so Muxy is temporarily using English."
    };
    sections.extend(visible(
        modal,
        category,
        "Language",
        Some(language_footer),
        true,
        vec![
            picker_row(
                modal,
                "App Language",
                LOCALIZATION,
                "",
                appended_stored(
                    vec![Choice::new("", "English")],
                    &localization,
                    unavailable_label(&localization, "Unavailable Language"),
                ),
                cx,
            ),
            controls::row(
                style,
                "More Languages",
                controls::button(
                    style,
                    "browse-languages",
                    "Browse Language Extensions…",
                    false,
                    |_, _, _| {},
                ),
            ),
        ],
    ));

    sections.extend(visible(
        modal,
        category,
        "Layout",
        None,
        true,
        vec![segmented_row(
            modal,
            "App Layout",
            APP_LAYOUT,
            "projectFocused",
            vec![
                Choice::new("projectFocused", "Project Focused"),
                Choice::new("tabFocused", "Tab Focused"),
                Choice::new("agentsFocused", "Agents Focused"),
            ],
            cx,
        )],
    ));

    sections.extend(visible(
        modal,
        category,
        "Appearance",
        Some(
            "Transparency shows the desktop through terminal panes, the top bar, and the status bar. Vibrancy controls the native macOS material intensity and is required for the effect.",
        ),
        true,
        vec![appearance_sliders(modal, cx)],
    ));

    let mut sidebar = vec![
        vibrancy_row(style, cx),
        toggle_row(style, "Show Home", "muxy.showHomeProject", true, cx),
        toggle_row(style, "Show Tips", "muxy.tips.visible", true, cx),
        toggle_row(
            style,
            "Auto-expand worktrees on project switch",
            "muxy.general.autoExpandWorktreesOnProjectSwitch",
            false,
            cx,
        ),
    ];
    if !modal.extension_options().sidebars.is_empty() {
        let selected = settings::string_value("muxy.activeSidebar", "");
        let choices = modal.extension_options().sidebars.iter().cloned().fold(
            vec![Choice::new("", "Built-in")],
            |mut choices, item| {
                choices.push(Choice::new(item.0, item.1));
                choices
            },
        );
        sidebar.insert(
            0,
            picker_row(
                modal,
                "Active Sidebar",
                "muxy.activeSidebar",
                "",
                appended_stored(
                    choices,
                    &selected,
                    unavailable_label(&selected, "Unavailable Extension Sidebar"),
                ),
                cx,
            ),
        );
    }
    if project_focused {
        sidebar.push(toggle_row(
            style,
            "Always Show Project Search",
            "muxy.showProjectSearch",
            false,
            cx,
        ));
        sidebar.push(segmented_row(
            modal,
            "Collapsed Style",
            "muxy.sidebarCollapsedStyle",
            "icons",
            vec![
                Choice::new("hidden", "Hidden"),
                Choice::new("icons", "Icons"),
            ],
            cx,
        ));
        sidebar.push(segmented_row(
            modal,
            "Expanded Style",
            "muxy.sidebarExpandedStyle",
            "wide",
            vec![Choice::new("icons", "Icons"), Choice::new("wide", "Wide")],
            cx,
        ));
    } else {
        sidebar.push(toggle_row(
            style,
            "Nest worktrees inside projects",
            "muxy.worktrees.groupWorktrees",
            false,
            cx,
        ));
    }
    sections.extend(visible(modal, category, "Sidebar", None, true, sidebar));

    sections.extend(visible(
        modal,
        category,
        "Worktrees",
        None,
        true,
        vec![
            toggle_row(
                style,
                "Show unread notification indicator on worktrees",
                "muxy.worktrees.showUnreadIndicator",
                true,
                cx,
            ),
            toggle_row(
                style,
                "Order worktrees by most-recently-used",
                "muxy.worktrees.orderByMRU",
                true,
                cx,
            ),
        ],
    ));

    sections.extend(visible(
        modal,
        category,
        "Theme",
        None,
        true,
        vec![
            theme_row(modal, "Light Theme", "muxy.theme.light", cx),
            theme_row(modal, "Dark Theme", "muxy.theme.dark", cx),
        ],
    ));

    sections.extend(visible(
        modal,
        category,
        "Interface",
        None,
        false,
        vec![
            size_row(modal, cx),
            tab_width_row(modal, cx),
            toggle_row(
                style,
                "Show Top Bar Actions",
                "muxy.showTopBarActions",
                true,
                cx,
            ),
            toggle_row(style, "Show Status Bar", "muxy.showStatusBar", true, cx),
            toggle_row(
                style,
                "Show Resource Usage in Status Bar",
                "muxy.showResourceUsageInStatusBar",
                true,
                cx,
            ),
        ],
    ));

    sections
}

fn vibrancy_row(style: Style, cx: &mut Context<SettingsModal>) -> AnyElement {
    let vibrant = settings::string_value(BACKGROUND_STYLE, "vibrant") == "vibrant";
    controls::row(
        style,
        "Vibrancy",
        controls::toggle(
            style,
            BACKGROUND_STYLE,
            vibrant,
            cx.listener(move |modal: &mut SettingsModal, _, _, cx| {
                let vibrant = settings::string_value(BACKGROUND_STYLE, "vibrant") == "vibrant";
                let next = if vibrant { "solid" } else { "vibrant" };
                modal.write(BACKGROUND_STYLE, Value::String(next.to_owned()), cx);
            }),
        ),
    )
}

fn theme_row(
    modal: &SettingsModal,
    label: &str,
    key: &'static str,
    cx: &mut Context<SettingsModal>,
) -> AnyElement {
    let style = modal.style();
    let metrics = style.metrics;
    let name = settings::string_value(key, "");
    let name = if name.is_empty() {
        "Default".to_owned()
    } else {
        name
    };

    let button = controls::picker_trigger(
        style,
        key,
        &name,
        modal.theme_browser(key).is_some(),
        cx.listener(move |modal: &mut SettingsModal, _, _, cx| {
            modal.toggle_theme_browser(key, cx);
        }),
    );

    let mut wrapper = div().relative().flex().flex_col().flex_none().child(button);
    if let Some(browser) = modal.theme_browser(key) {
        wrapper = wrapper.child(
            div()
                .absolute()
                .top(metrics.control_medium() + metrics.spacing1())
                .right_0()
                .w(px(0.0))
                .h(px(0.0))
                .child(
                    deferred(
                        anchored()
                            .anchor(Corner::TopRight)
                            .snap_to_window_with_margin(px(8.0))
                            .child(browser.clone()),
                    )
                    .with_priority(1),
                ),
        );
    }

    controls::row(style, label, wrapper.into_any_element())
}

fn size_row(modal: &SettingsModal, cx: &mut Context<SettingsModal>) -> AnyElement {
    let style = modal.style();
    let selected = settings::string_value("muxy.ui.scale", "regular");
    controls::row(
        style,
        "Size",
        controls::segmented(
            style,
            "muxy.ui.scale",
            vec![
                Choice::new("regular", "Default"),
                Choice::new("large", "Large"),
                Choice::new("extraLarge", "Extra Large"),
                Choice::new("huge", "Huge"),
            ],
            &selected,
            cx.listener(|modal: &mut SettingsModal, value: &SharedString, _, cx| {
                modal.set_scale(ScalePreset::parse(value), cx);
            }),
        ),
    )
}

fn appearance_sliders(modal: &SettingsModal, cx: &mut Context<SettingsModal>) -> AnyElement {
    let style = modal.style();
    let metrics = style.metrics;
    let transparency = modal.slider_value(
        TRANSPARENCY_SLIDER,
        settings::i64_value(TRANSPARENCY, 0) as f32,
    );
    let blur = modal.slider_value(BLUR_SLIDER, settings::i64_value(BLUR, 70) as f32);

    div()
        .flex()
        .flex_col()
        .gap(metrics.spacing4())
        .px(metrics.spacing6())
        .py(metrics.spacing3())
        .child(
            slider_line(
                modal,
                "App transparency",
                TRANSPARENCY_SLIDER,
                transparency,
                format!("{}%", transparency as i64),
                cx,
            )
            .child(div().w(metrics.scaled(64.0)).flex_none()),
        )
        .child(
            slider_line(
                modal,
                "App vibrancy",
                BLUR_SLIDER,
                blur,
                format!("{}%", blur as i64),
                cx,
            )
            .child(div().flex_none().child(controls::button(
                style,
                "appearance-reset",
                "Reset",
                true,
                cx.listener(|modal: &mut SettingsModal, _, _, cx| {
                    modal.write(TRANSPARENCY, Value::Number(0.into()), cx);
                    modal.write(BLUR, Value::Number(70.into()), cx);
                }),
            ))),
        )
        .into_any_element()
}

pub(super) fn slider_line(
    modal: &SettingsModal,
    label: &str,
    spec: SliderSpec,
    value: f32,
    readout: String,
    cx: &mut Context<SettingsModal>,
) -> gpui::Div {
    let style = modal.style();
    let metrics = style.metrics;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(metrics.spacing4())
        .child(
            div()
                .flex_none()
                .text_size(metrics.font_body())
                .text_color(style.theme.fg)
                .child(SharedString::from(label.to_owned())),
        )
        .child(div().flex_grow().min_w(metrics.spacing4()))
        .child(controls::slider(
            style,
            spec.key,
            value,
            (spec.min, spec.max),
            cx.listener(move |modal: &mut SettingsModal, grab: &Grab, _, cx| {
                modal.begin_drag(spec, grab, cx);
            }),
        ))
        .child(
            div()
                .flex_none()
                .w(metrics.scaled(34.0))
                .text_size(metrics.font_footnote())
                .text_color(style.theme.fg_muted)
                .child(SharedString::from(readout)),
        )
}

fn tab_width_row(modal: &SettingsModal, cx: &mut Context<SettingsModal>) -> AnyElement {
    let style = modal.style();
    let metrics = style.metrics;
    let stored = settings::f64_value(TAB_MAX_WIDTH, 200.0) as f32;
    let value = modal.slider_value(TAB_WIDTH_SLIDER, stored);
    let readout = if value >= TAB_WIDTH_SLIDER.max {
        "Full-width".to_owned()
    } else {
        format!("{}px", value as i64)
    };

    controls::row(
        style,
        "Tab header width",
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.spacing3())
            .child(controls::slider(
                style,
                TAB_MAX_WIDTH,
                value,
                (TAB_WIDTH_SLIDER.min, TAB_WIDTH_SLIDER.max),
                cx.listener(|modal: &mut SettingsModal, grab: &Grab, _, cx| {
                    modal.begin_drag(TAB_WIDTH_SLIDER, grab, cx);
                }),
            ))
            .child(
                div()
                    .flex_none()
                    .w(metrics.scaled(64.0))
                    .text_size(metrics.font_body())
                    .text_color(style.theme.fg_muted)
                    .child(SharedString::from(readout)),
            )
            .into_any_element(),
    )
}
