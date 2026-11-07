//! Touch-sized preference and reading-data editor. Changes are committed through
//! the owner so persistence failures remain visible to the user.
use crate::widgets::{
    action, combo, disclosure, disclosure_with_toggle, field_pair, form_button as button,
    label as text, section, stack, FormValidation,
};
use ember_core::{validate_username, MuteRules, Preferences, ReaderState, ThemeMode};
use ferroui_base::input::{InputElement, Key, KeyEventArgs};
use ferroui_base::layout::{HorizontalAlignment, VerticalAlignment};
use ferroui_base::media::{Color, SolidColorBrush};
use ferroui_base::{BoxedValue, CornerRadius, Ref, Thickness, WeakRef};
use ferroui_controls::automation::{AutomationLiveSetting, AutomationProperties};
use ferroui_controls::primitives::{SelectingItemsControl, ToggleButton};
use ferroui_controls::{
    Border, Button, CheckBox, ColumnDefinitions, ComboBox, Control, DropDownButton, Grid,
    MenuFlyout, MenuItem, NumberBox, NumberBoxSpinButtonPlacementMode, NumberBoxValidationMode,
    ScrollViewer, StackPanel, TextBlock, TextBox, ToolTip, WrapPanel,
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    rc::Rc,
};

/// The longest text of a settings field; the backup field takes up to 16 MiB.
const FIELD_LENGTH: i32 = 8192;

fn field(
    parent: &StackPanel,
    label: &str,
    value: &str,
    placeholder: &str,
    multiline: bool,
) -> Ref<TextBox> {
    crate::widgets::field(parent, label, value, placeholder, FIELD_LENGTH, multiline)
}

pub enum SettingsAction {
    Preferences(Preferences),
    Mutes(MuteRules),
    Export(Rc<dyn Fn(&str)>),
    Import(ReaderState),
    MeasureStorage(Rc<dyn Fn(&str)>),
    CleanTemporary,
    ClearCache,
    ClearHistory,
}

pub(super) struct LivePreferences {
    saved: Rc<RefCell<Preferences>>,
    hide_read: WeakRef<CheckBox>,
    update_actions: Rc<dyn Fn()>,
}

impl LivePreferences {
    pub(super) fn set_hide_read(&self, enabled: bool) {
        self.saved.borrow_mut().hide_read = enabled;
        if let Some(field) = self.hide_read.upgrade() {
            field.set_is_checked(Some(enabled));
        }
        (self.update_actions)();
    }
}

pub(super) type RestorePreferences = Box<dyn FnOnce(&ReaderState) -> LivePreferences>;

struct PreferenceEditors {
    theme: Ref<ComboBox>,
    accent: Ref<TextBox>,
    background: Ref<TextBox>,
    size: Ref<NumberBox>,
    spacing: Ref<NumberBox>,
    cache: Ref<TextBox>,
    flags: [Ref<CheckBox>; 4],
}

impl PreferenceEditors {
    fn flags(p: &Preferences) -> [bool; 4] {
        [p.compact, p.reduced_motion, p.hide_read, p.show_dead]
    }

    fn restore(&self, p: &Preferences) {
        self.theme.set_selected_index(theme_index(p.theme));
        self.accent.set_text(Some(&p.accent));
        self.background
            .set_text(Some(p.custom_background.as_deref().unwrap_or("")));
        replace_reading_value(&self.size, p.font_size);
        replace_reading_value(&self.spacing, p.line_height);
        self.cache.set_text(Some(&p.cache_max_items.to_string()));
        for (field, value) in self.flags.iter().zip(Self::flags(p)) {
            field.set_is_checked(Some(value));
        }
    }

    fn clear_validation(&self) {
        let mut validation = FormValidation::default();
        for field in [
            self.accent.clone().upcast::<Control>(),
            self.background.clone().upcast(),
            self.size.clone().upcast(),
            self.spacing.clone().upcast(),
            self.cache.clone().upcast(),
        ] {
            validation.check(&field, Ok(()));
        }
    }

    fn changed(&self, p: &Preferences) -> bool {
        self.theme.selected_index() != theme_index(p.theme)
            || self.accent.text().unwrap_or_default() != p.accent
            || self.background.text().unwrap_or_default()
                != p.custom_background.as_deref().unwrap_or("")
            || self.cache.text().unwrap_or_default() != p.cache_max_items.to_string()
            || !number_matches(&self.size, p.font_size)
            || !number_matches(&self.spacing, p.line_height)
            || self
                .flags
                .iter()
                .zip(Self::flags(p))
                .any(|(field, saved)| field.is_checked() != Some(saved))
    }
}

fn theme_index(theme: ThemeMode) -> i32 {
    match theme {
        ThemeMode::System => 0,
        ThemeMode::Light => 1,
        ThemeMode::Dark => 2,
        ThemeMode::Custom => 3,
    }
}

// NumberBox commits its value on Enter or focus loss. An unfinished edit in the
// template's TextBox must already enable Apply and Discard.
fn number_matches(field: &NumberBox, saved: f64) -> bool {
    field.value() == saved
        && field
            .get_visual_descendants()
            .find_map(|v| v.cast::<TextBox>())
            .is_none_or(|editor| {
                let raw = editor.text().unwrap_or_default();
                raw == field.text() || raw.trim().parse::<f64>().ok() == Some(saved)
            })
}

