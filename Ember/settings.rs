//! Touch-sized preference and reading-data editor. Changes are committed through
//! the owner so persistence failures remain visible to the user.
use crate::widgets::{
    action, combo, disclosure, field_pair, form_button as button, label as text, section, stack,
};
use ember_core::{MuteRules, Preferences, ReaderState, ThemeMode};
use ferroui_base::input::{InputElement, Key, KeyEventArgs};
use ferroui_base::layout::{HorizontalAlignment, VerticalAlignment};
use ferroui_base::{BoxedValue, Ref, Thickness, WeakRef};
use ferroui_controls::automation::{AutomationLiveSetting, AutomationProperties};
use ferroui_controls::{
    Button, CheckBox, Control, NumberBox, NumberBoxSpinButtonPlacementMode, ScrollViewer,
    StackPanel, TextBlock, TextBox, WrapPanel,
};
use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

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
    ClearCache,
    ClearHistory,
}

pub fn build(state: &ReaderState, on_action: Rc<dyn Fn(SettingsAction)>) -> Ref<Control> {
    let root = StackPanel::new();
    root.set_spacing(10.0);
    root.set_margin(Thickness::uniform(16.0));
    root.set_max_width(720.0);
    root.set_horizontal_alignment(HorizontalAlignment::Stretch);

    let appearance = section(&root, "Appearance & reading");
    let (theme_field, accent_field) = field_pair(&appearance);
    let theme = combo(
        &theme_field,
        "Theme",
        &["Follow system", "Light", "Dark", "Custom"],
        match state.preferences.theme {
            ThemeMode::System => 0,
            ThemeMode::Light => 1,
            ThemeMode::Dark => 2,
            ThemeMode::Custom => 3,
        },
    );
    let accent = field(
        &accent_field,
        "Accent color",
        &state.preferences.accent,
        "#FF6B35",
        false,
    );
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
    appearance
        .children()
        .add(text("Reading preview", 12.0, true));
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
    appearance.children().add(apply.clone());
    let current = RefCell::new(state.preferences.clone());
    let callback = on_action.clone();
    apply.click(move |_, _| {
        let result = (|| -> Result<Preferences, String> {
            let mut p = current.borrow().clone();
            p.theme = match theme.selected_index() {
                1 => ThemeMode::Light,
                2 => ThemeMode::Dark,
                3 => ThemeMode::Custom,
                _ => ThemeMode::System,
            };
            let accent = accent.text().unwrap_or_default();
            if !valid_color(accent.trim()) {
                return Err("Use a six-digit hex accent, such as #FF6B35.".into());
            }
            p.accent = accent.trim().to_owned();
            let background = background.text().unwrap_or_default();
            let background = background.trim();
            if p.theme == ThemeMode::Custom && !background.is_empty() && !valid_color(background) {
                return Err("Use a six-digit hex background or leave it empty.".into());
            }
            if p.theme == ThemeMode::Custom {
                p.custom_background = (!background.is_empty()).then(|| background.to_owned());
            }
            p.font_size = reading_value(&font_size, "Font size")?;
            p.line_height = reading_value(&line_height, "Line spacing")?;
            p.cache_max_items = cache_limit
                .text()
                .unwrap_or_default()
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|v| (100..=100_000).contains(v))
                .ok_or("Cache size must be a whole number between 100 and 100000.")?;
            p.compact = compact.is_checked() == Some(true);
            p.reduced_motion = motion.is_checked() == Some(true);
            p.show_dead = show_dead.is_checked() == Some(true);
            p.hide_read = hide_read.is_checked() == Some(true);
            Ok(p)
        })();
        match result {
            Ok(p) => {
                validation.set_is_visible(false);
                current.replace(p.clone());
                callback(SettingsAction::Preferences(p));
            }
            Err(error) => show_error(&validation, &error),
        }
    });

    let filters = disclosure(&root, "Muted domains, authors & keywords", false);
    filters.children().add(text("Separate entries with commas or new lines. Muting a domain includes its subdomains. Usernames are case-sensitive.", 13.0, true));
    let domains = field(
        &filters,
        "Muted domains",
        &join(&state.mutes.domains),
        "example.com",
        true,
    );
    let users = field(
        &filters,
        "Muted authors",
        &join(&state.mutes.users),
        "username",
        true,
    );
    let keywords = field(
        &filters,
        "Muted title keywords",
        &join(&state.mutes.keywords),
        "Words or phrases",
        true,
    );
    let save_filters = button("Save filters");
    filters.children().add(save_filters.clone());
    let callback = on_action.clone();
    save_filters.click(move |_, _| {
        callback(SettingsAction::Mutes(MuteRules {
            domains: split(&domains.text().unwrap_or_default()),
            users: split(&users.text().unwrap_or_default()),
            keywords: split(&keywords.text().unwrap_or_default()),
        }))
    });

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
        "Current backup JSON",
        "",
        "",
        16 * 1024 * 1024,
        true,
    );
    output.set_is_read_only(true);
    let callback = on_action.clone();
    let show_backup = backup.clone();
    let export = action("Copy backup", "copy", move || {
        let output = output.downgrade();
        let panel = show_backup.downgrade();
        callback(SettingsAction::Export(Rc::new(move |json| {
            if let (Some(output), Some(panel)) = (output.upgrade(), panel.upgrade()) {
                output.set_text(Some(json));
                panel.set_is_visible(true);
            }
        })));
    });
    export.set_horizontal_alignment(HorizontalAlignment::Left);
    data.children().add(export);
    data.children().add(backup);
    let json = crate::widgets::field(
        &data,
        "Reading data JSON",
        "",
        "Paste an Ember backup to restore",
        16 * 1024 * 1024,
        true,
    );
    let review = Rc::new(RefCell::new(None::<PendingReview>));
    let edited = review.clone();
    json.property_changed(move |e| {
        if e.property() == TextBox::text_property().as_property()
            && edited
                .borrow()
                .as_ref()
                .is_some_and(|review| matches!(review.action, SettingsAction::Import(_)))
        {
            finish_review(&edited, false);
        }
    });
    reviewed_action(
        &data,
        &review,
        ("Import this backup", "download"),
        "Replace reading data",
        move || {
            let value = json.text().unwrap_or_default();
            if value.trim().is_empty() {
                return Err("Paste an Ember backup first.".to_owned());
            }
            let state = ReaderState::import_json(&value).map_err(|error| error.to_string())?;
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

    let maintenance = disclosure(&root, "Clear local data", false);
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
    scroll.upcast()
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
        }
    }
    Some(pending.action)
}

