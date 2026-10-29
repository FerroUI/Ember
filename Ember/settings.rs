//! Touch-sized preference and reading-data editor. Changes are committed through
//! the owner so persistence failures remain visible to the user.
use crate::widgets::{combo, form_button as button, label as text, section};
use ember_core::{MuteRules, Preferences, ReaderState, ThemeMode};
use ferroui_base::layout::HorizontalAlignment;
use ferroui_base::{BoxedValue, Ref, Thickness};
use ferroui_controls::automation::AutomationProperties;
use ferroui_controls::{CheckBox, Control, ScrollViewer, StackPanel, TextBox};
use std::{collections::BTreeSet, rc::Rc};

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
    Export,
    Import(String),
    ClearCache,
    ClearHistory,
}

pub fn build(state: &ReaderState, on_action: Rc<dyn Fn(SettingsAction)>) -> Ref<Control> {
    let root = StackPanel::new();
    root.set_spacing(20.0);
    root.set_margin(Thickness::uniform(22.0));
    root.set_max_width(760.0);
    root.set_horizontal_alignment(HorizontalAlignment::Stretch);
    root.children().add(text("Make Ember yours", 28.0, false));
    root.children().add(text(
        "A quieter reading space, tuned to the way you browse.",
        14.0,
        true,
    ));

    let appearance = section(&root, "Appearance & reading");
    let theme = combo(
        &appearance,
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
        &appearance,
        "Accent color",
        &state.preferences.accent,
        "#FF6B35",
        false,
    );
    let background = field(
        &appearance,
        "Custom background",
        state.preferences.custom_background.as_deref().unwrap_or(""),
        "Optional, for example #172126",
        false,
    );
    let font_size = field(
        &appearance,
        "Reading font size",
        &state.preferences.font_size.to_string(),
        "12–32",
        false,
    );
    let line_height = field(
        &appearance,
        "Line spacing",
        &state.preferences.line_height.to_string(),
        "1.1–2.2",
        false,
    );
    let compact = toggle(
        &appearance,
        "Compact story cards",
        state.preferences.compact,
    );
    let motion = toggle(
        &appearance,
        "Reduce animation",
        state.preferences.reduced_motion,
    );
    let show_dead = toggle(
        &appearance,
        "Show flagged comments",
        state.preferences.show_dead,
    );
    let hide_read = toggle(
        &appearance,
        "Hide stories I have read",
        state.preferences.hide_read,
    );
    let cache_limit = field(
        &appearance,
        "Maximum cached items",
        &state.preferences.cache_max_items.to_string(),
        "100–100000",
        false,
    );
    let validation = text("", 13.0, true);
    appearance.children().add(validation.clone());
    let apply = button("Apply preferences");
    apply.classes().add("accent");
    appearance.children().add(apply.clone());
    let previous = state.preferences.clone();
    let callback = on_action.clone();
    apply.click(move |_, _| {
        let result = (|| -> Result<Preferences, String> {
            let mut p = previous.clone();
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
            if !background.is_empty() && !valid_color(background) {
                return Err("Use a six-digit hex background or leave it empty.".into());
            }
            p.custom_background = (!background.is_empty()).then(|| background.to_owned());
            p.font_size = parse_number(&font_size, "Font size", 12.0, 32.0)?;
            p.line_height = parse_number(&line_height, "Line spacing", 1.1, 2.2)?;
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
                validation.set_text(Some("Applying preferences…"));
                callback(SettingsAction::Preferences(p));
            }
            Err(error) => validation.set_text(Some(&error)),
        }
    });

    let filters = section(&root, "Keep your feed focused");
    filters.children().add(text("Separate entries with commas or new lines. Domain filters include subdomains; usernames are case-sensitive.", 13.0, true));
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

    let data = section(&root, "Your reading data");
    data.children().add(text("Bookmarks, history, filters, and preferences stay on this device. Export a backup or paste a previous backup to restore it.", 13.0, true));
    let json = field(
        &data,
        "Reading data JSON",
        "",
        "Paste an Ember backup to import",
        true,
    );
    json.set_min_height(180.0);
    json.set_max_height(320.0);
    // The limit is raised before the backup is put in: the field's own limit would cut it.
    json.set_max_length(16 * 1024 * 1024);
    json.set_text(Some(&state.export_json().unwrap_or_default()));
    let export = button("Copy backup");
    let callback = on_action.clone();
    export.click(move |_, _| callback(SettingsAction::Export));
    data.children().add(export);
    let import = button("Import this backup");
    let callback = on_action.clone();
    let import_validation = text("", 13.0, true);
    let message = import_validation.clone();
    import.click(move |_, _| {
        let value = json.text().unwrap_or_default();
        match ReaderState::import_json(&value) {
            Ok(_) => {
                message.set_text(Some("Importing backup…"));
                callback(SettingsAction::Import(value));
            }
            Err(error) => message.set_text(Some(&error.to_string())),
        }
    });
    data.children().add(import);
    data.children().add(import_validation);
    let clear = button("Clear cached content");
    let callback = on_action.clone();
    clear.click(move |_, _| callback(SettingsAction::ClearCache));
    data.children().add(clear);
    let history = button("Clear reading history");
    history.click(move |_, _| on_action(SettingsAction::ClearHistory));
    data.children().add(history);
    data.children().add(text("Clearing cached content keeps your preferences and bookmarks. Downloaded stories will need to be downloaded again.", 12.0, true));
    let scroll = ScrollViewer::new();
    scroll.set_content(Some(Control::boxed(root)));
    scroll.upcast()
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
fn parse_number(input: &TextBox, label: &str, minimum: f64, maximum: f64) -> Result<f64, String> {
    input
        .text()
        .unwrap_or_default()
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= minimum && *value <= maximum)
        .ok_or_else(|| format!("{label} must be between {minimum} and {maximum}."))
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