fn build_preferences(
    root: &StackPanel,
    state: &ReaderState,
    on_action: Rc<dyn Fn(SettingsAction)>,
) -> LivePreferences {
    let appearance = section(root, "Appearance & reading");
    let (theme_field, accent_field) = field_pair(&appearance);
    let theme = combo(
        &theme_field,
        "Theme",
        &["Follow system", "Light", "Dark", "Custom"],
        theme_index(state.preferences.theme),
    );
    let accent = field(
        &accent_field,
        "Accent color",
        &state.preferences.accent,
        "#FF6B35",
        false,
    );
    color_preview(&accent, None);
    accent_choices(&accent_field, &accent);
    let custom = stack(4.0);
    custom.set_is_visible(theme.selected_index() == 3);
    appearance.children().add(custom.clone());
    let background = field(
        &custom,
        "Custom background",
        state.preferences.custom_background.as_deref().unwrap_or(""),
        "Optional, for example #172126",
        false,
    );
    color_preview(&background, Some("#181B1F"));
    let weak_theme = theme.downgrade();
    theme.selection_changed(move |_, _| {
        if let Some(theme) = weak_theme.upgrade() {
            custom.set_is_visible(theme.selected_index() == 3);
        }
    });
    let (type_size, leading) = field_pair(&appearance);
    let font_size = reading_number(
        &type_size,
        "Reading font size",
        state.preferences.font_size,
        12.0,
        32.0,
        1.0,
    );
    let line_height = reading_number(
        &leading,
        "Line spacing",
        state.preferences.line_height,
        1.1,
        2.2,
        0.1,
    );
    reading_preview_header(&appearance, &font_size, &line_height);
    let preview = crate::widgets::text(
        "The best part of a story is often in the discussion.",
        state.preferences.font_size,
    );
    preview.set_line_height(state.preferences.font_size * state.preferences.line_height);
    appearance.children().add(preview.clone());
    for input in [&font_size, &line_height] {
        let preview = preview.downgrade();
        let size = font_size.downgrade();
        let spacing = line_height.downgrade();
        input.property_changed(move |e| {
            if e.property() != NumberBox::value_property().as_property() {
                return;
            }
            if let (Some(preview), Some(size), Some(spacing)) =
                (preview.upgrade(), size.upgrade(), spacing.upgrade())
            {
                if size.value().is_finite() && spacing.value().is_finite() {
                    preview.set_font_size(size.value());
                    preview.set_line_height(size.value() * spacing.value());
                }
            }
        });
    }
    let (density, feed) = field_pair(&appearance);
    let compact = toggle(&density, "Compact layout", state.preferences.compact);
    let motion = toggle(&density, "Reduce motion", state.preferences.reduced_motion);
    let hide_read = toggle(&feed, "Hide read stories", state.preferences.hide_read);
    let show_dead = toggle(&feed, "Show flagged items", state.preferences.show_dead);
    let storage = disclosure(&appearance, "Offline storage limit", false);
    let cache_limit = field(
        &storage,
        "Maximum cached items",
        &state.preferences.cache_max_items.to_string(),
        "100–100000",
        false,
    );
    storage.children().add(text(
        "Ember keeps downloaded discussions when trimming its cache.",
        12.0,
        true,
    ));
    let validation = feedback(&appearance);
    let apply = button("Apply preferences");
    apply.classes().add("accent");
    let commands = WrapPanel::new();
    commands.set_item_spacing(6.0);
    commands.children().add(apply.clone());
    let reset = button("Reset theme & type");
    reset.classes().add("quiet");
    AutomationProperties::set_help_text(
        &reset,
        Some("Preview the default colors, reading size and line spacing."),
    );
    commands.children().add(reset.clone());
    let discard = button("Discard preference edits");
    discard.classes().add("quiet");
    commands.children().add(discard.clone());
    appearance.children().add(commands);
    let current = Rc::new(RefCell::new(state.preferences.clone()));
    let editors = Rc::new(PreferenceEditors {
        theme: theme.clone(),
        accent: accent.clone(),
        background: background.clone(),
        size: font_size.clone(),
        spacing: line_height.clone(),
        cache: cache_limit.clone(),
        flags: [
            compact.clone(),
            motion.clone(),
            hide_read.clone(),
            show_dead.clone(),
        ],
    });
    let update_actions: Rc<dyn Fn()> = {
        let (apply, discard) = (apply.downgrade(), discard.downgrade());
        let (editors, saved) = (editors.clone(), current.clone());
        let message = validation.downgrade();
        Rc::new(move || {
            let dirty = editors.changed(&saved.borrow());
            if let Some(apply) = apply.upgrade() {
                apply.set_is_enabled(dirty);
            }
            if let Some(discard) = discard.upgrade() {
                discard.set_is_visible(dirty);
            }
            if !dirty {
                editors.clear_validation();
                if let Some(message) = message.upgrade() {
                    message.set_is_visible(false);
                }
            }
        })
    };
    update_actions();
    // Listen on the section, including edits in the NumberBox template.
    let update = update_actions.clone();
    appearance.add_handler(TextBox::text_changing_event(), move |_, _| update());
    let update = update_actions.clone();
    appearance.add_handler(
        SelectingItemsControl::selection_changed_event(),
        move |_, _| update(),
    );
    let update = update_actions.clone();
    appearance.add_handler(ToggleButton::is_checked_changed_event(), move |_, _| {
        update()
    });
    {
        let (editors, saved, update) = (editors.clone(), current.clone(), update_actions.clone());
        let message = validation.clone();
        let reset = reset.downgrade();
        discard.click(move |_, _| {
            editors.restore(&saved.borrow());
            update();
            message.set_text(Some("Preference edits discarded."));
            message.set_is_visible(true);
            if let Some(button) = reset.upgrade() {
                button.focus();
            }
        });
    }
    let live = LivePreferences {
        saved: current.clone(),
        hide_read: hide_read.downgrade(),
        update_actions: update_actions.clone(),
    };
    {
        let theme = theme.clone();
        let accent = accent.clone();
        let background = background.clone();
        let size = font_size.clone();
        let spacing = line_height.clone();
        let message = validation.clone();
        let update = update_actions.clone();
        reset.click(move |_, _| {
            let defaults = Preferences::default();
            theme.set_selected_index(0);
            accent.set_text(Some(&defaults.accent));
            background.set_text(Some(""));
            replace_reading_value(&size, defaults.font_size);
            replace_reading_value(&spacing, defaults.line_height);
            let mut check = FormValidation::default();
            for field in [
                accent.clone().upcast::<Control>(),
                background.clone().upcast(),
                size.clone().upcast(),
                spacing.clone().upcast(),
            ] {
                check.check(&field, Ok(()));
            }
            update();
            message.set_text(Some("Default theme and type previewed."));
            message.set_is_visible(true);
        });
    }
    let callback = on_action.clone();
    apply.click(move |_, _| {
        let mut check = FormValidation::default();
        let mut p = current.borrow().clone();
        p.theme = match theme.selected_index() {
            1 => ThemeMode::Light,
            2 => ThemeMode::Dark,
            3 => ThemeMode::Custom,
            _ => ThemeMode::System,
        };
        let value = accent.text().unwrap_or_default();
        p.accent = value.trim().to_owned();
        check.check(
            &accent,
            if valid_color(&p.accent) {
                Ok(())
            } else {
                Err("Accent color needs six hex digits, for example #FF6B35.".into())
            },
        );
        let value = background.text().unwrap_or_default();
        let value = value.trim();
        check.check(
            &background,
            if p.theme != ThemeMode::Custom || value.is_empty() || valid_color(value) {
                Ok(())
            } else {
                Err("Custom background needs six hex digits, or leave it empty.".into())
            },
        );
        if value.is_empty() || valid_color(value) {
            p.custom_background = (!value.is_empty()).then(|| value.to_owned());
        }
        if let Some(size) = check.check(&font_size, reading_value(&font_size, "Reading font size"))
        {
            p.font_size = size;
        }
        if let Some(spacing) =
            check.check(&line_height, reading_value(&line_height, "Line spacing"))
        {
            p.line_height = spacing;
        }
        let limit = cache_limit
            .text()
            .unwrap_or_default()
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|v| (100..=100_000).contains(v))
            .ok_or_else(|| {
                "Maximum cached items must be a whole number between 100 and 100,000.".to_owned()
            });
        if let Some(limit) = check.check(&cache_limit, limit) {
            p.cache_max_items = limit;
        }
        if check.finish(&validation) {
            p.compact = compact.is_checked() == Some(true);
            p.reduced_motion = motion.is_checked() == Some(true);
            p.show_dead = show_dead.is_checked() == Some(true);
            p.hide_read = hide_read.is_checked() == Some(true);
            current.replace(p.clone());
            editors.restore(&p);
            update_actions();
            callback(SettingsAction::Preferences(p));
        }
    });

    mute_settings(root, &state.mutes, &on_action);
    live
}

