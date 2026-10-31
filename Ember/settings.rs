//! Touch-sized preference and reading-data editor. Changes are committed through
//! the owner so persistence failures remain visible to the user.
use crate::widgets::{
    action, combo, disclosure, field_pair, form_button as button, label as text, section, stack,
};
use ember_core::{MuteRules, Preferences, ReaderState, ThemeMode};
use ferroui_base::layout::HorizontalAlignment;
use ferroui_base::{BoxedValue, Ref, Thickness};
use ferroui_controls::automation::{AutomationLiveSetting, AutomationProperties};
use ferroui_controls::{
    CheckBox, Control, NumberBox, NumberBoxSpinButtonPlacementMode, ScrollViewer, StackPanel,
    TextBlock, TextBox,
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

#[derive(Clone)]
pub enum SettingsAction {
    Preferences(Preferences),
    Mutes(MuteRules),
    Export(Rc<dyn Fn(&str)>),
    Import(String),
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
    data.children().add(text(
        "Import replaces the reading data on this device.",
        12.0,
        true,
    ));
    let import_validation = feedback(&data);
    let callback = on_action.clone();
    let import = action("Import this backup", "download", move || {
        let value = json.text().unwrap_or_default();
        if value.trim().is_empty() {
            show_error(&import_validation, "Paste a backup first.");
        } else {
            import_validation.set_is_visible(false);
            callback(SettingsAction::Import(value));
        }
    });
    import.set_horizontal_alignment(HorizontalAlignment::Left);
    data.children().add(import);

    let maintenance = disclosure(&root, "Clear local data", false);
    maintenance.children().add(text("Clear the cache to remove downloaded stories and discussions. Bookmarks and preferences are kept.", 13.0, true));
    let clear = button("Clear cached content");
    let callback = on_action.clone();
    clear.click(move |_, _| callback(SettingsAction::ClearCache));
    maintenance.children().add(clear);
    maintenance.children().add(text(
        "Clear history to remove reading positions, read markers, and recent searches.",
        13.0,
        true,
    ));
    let history = button("Clear reading history");
    history.click(move |_, _| on_action(SettingsAction::ClearHistory));
    maintenance.children().add(history);
    let scroll = ScrollViewer::new();
    scroll.set_content(Some(Control::boxed(root)));
    scroll.upcast()
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