fn reviewed_action(
    parent: &StackPanel,
    review: &Rc<RefCell<Option<PendingReview>>>,
    (label, icon): (&str, &str),
    confirm_label: &str,
    prepare: impl Fn() -> Result<(String, SettingsAction), String> + 'static,
    on_action: &Rc<dyn Fn(SettingsAction)>,
) {
    let trigger = button(label);
    trigger.classes().add("quiet");
    trigger.set_content(Some(Control::boxed(crate::widgets::nav_content(
        icon, label, false, false,
    ))));
    parent.children().add(trigger.clone());
    let error = feedback(parent);
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
            Err(message) => show_error(&error, &message),
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
}

fn feedback(parent: &StackPanel) -> Ref<TextBlock> {
    let message = text("", 13.0, false);
    message.set_is_visible(false);
    AutomationProperties::set_live_setting(&message, AutomationLiveSetting::Polite);
    parent.children().add(message.clone());
    message
}

fn show_error(message: &TextBlock, value: &str) {
    message.set_text(Some(value));
    message.set_is_visible(true);
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
fn valid_color(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
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
    input.set_numeric_value(value);
    input.set_spin_button_placement_mode(NumberBoxSpinButtonPlacementMode::Inline);
    AutomationProperties::set_name(&input, Some(label));
    parent.children().add(input.clone());
    input
}

fn reading_value(input: &NumberBox, label: &str) -> Result<f64, String> {
    if input.validate_input() && input.value().is_finite() {
        Ok(input.value())
    } else {
        Err(format!(
            "{label} must be between {} and {}.",
            input.minimum(),
            input.maximum()
        ))
    }
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
    use ferroui_controls::{ComboBox, Window};
    use sample_testing::Shell;

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
    fn reading_steps_preview_and_apply_without_losing_custom_background() {
        let shell = Shell::start(&crate::SAMPLE, 720.0, 1120.0, || crate::App::new().upcast());
        let applied = Rc::new(RefCell::new(Preferences::default()));
        let result = applied.clone();
        let form = build(
            &ReaderState::default(),
            Rc::new(move |action| {
                if let SettingsAction::Preferences(prefs) = action {
                    result.replace(prefs);
                }
            }),
        );
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
        window.close();
    }
}