pub(super) struct SettingsBack {
    review: Rc<RefCell<Option<PendingReview>>>,
    backup: WeakRef<StackPanel>,
    output: WeakRef<TextBox>,
    export: WeakRef<Button>,
}

impl SettingsBack {
    fn hide_backup(&self) -> bool {
        let Some(panel) = self.backup.upgrade().filter(|panel| panel.is_visible()) else {
            return false;
        };
        if let Some(output) = self.output.upgrade() {
            output.set_text(Some(""));
        }
        panel.set_is_visible(false);
        if let Some(trigger) = self.export.upgrade() {
            trigger.focus();
            trigger.bring_into_view();
        }
        true
    }

    pub(super) fn dismiss(&self) -> bool {
        finish_review(&self.review, true).is_some() || self.hide_backup()
    }
}

pub(super) fn build(
    on_action: Rc<dyn Fn(SettingsAction)>,
) -> (Ref<Control>, RestorePreferences, Rc<SettingsBack>) {
    let root = StackPanel::new();
    root.set_spacing(10.0);
    root.set_margin(Thickness::uniform(16.0));
    root.set_max_width(720.0);
    root.set_horizontal_alignment(HorizontalAlignment::Stretch);

    let preferences = stack(10.0);
    preferences.children().add(text(
        "Waiting for saved preferences. You can restore a backup below.",
        13.0,
        true,
    ));
    root.children().add(preferences.clone());
    let data = section(&root, "Reading data");
    data.children().add(text(
        "Back up bookmarks, history, filters, and preferences on this device. Downloaded story content is stored separately.",
        13.0, true,
    ));
    // Serialize on demand. A library can be up to 16 MiB and Settings opens on the UI thread.
    let backup = stack(4.0);
    backup.set_is_visible(false);
    let output = crate::widgets::field(
        &backup,
        "Generated backup JSON",
        "",
        "",
        16 * 1024 * 1024,
        true,
    );
    output.set_is_read_only(true);
    let callback = on_action.clone();
    let show_backup = backup.clone();
    let export_output = output.clone();
    let export = action("Copy backup", "copy", move || {
        let output = export_output.downgrade();
        let panel = show_backup.downgrade();
        callback(SettingsAction::Export(Rc::new(move |json| {
            if let (Some(output), Some(panel)) = (output.upgrade(), panel.upgrade()) {
                output.set_text(Some(json));
                panel.set_is_visible(true);
                output.focus();
                output.select_all();
                output.bring_into_view();
            }
        })));
    });
    export.set_horizontal_alignment(HorizontalAlignment::Left);
    export.set_is_visible(false);
    data.children().add(export.clone());
    let dismiss = button("Hide backup");
    dismiss.classes().add("quiet");
    let review = Rc::new(RefCell::new(None::<PendingReview>));
    let back = Rc::new(SettingsBack {
        review: review.clone(),
        backup: backup.downgrade(),
        output: output.downgrade(),
        export: export.downgrade(),
    });
    let close_backup = back.clone();
    dismiss.click(move |_, _| {
        close_backup.hide_backup();
    });
    backup.children().add(dismiss);
    data.children().add(backup);
    let json = crate::widgets::field(
        &data,
        "Reading data JSON",
        "",
        "Paste an Ember backup to restore",
        16 * 1024 * 1024,
        true,
    );
    let input = json.clone();
    let import_error = reviewed_action(
        &data,
        &review,
        ("Import this backup", "download"),
        "Replace reading data",
        move || {
            let value = json.text().unwrap_or_default();
            if value.trim().is_empty() {
                return Err((
                    json.clone().upcast(),
                    "Paste an Ember backup first.".to_owned(),
                ));
            }
            let state = ReaderState::import_json(&value).map_err(|error| {
                (
                    json.clone().upcast(),
                    format!("Reading data JSON is invalid. {error}"),
                )
            })?;
            let saved = state
                .items
                .values()
                .filter(|item| item.saved_at.is_some())
                .count();
            let read = state
                .items
                .values()
                .filter(|item| item.last_opened_at.is_some())
                .count();
            let stories = if saved == 1 { "story" } else { "stories" };
            let entries = if read == 1 { "entry" } else { "entries" };
            let summary = format!("Replace this device’s library, filters, and preferences? The backup contains {saved} saved {stories} and {read} history {entries}.");
            Ok((summary, SettingsAction::Import(state)))
        },
        &on_action,
    );

    let edited = review.clone();
    let weak_input = input.downgrade();
    input.property_changed(move |e| {
        if e.property() != TextBox::text_property().as_property() {
            return;
        }
        import_error.set_is_visible(false);
        if let Some(input) = weak_input.upgrade() {
            FormValidation::default().check(&input, Ok(()));
        }
        if edited
            .borrow()
            .as_ref()
            .is_some_and(|review| matches!(review.action, SettingsAction::Import(_)))
        {
            finish_review(&edited, false);
        }
    });

    let cleanup = stack(0.0);
    cleanup.set_is_visible(false);
    root.children().add(cleanup.clone());
    let (disclosure, maintenance) = disclosure_with_toggle(&cleanup, "Clear local data", false);
    let active = review.clone();
    let toggle = disclosure.downgrade();
    disclosure.property_changed(move |event| {
        if event.property() == ferroui_controls::Expander::is_expanded_property().as_property()
            && toggle.upgrade().is_some_and(|d| !d.is_expanded())
            && active
                .borrow()
                .as_ref()
                .is_some_and(|pending| !matches!(pending.action, SettingsAction::Import(_)))
        {
            finish_review(&active, false);
        }
    });
    let usage = text("Measure cached data on this device.", 13.0, true);
    AutomationProperties::set_live_setting(&usage, AutomationLiveSetting::Polite);
    maintenance.children().add(usage.clone());
    let measure = button("Check storage use");
    let weak_button = measure.downgrade();
    let weak_usage = usage.downgrade();
    let action = on_action.clone();
    measure.click(move |_, _| {
        let (Some(button), Some(label)) = (weak_button.upgrade(), weak_usage.upgrade()) else {
            return;
        };
        button.set_is_enabled(false);
        label.set_text(Some("Measuring cached data…"));
        let report_button = button.downgrade();
        let report_label = label.downgrade();
        action(SettingsAction::MeasureStorage(Rc::new(move |result| {
            if let Some(button) = report_button.upgrade() {
                button.set_is_enabled(true);
            }
            if let Some(label) = report_label.upgrade() {
                label.set_text(Some(result));
            }
        })));
    });
    maintenance.children().add(measure);
    reviewed_action(
        &maintenance,
        &review,
        ("Clean temporary cache", "refresh"),
        "Clean temporary data",
        || {
            Ok(("Remove temporary story data, cached searches, and profiles? Bookmarks, feed lists, and downloaded discussions are kept.".into(), SettingsAction::CleanTemporary))
        },
        &on_action,
    );
    reviewed_action(
        &maintenance,
        &review,
        ("Clear cached content", "download"),
        "Remove cached content",
        || {
            Ok(("Remove downloaded discussions and cached stories from this device? You can download them again. Bookmarks, history, and preferences are kept.".into(), SettingsAction::ClearCache))
        },
        &on_action,
    );
    reviewed_action(
        &maintenance,
        &review,
        ("Clear reading history", "history"),
        "Remove reading history",
        || {
            Ok(("Remove read markers, reading positions, and recent searches from this device? Bookmarks and downloaded discussions are kept.".into(), SettingsAction::ClearHistory))
        },
        &on_action,
    );
    let scroll = ScrollViewer::new();
    scroll.set_content(Some(Control::boxed(root)));
    let restore = Box::new(move |state: &ReaderState| {
        preferences.children().clear();
        let live = build_preferences(&preferences, state, on_action);
        export.set_is_visible(true);
        cleanup.set_is_visible(true);
        live
    });
    (scroll.upcast(), restore, back)
}

struct PendingReview {
    trigger: WeakRef<Button>,
    panel: WeakRef<StackPanel>,
    action: SettingsAction,
}

// Taking the action before dispatch prevents repeated input from applying it twice.
fn finish_review(
    review: &RefCell<Option<PendingReview>>,
    restore_focus: bool,
) -> Option<SettingsAction> {
    let pending = review.borrow_mut().take()?;
    if let Some(panel) = pending.panel.upgrade() {
        panel.set_is_visible(false);
    }
    if let Some(trigger) = pending.trigger.upgrade() {
        trigger.set_is_enabled(true);
        if restore_focus {
            trigger.focus();
            trigger.bring_into_view();
        }
    }
    Some(pending.action)
}

fn reviewed_action(
    parent: &StackPanel,
    review: &Rc<RefCell<Option<PendingReview>>>,
    (label, icon): (&str, &str),
    confirm_label: &str,
    prepare: impl Fn() -> Result<(String, SettingsAction), (Ref<Control>, String)> + 'static,
    on_action: &Rc<dyn Fn(SettingsAction)>,
) -> Ref<TextBlock> {
    let trigger = button(label);
    trigger.classes().add("quiet");
    trigger.set_content(Some(Control::boxed(crate::widgets::nav_content(
        icon, label, false,
    ))));
    parent.children().add(trigger.clone());
    let feedback = feedback(parent);
    let error = feedback.clone();
    let panel = stack(6.0);
    panel.set_is_visible(false);
    parent.children().add(panel.clone());
    let prompt = text("", 13.0, false);
    AutomationProperties::set_live_setting(&prompt, AutomationLiveSetting::Polite);
    panel.children().add(prompt.clone());
    let buttons = WrapPanel::new();
    buttons.set_item_spacing(8.0);
    buttons.set_line_spacing(4.0);
    let cancel = button("Cancel");
    AutomationProperties::set_name(&cancel, Some(&format!("Cancel {label}")));
    let confirm = button(confirm_label);
    buttons.children().add(cancel.clone());
    buttons.children().add(confirm.clone());
    panel.children().add(buttons);

    let state = review.clone();
    let target = trigger.downgrade();
    let host = panel.downgrade();
    let first = cancel.downgrade();
    trigger.click(move |_, _| {
        let (Some(trigger), Some(panel), Some(cancel)) =
            (target.upgrade(), host.upgrade(), first.upgrade())
        else {
            return;
        };
        if !trigger.is_enabled() {
            return;
        }
        finish_review(&state, false);
        match prepare() {
            Ok((description, action)) => {
                state.replace(Some(PendingReview {
                    trigger: target.clone(),
                    panel: host.clone(),
                    action,
                }));
                error.set_is_visible(false);
                prompt.set_text(Some(&description));
                trigger.set_is_enabled(false);
                panel.set_is_visible(true);
                cancel.focus();
                cancel.bring_into_view();
            }
            Err((field, message)) => {
                let mut check = FormValidation::default();
                check.check::<()>(&field, Err(message));
                check.finish(&error);
            }
        }
    });
    let state = review.clone();
    cancel.click(move |_, _| {
        finish_review(&state, true);
    });
    let state = review.clone();
    panel.add_handler(
        InputElement::key_down_event(),
        move |_, e: &KeyEventArgs| {
            if e.key == Key::Escape {
                e.set_handled(true);
                finish_review(&state, true);
            }
        },
    );
    let state = review.clone();
    let target = trigger.downgrade();
    let callback = on_action.clone();
    confirm.click(move |_, _| {
        if target
            .upgrade()
            .is_some_and(|trigger| !trigger.is_enabled())
        {
            if let Some(action) = finish_review(&state, true) {
                callback(action);
            }
        }
    });
    feedback
}

fn feedback(parent: &StackPanel) -> Ref<TextBlock> {
    let message = text("", 13.0, false);
    message.set_is_visible(false);
    AutomationProperties::set_live_setting(&message, AutomationLiveSetting::Polite);
    parent.children().add(message.clone());
    message
}

fn toggle(parent: &StackPanel, label: &str, value: bool) -> Ref<CheckBox> {
    let input = CheckBox::new();
    input.set_content(Some(Rc::new(label.to_owned()) as BoxedValue));
    input.set_is_checked(Some(value));
    input.set_min_height(44.0);
    input.set_vertical_content_alignment(VerticalAlignment::Center);
    input.set_padding(Thickness::new(8.0, 0.0, 0.0, 0.0));
    AutomationProperties::set_name(&input, Some(label));
    parent.children().add(input.clone());
    input
}
/// Put the draft color in the field so custom hex values get the same preview as presets.
fn color_preview(input: &TextBox, default: Option<&'static str>) {
    let swatch = Border::new();
    swatch.set_width(16.0);
    swatch.set_height(16.0);
    swatch.set_margin(Thickness::new(8.0, 0.0, 0.0, 0.0));
    swatch.set_corner_radius(CornerRadius::uniform(8.0));
    swatch.set_vertical_alignment(VerticalAlignment::Center);
    swatch.set_border_thickness(Thickness::uniform(1.0));
    crate::widgets::bind_resource(
        &swatch,
        Border::border_brush_property().as_property(),
        "EmberMuted",
    );
    input.set_inner_left_content(Some(Control::boxed(&swatch)));
    let weak_input = input.to_ref().downgrade();
    let update = move || {
        if let Some(input) = weak_input.upgrade() {
            let raw = input.text().unwrap_or_default();
            let value = if raw.trim().is_empty() {
                default.unwrap_or("")
            } else {
                raw.trim()
            };
            swatch.set_background(
                valid_color(value)
                    .then(|| Color::parse(value).ok())
                    .flatten()
                    .map(|c| SolidColorBrush::with_color(c).into()),
            );
        }
    };
    update();
    input.property_changed(move |e| {
        if e.property() == TextBox::text_property().as_property() {
            update();
        }
    });
}

fn accent_choices(parent: &StackPanel, input: &TextBox) {
    let choices = WrapPanel::new();
    choices.set_item_spacing(2.0);
    for (name, hex, ink) in [
        ("Ember", "#FF6B35", "#151515"),
        ("Ocean", "#2864A7", "#FFFFFF"),
        ("Forest", "#216E58", "#FFFFFF"),
        ("Plum", "#805990", "#FFFFFF"),
    ] {
        let choice = Button::new();
        choice.classes().add("quiet");
        choice.set_width(44.0);
        choice.set_height(44.0);
        choice.set_padding(Thickness::uniform(8.0));
        let chip = Border::new();
        chip.set_width(24.0);
        chip.set_height(24.0);
        chip.set_corner_radius(CornerRadius::uniform(12.0));
        chip.set_border_thickness(Thickness::uniform(1.0));
        crate::widgets::bind_resource(
            &chip,
            Border::border_brush_property().as_property(),
            "EmberMuted",
        );
        chip.set_background(Some(
            SolidColorBrush::with_color(Color::parse(hex).expect("accent preset")).into(),
        ));
        let mark = crate::icons::icon("check");
        mark.set_width(14.0);
        mark.set_height(14.0);
        mark.set_horizontal_alignment(HorizontalAlignment::Center);
        mark.set_vertical_alignment(VerticalAlignment::Center);
        mark.set_foreground(Some(
            SolidColorBrush::with_color(Color::parse(ink).expect("accent mark")).into(),
        ));
        chip.set_child(mark.clone());
        choice.set_content(Some(Control::boxed(chip)));
        let label = format!("{name} accent");
        AutomationProperties::set_name(&choice, Some(&label));
        ToolTip::set_tip(
            &choice,
            Some(Rc::new(format!("{name} · {hex}")) as BoxedValue),
        );
        let editor = input.to_ref().downgrade();
        choice.click(move |_, _| {
            if let Some(input) = editor.upgrade() {
                input.set_text(Some(hex));
            }
        });
        let editor = input.to_ref().downgrade();
        let button = choice.downgrade();
        let update = move || {
            if let (Some(input), Some(button)) = (editor.upgrade(), button.upgrade()) {
                let selected = input
                    .text()
                    .is_some_and(|s| s.trim().eq_ignore_ascii_case(hex));
                mark.set_is_visible(selected);
                AutomationProperties::set_item_status(&button, selected.then_some("Selected"));
            }
        };
        update();
        input.property_changed(move |e| {
            if e.property() == TextBox::text_property().as_property() {
                update();
            }
        });
        choices.children().add(choice);
    }
    parent.children().add(choices);
}

fn valid_color(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}
fn reading_preview_header(parent: &StackPanel, size: &NumberBox, spacing: &NumberBox) {
    let row = Grid::new();
    row.set_column_definitions(
        ColumnDefinitions::parse("*,Auto").expect("reading preview columns"),
    );
    let caption = text("Reading preview", 12.0, true);
    caption.set_vertical_alignment(VerticalAlignment::Center);
    row.children().add(caption);
    let presets = DropDownButton::new();
    presets.classes().add("quiet");
    presets.set_min_height(44.0);
    presets.set_content(Some(Rc::new("Text presets".to_owned())));
    AutomationProperties::set_name(&presets, Some("Text presets"));
    let menu = MenuFlyout::new();
    for (label, font_size, line_height) in [
        ("Small · 14 px / 1.4×", 14.0, 1.4),
        ("Standard · 16 px / 1.5×", 16.0, 1.5),
        ("Large · 20 px / 1.6×", 20.0, 1.6),
        ("Extra large · 24 px / 1.7×", 24.0, 1.7),
    ] {
        let item = MenuItem::new();
        item.set_header(Some(Rc::new(label.to_owned())));
        let size = size.to_ref().downgrade();
        let spacing = spacing.to_ref().downgrade();
        item.click(move |_, _| {
            for (editor, value) in [(&size, font_size), (&spacing, line_height)] {
                if let Some(editor) = editor.upgrade() {
                    replace_reading_value(&editor, value);
                }
            }
        });
        menu.items().add(Some(Control::boxed(item)));
    }
    presets.set_flyout(menu);
    Grid::set_column(&presets, 1);
    row.children().add(presets);
    parent.children().add(row);
}

// A deliberate replacement must also overwrite an unfinished or invalid edit.
fn replace_reading_value(editor: &NumberBox, value: f64) {
    let validation = editor.validation_mode();
    editor.set_validation_mode(NumberBoxValidationMode::InvalidInputOverwritten);
    editor.set_numeric_value(value);
    editor.set_validation_mode(validation);
}

fn reading_number(
    parent: &StackPanel,
    label: &str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
) -> Ref<NumberBox> {
    let input = NumberBox::new();
    input.set_header(Some(Rc::new(label.to_owned()) as BoxedValue));
    input.set_minimum(min);
    input.set_maximum(max);
    input.set_small_change(step);
    input.set_validation_mode(NumberBoxValidationMode::Disabled);
    input.set_numeric_value(value);
    input.set_spin_button_placement_mode(NumberBoxSpinButtonPlacementMode::Inline);
    AutomationProperties::set_name(&input, Some(label));
    parent.children().add(input.clone());
    input
}

fn reading_value(input: &NumberBox, label: &str) -> Result<f64, String> {
    if input.validate_input() && (input.minimum()..=input.maximum()).contains(&input.value()) {
        Ok(input.value())
    } else {
        Err(format!(
            "{label} must be between {} and {}.",
            input.minimum(),
            input.maximum()
        ))
    }
}

fn mute_settings(parent: &StackPanel, rules: &MuteRules, on_action: &Rc<dyn Fn(SettingsAction)>) {
    let panel = disclosure(parent, "Muted domains, authors & keywords", false);
    let summary = text(&mute_summary(rules), 12.0, true);
    AutomationProperties::set_live_setting(&summary, AutomationLiveSetting::Polite);
    panel.children().add(summary.clone());
    let kind = combo(
        &panel,
        "Edit mute rules",
        &["Domains", "Authors", "Title keywords"],
        0,
    );
    let editors = [
        (
            "Muted domains",
            "example.com or an article URL",
            "A domain also mutes its subdomains.",
            &rules.domains,
        ),
        (
            "Muted authors",
            "Hacker News usernames",
            "Usernames are case-sensitive. A leading @ is optional.",
            &rules.users,
        ),
        (
            "Muted title keywords",
            "Words or phrases",
            "Keywords match anywhere in a story title, regardless of case.",
            &rules.keywords,
        ),
    ];
    let groups = editors.map(|(label, placeholder, hint, values)| {
        let group = stack(4.0);
        // Keep larger rule sets imported from an existing library fully editable.
        let original = join(values);
        let editor = crate::widgets::field(
            &group,
            label,
            &original,
            placeholder,
            FIELD_LENGTH.max(original.len() as i32),
            true,
        );
        group.children().add(text(hint, 12.0, true));
        panel.children().add(group.clone());
        (group, editor)
    });
    let hosts = groups.each_ref().map(|(group, _)| group.downgrade());
    let selected = kind.downgrade();
    let switch = move || {
        if let Some(kind) = selected.upgrade() {
            for (i, host) in hosts.iter().enumerate() {
                if let Some(host) = host.upgrade() {
                    host.set_is_visible(kind.selected_index() == i as i32);
                }
            }
        }
    };
    switch();
    kind.selection_changed(move |_, _| switch());
    panel.children().add(text(
        "Separate entries with commas or new lines.",
        12.0,
        true,
    ));
    let message = feedback(&panel);
    let changing = text("Unsaved filter changes.", 12.0, true);
    AutomationProperties::set_live_setting(&changing, AutomationLiveSetting::Polite);
    panel.children().add(changing.clone());
    let actions = WrapPanel::new();
    let save = button("Save filters");
    let discard = button("Discard filter edits");
    discard.classes().add("quiet");
    actions.children().add(save.clone());
    actions.children().add(discard.clone());
    panel.children().add(actions);

    let fields = groups.map(|(_, field)| field);
    let baseline = Rc::new(RefCell::new(
        fields
            .each_ref()
            .map(|field| field.text().unwrap_or_default()),
    ));
    let dirty = Rc::new(Cell::new([false; 3]));
    let submit = save.downgrade();
    let undo = discard.downgrade();
    let refresh = Rc::new(move |flags: [bool; 3]| {
        let changed = flags.contains(&true);
        if let Some(save) = submit.upgrade() {
            save.set_is_enabled(changed);
        }
        if let Some(discard) = undo.upgrade() {
            discard.set_is_visible(changed);
        }
        changing.set_is_visible(changed);
    });
    refresh(dirty.get());
    for (i, field) in fields.iter().enumerate() {
        let (baseline, dirty, refresh) = (baseline.clone(), dirty.clone(), refresh.clone());
        let input = field.downgrade();
        field.property_changed(move |event| {
            if event.property() != TextBox::text_property().as_property() {
                return;
            }
            if let Some(input) = input.upgrade() {
                let mut flags = dirty.get();
                flags[i] = input.text().unwrap_or_default() != baseline.borrow()[i];
                dirty.set(flags);
                refresh(flags);
            }
        });
    }
    {
        let fields = fields.clone();
        let baseline = baseline.clone();
        let kind = kind.clone();
        let message = message.clone();
        discard.click(move |_, _| {
            let mut check = FormValidation::default();
            for (field, saved) in fields.iter().zip(baseline.borrow().iter()) {
                field.set_text(Some(saved));
                check.check(field, Ok(()));
            }
            message.set_is_visible(false);
            if let Some(field) = fields.get(kind.selected_index() as usize) {
                field.focus();
                field.bring_into_view();
            }
        });
    }
    let [domains, users, keywords] = fields;
    let callback = on_action.clone();
    save.click(move |_, _| {
        let mut check = FormValidation::default();
        let domain_rules = check.check(
            &domains,
            split(&domains.text().unwrap_or_default())
                .into_iter()
                .map(|entry| {
                    MuteRules::domain_from_input(&entry)
                        .ok_or_else(|| format!("Enter a hostname or a web link for “{entry}”."))
                })
                .collect::<Result<BTreeSet<_>, _>>(),
        );
        let user_rules = check.check(
            &users,
            split(&users.text().unwrap_or_default())
                .into_iter()
                .map(|entry| {
                    let author = entry.strip_prefix('@').unwrap_or(&entry);
                    validate_username(author)
                        .map_err(|_| format!("“{entry}” is not a Hacker News username."))?;
                    Ok::<_, String>(author.to_owned())
                })
                .collect::<Result<BTreeSet<_>, _>>(),
        );
        if domain_rules.is_none() {
            kind.set_selected_index(0);
        } else if user_rules.is_none() {
            kind.set_selected_index(1);
        }
        if !check.finish(&message) {
            return;
        }
        let rules = MuteRules {
            domains: domain_rules.unwrap_or_default(),
            users: user_rules.unwrap_or_default(),
            keywords: split(&keywords.text().unwrap_or_default()),
        };
        baseline.replace([
            join(&rules.domains),
            join(&rules.users),
            join(&rules.keywords),
        ]);
        for (field, saved) in [&domains, &users, &keywords]
            .into_iter()
            .zip(baseline.borrow().iter())
        {
            field.set_text(Some(saved));
        }
        dirty.set([false; 3]);
        refresh(dirty.get());
        summary.set_text(Some(&mute_summary(&rules)));
        callback(SettingsAction::Mutes(rules));
    });
}

fn mute_summary(rules: &MuteRules) -> String {
    if rules.domains.is_empty() && rules.users.is_empty() && rules.keywords.is_empty() {
        return "No mute rules saved.".into();
    }
    [
        (rules.domains.len(), "domain"),
        (rules.users.len(), "author"),
        (rules.keywords.len(), "keyword"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, name)| format!("{count} {name}{}", if count == 1 { "" } else { "s" }))
    .collect::<Vec<_>>()
    .join(" · ")
}

fn join(values: &BTreeSet<String>) -> String {
    values.iter().cloned().collect::<Vec<_>>().join("\n")
}
fn split(value: &str) -> BTreeSet<String> {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroui_controls::Window;
    use sample_testing::Shell;

    #[test]
    fn generated_backup_can_be_copied_manually_and_dismissed_on_a_phone() {
        use ferroui_base::interactivity::RoutedEventArgs;
        let shell = Shell::start(&crate::SAMPLE, 360.0, 640.0, || crate::App::new().upcast());
        let (control, restore, _) = build(Rc::new(|action| {
            if let SettingsAction::Export(show) = action {
                show("snapshot");
            }
        }));
        restore(&ReaderState::default());
        let window = Window::new();
        window.set_content(Some(Control::boxed(control)));
        window.show();
        shell.settle();
        let named = |name| {
            window
                .get_visual_descendants()
                .filter_map(|v| v.cast::<Control>())
                .find(|v| AutomationProperties::get_name(v).as_deref() == Some(name))
                .unwrap()
        };
        let output = named("Generated backup JSON").cast::<TextBox>().unwrap();
        let export = named("Copy backup");
        export.raise_event(&RoutedEventArgs::with_event(Button::click_event()));
        shell.settle();
        assert!(output.is_effectively_visible() && output.is_focused());
        assert_eq!(output.selected_text(), "snapshot");
        named("Hide backup").raise_event(&RoutedEventArgs::with_event(Button::click_event()));
        shell.settle();
        assert!(!output.is_effectively_visible());
        assert_eq!(output.text().as_deref(), Some(""));
        assert!(export.is_focused());
        window.close();
    }

    #[test]
    fn destructive_actions_require_one_confirmation_and_restore_focus_on_cancel() {
        let shell = Shell::start(&crate::SAMPLE, 400.0, 600.0, || crate::App::new().upcast());
        let host = stack(6.0);
        let review = Rc::new(RefCell::new(None));
        let events = Rc::new(RefCell::new(Vec::new()));
        let changes = events.clone();
        let callback: Rc<dyn Fn(SettingsAction)> =
            Rc::new(move |action| changes.borrow_mut().push(action));
        reviewed_action(
            &host,
            &review,
            ("Clear cache", "download"),
            "Remove cache",
            || Ok(("Clear downloads?".into(), SettingsAction::ClearCache)),
            &callback,
        );
        reviewed_action(
            &host,
            &review,
            ("Clear history", "history"),
            "Remove history",
            || Ok(("Clear history?".into(), SettingsAction::ClearHistory)),
            &callback,
        );
        let window = Window::new();
        window.set_content(Some(Control::boxed(host)));
        window.show();
        shell.settle();
        let named = |name: &str| {
            window
                .get_visual_descendants()
                .filter_map(|v| v.cast::<Button>())
                .find(|v| AutomationProperties::get_name(v).as_deref() == Some(name))
                .unwrap()
        };
        let click = |name| {
            shell.click(0, Shell::bounds_of(&window, &named(name)).center());
            shell.settle();
        };
        let origin = Shell::bounds_of(&window, &named("Clear cache")).center();
        shell.click(0, origin);
        shell.settle();
        shell.click(0, origin);
        shell.settle();
        assert!(events.borrow().is_empty());
        assert!(!named("Clear cache").is_enabled());
        assert!(named("Remove cache").is_effectively_visible());
        click("Clear history");
        assert!(named("Clear cache").is_enabled());
        assert!(!named("Remove cache").is_effectively_visible());
        let cancel = named("Cancel Clear history");
        assert!(cancel.is_focused());
        let mut escape = KeyEventArgs::new();
        escape.key = Key::Escape;
        escape.set_routed_event(Some(InputElement::key_down_event()));
        cancel.raise_event(&escape);
        shell.settle();
        assert!(escape.handled());
        assert!(named("Clear history").is_focused());
        assert!(events.borrow().is_empty());
        click("Clear history");
        click("Remove history");
        assert!(matches!(
            events.borrow().as_slice(),
            [SettingsAction::ClearHistory]
        ));
        assert!(!named("Remove history").is_effectively_visible());
        window.close();
    }

    #[test]
    fn mute_editors_keep_drafts_reveal_errors_and_save_canonical_rules() {
        let shell = Shell::start(&crate::SAMPLE, 360.0, 645.0, || crate::App::new().upcast());
        let host = stack(4.0);
        let saved = Rc::new(RefCell::new(None));
        let result = saved.clone();
        let callback: Rc<dyn Fn(SettingsAction)> = Rc::new(move |a| {
            if let SettingsAction::Mutes(rules) = a {
                result.replace(Some(rules));
            }
        });
        mute_settings(&host, &MuteRules::default(), &callback);
        let w = Window::new();
        w.set_content(Some(Control::boxed(host)));
        w.show();
        shell.settle();
        let named = |name: &str| {
            w.get_visual_descendants()
                .filter_map(|v| v.cast::<Control>())
                .find(|c| AutomationProperties::get_name(c).as_deref() == Some(name))
                .unwrap()
        };
        named("Muted domains, authors & keywords")
            .cast::<ferroui_controls::Expander>()
            .unwrap()
            .set_is_expanded(true);
        shell.settle();
        let kind = named("Edit mute rules").cast::<ComboBox>().unwrap();
        let domains = named("Muted domains").cast::<TextBox>().unwrap();
        let users = named("Muted authors").cast::<TextBox>().unwrap();
        let keywords = named("Muted title keywords").cast::<TextBox>().unwrap();
        let click_save = || {
            shell.click(0, Shell::bounds_of(&w, &named("Save filters")).center());
            shell.settle();
        };
        domains.set_text(Some("https://person@example.org"));
        kind.set_selected_index(1);
        users.set_text(Some("@Lin, wrong name"));
        kind.set_selected_index(2);
        shell.settle();
        assert!(!domains.is_effectively_visible() && !users.is_effectively_visible());
        keywords.set_text(Some("game, ai"));
        click_save();
        assert_eq!(kind.selected_index(), 0);
        assert!(domains.is_focused());
        assert!(saved.borrow().is_none());
        domains.set_text(Some("https://www.example.org/post, example.org:443"));
        click_save();
        assert_eq!(kind.selected_index(), 1);
        assert!(users.is_focused());
        users.set_text(Some("@Lin"));
        click_save();
        let rules = saved.borrow();
        let rules = rules.as_ref().unwrap();
        assert_eq!(join(&rules.domains), "example.org");
        assert_eq!(join(&rules.users), "Lin");
        assert_eq!(join(&rules.keywords), "ai\ngame");
        assert_eq!(domains.text().as_deref(), Some("example.org"));
        assert_eq!(mute_summary(rules), "1 domain · 1 author · 2 keywords");
        assert!(!named("Save filters").is_enabled());
        domains.set_text(Some("new.example"));
        users.set_text(Some("invalid author"));
        click_save();
        assert!(ferroui_controls::DataValidationErrors::get_has_errors(
            &users
        ));
        keywords.set_text(Some("one more"));
        shell.settle();
        let discard = named("Discard filter edits");
        assert!(discard.is_effectively_visible());
        shell.click(0, Shell::bounds_of(&w, &discard).center());
        shell.settle();
        assert!(users.is_focused());
        assert!(!ferroui_controls::DataValidationErrors::get_has_errors(
            &users
        ));
        assert_eq!(domains.text().as_deref(), Some("example.org"));
        assert_eq!(users.text().as_deref(), Some("Lin"));
        assert_eq!(keywords.text().as_deref(), Some("ai\ngame"));
        assert!(!discard.is_effectively_visible());
        assert!(!named("Save filters").is_enabled());
        w.close();
    }

    #[test]
    fn reading_steps_preview_and_apply_without_losing_custom_background() {
        let shell = Shell::start(&crate::SAMPLE, 720.0, 1120.0, || crate::App::new().upcast());
        let applied = Rc::new(RefCell::new(Preferences::default()));
        let result = applied.clone();
        let (form, restore, _) = build(Rc::new(move |action| {
            if let SettingsAction::Preferences(prefs) = action {
                result.replace(prefs);
            }
        }));
        restore(&ReaderState::default());
        let window = Window::new();
        window.set_width(720.0);
        window.set_height(1120.0);
        window.set_content(Some(Control::boxed(form)));
        window.show();
        shell.settle();
        let named = |name: &str| {
            window
                .get_visual_descendants()
                .filter_map(|v| v.cast::<Control>())
                .find(|c| AutomationProperties::get_name(c).as_deref() == Some(name))
                .unwrap_or_else(|| panic!("missing {name}"))
        };

        let accent = named("Accent color").cast::<TextBox>().unwrap();
        let ocean = named("Ocean accent");
        shell.click(0, Shell::bounds_of(&window, &ocean).center());
        shell.settle();
        assert_eq!(accent.text().as_deref(), Some("#2864A7"));
        assert_eq!(
            AutomationProperties::get_item_status(&ocean).as_deref(),
            Some("Selected")
        );
        assert!(accent.inner_left_content().is_some());
        let size = named("Reading font size").cast::<NumberBox>().unwrap();
        size.step_value(size.small_change());
        let preview = window
            .get_visual_descendants()
            .filter_map(|v| v.cast::<TextBlock>())
            .find(|v| {
                v.text().as_deref() == Some("The best part of a story is often in the discussion.")
            })
            .unwrap();
        assert_eq!(preview.font_size(), 17.0);
        assert_eq!(preview.line_height(), 25.5);
        let theme = named("Theme").cast::<ComboBox>().unwrap();
        let background = named("Custom background").cast::<TextBox>().unwrap();
        assert!(!background.is_effectively_visible());
        theme.set_selected_index(3);
        shell.settle();
        assert!(background.is_effectively_visible());
        background.set_text(Some("#29405D"));
        let apply = || {
            shell.click(
                0,
                Shell::bounds_of(&window, &named("Apply preferences")).center(),
            );
            shell.settle();
        };
        apply();
        assert_eq!(applied.borrow().theme, ThemeMode::Custom);
        theme.set_selected_index(1);
        shell.settle();
        apply();
        assert_eq!(applied.borrow().theme, ThemeMode::Light);
        assert_eq!(
            applied.borrow().custom_background.as_deref(),
            Some("#29405D")
        );
        assert_eq!(applied.borrow().font_size, 17.0);
        assert_eq!(applied.borrow().accent, "#2864A7");

        let storage = named("Offline storage limit")
            .cast::<ferroui_controls::Expander>()
            .unwrap();
        storage.set_is_expanded(true);
        shell.settle();
        let cache = named("Maximum cached items").cast::<TextBox>().unwrap();
        cache.set_text(Some("many"));
        storage.set_is_expanded(false);
        shell.settle();
        accent.set_text(Some("#123"));
        apply();
        assert!(accent.is_focused());
        assert!(ferroui_controls::DataValidationErrors::get_has_errors(
            &accent
        ));
        assert!(ferroui_controls::DataValidationErrors::get_has_errors(
            &cache
        ));
        assert_eq!(accent.text().as_deref(), Some("#123"));
        assert!(AutomationProperties::get_item_status(&ocean).is_none());

        accent.set_text(Some("#196B73"));
        apply();
        assert!(cache.is_focused());
        assert!(cache.is_effectively_visible());
        assert_eq!(cache.text().as_deref(), Some("many"));
        assert_eq!(applied.borrow().accent, "#2864A7");
        cache.set_text(Some("5000"));
        apply();
        assert_eq!(applied.borrow().accent, "#196B73");

        let size_editor = size
            .get_visual_descendants()
            .find_map(|v| v.cast::<TextBox>())
            .unwrap();
        size_editor.set_text(Some("twenty"));
        apply();
        assert!(size_editor.is_focused());
        assert_eq!(size_editor.text().as_deref(), Some("twenty"));
        assert_eq!(applied.borrow().font_size, 17.0);
        size_editor.set_text(Some("20"));
        apply();
        assert_eq!(applied.borrow().font_size, 20.0);
        let presets = named("Text presets").cast::<DropDownButton>().unwrap();
        let menu = presets.flyout().unwrap().cast::<MenuFlyout>().unwrap();
        let small = Control::from_boxed(&menu.items().get_at(0).unwrap())
            .unwrap()
            .cast::<MenuItem>()
            .unwrap();
        small.raise_event(&ferroui_base::interactivity::RoutedEventArgs::with_event(
            MenuItem::click_event(),
        ));
        assert_eq!(preview.font_size(), 14.0);
        assert!((preview.line_height() - 19.6).abs() < 0.001);
        assert_eq!(
            applied.borrow().font_size,
            20.0,
            "preset is a draft until Apply"
        );
        apply();
        assert_eq!(applied.borrow().font_size, 14.0);
        assert_eq!(applied.borrow().line_height, 1.4);
        size_editor.set_text(Some("bad draft"));
        small.raise_event(&ferroui_base::interactivity::RoutedEventArgs::with_event(
            MenuItem::click_event(),
        ));
        assert_eq!(size_editor.text().as_deref(), Some("14"));
        let reset = named("Reset theme & type").cast::<Button>().unwrap();
        let reduced = named("Reduce motion").cast::<CheckBox>().unwrap();
        reduced.set_is_checked(Some(true));
        size_editor.set_text(Some("invalid"));
        reset.raise_event(&ferroui_base::interactivity::RoutedEventArgs::with_event(
            Button::click_event(),
        ));
        assert_eq!(applied.borrow().font_size, 14.0, "reset must stay a draft");
        assert_eq!(size_editor.text().as_deref(), Some("16"));
        assert_eq!(preview.font_size(), 16.0);
        shell.settle();
        apply();
        assert_eq!(
            *applied.borrow(),
            Preferences {
                reduced_motion: true,
                ..Default::default()
            }
        );
        window.close();
    }
}
