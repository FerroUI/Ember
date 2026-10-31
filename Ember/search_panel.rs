//! Search and public-profile forms. Filters are data, never interpolated tags.
use crate::widgets::{
    combo, disclosure, field_pair, form_button as button, label as text, section,
};
use ember_core::{validate_username, SearchKind, SearchQuery, SearchSort};
use ferroui_base::input::{InputElement, Key, KeyEventArgs, KeyModifiers};
use ferroui_base::utilities::DateTime;
use ferroui_base::{Ref, Thickness};
use ferroui_controls::automation::{AutomationLiveSetting, AutomationProperties};
use ferroui_controls::{Button, Control, ScrollViewer, StackPanel, TextBox};
use std::rc::Rc;

/// The longest text of a search field.
const FIELD_LENGTH: i32 = 2048;

fn field(parent: &StackPanel, label: &str, value: &str, placeholder: &str) -> Ref<TextBox> {
    crate::widgets::field(parent, label, value, placeholder, FIELD_LENGTH, false)
}

pub fn build(
    query: &SearchQuery,
    recent_searches: &[String],
    on_search: Rc<dyn Fn(SearchQuery)>,
    on_profile: Rc<dyn Fn(String)>,
) -> Ref<Control> {
    let root = StackPanel::new();
    root.set_spacing(10.0);
    root.set_margin(Thickness::uniform(16.0));
    root.set_max_width(720.0);
    let search = section(&root, "Search Hacker News");
    let words = field(
        &search,
        "Search terms",
        &query.query,
        "Stories, comments, or a phrase",
    );
    let weak_words = words.downgrade();
    words.loaded(move |_, _| {
        if let Some(input) = weak_words.upgrade() {
            input.focus();
        }
    });
    let (scope, order) = field_pair(&search);
    let kind = combo(
        &scope,
        "Search in",
        &[
            "Stories",
            "Comments",
            "Everything",
            "Ask HN",
            "Show HN",
            "Polls",
        ],
        match query.kind {
            SearchKind::Stories => 0,
            SearchKind::Comments => 1,
            SearchKind::All => 2,
            SearchKind::Ask => 3,
            SearchKind::Show => 4,
            SearchKind::Polls => 5,
        },
    );
    let sort = combo(
        &order,
        "Sort by",
        &["Most relevant", "Newest first"],
        if query.sort == SearchSort::Recent {
            1
        } else {
            0
        },
    );
    let filters = disclosure(&search, "More filters", has_advanced_filters(query));
    let author = field(
        &filters,
        "Author",
        query.author.as_deref().unwrap_or(""),
        "Any author",
    );
    let (score, replies) = field_pair(&filters);
    let points = field(
        &score,
        "Minimum points",
        &query.min_points.map(|v| v.to_string()).unwrap_or_default(),
        "Any score",
    );
    let comments = field(
        &replies,
        "Minimum comments",
        &query
            .min_comments
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "Any count",
    );
    let time = combo(
        &filters,
        "Published",
        &[
            "Any time",
            "Past 24 hours",
            "Past week",
            "Past month",
            "Past year",
            "Custom date range",
        ],
        if query.since.is_some() || query.until.is_some() {
            5
        } else {
            0
        },
    );
    let dates = StackPanel::new();
    dates.set_spacing(6.0);
    dates.set_is_visible(time.selected_index() == 5);
    let (start, end) = field_pair(&dates);
    let from_date = field(&start, "From date", &format_date(query.since), "YYYY-MM-DD");
    let through_date = field(
        &end,
        "Through date",
        &format_date(query.until),
        "YYYY-MM-DD",
    );
    dates.children().add(text(
        "Dates are inclusive, in UTC. Either field can be empty.",
        12.0,
        true,
    ));
    let weak_dates = dates.downgrade();
    filters.children().add(dates);
    let weak_time = time.downgrade();
    time.selection_changed(move |_, _| {
        if let (Some(dates), Some(time)) = (weak_dates.upgrade(), weak_time.upgrade()) {
            dates.set_is_visible(time.selected_index() == 5);
        }
    });
    let message = text("", 13.0, false);
    message.set_is_visible(false);
    AutomationProperties::set_live_setting(&message, AutomationLiveSetting::Polite);
    search.children().add(message.clone());
    let submit = button("Search");
    submit.classes().add("accent");
    search.children().add(submit.clone());
    if !recent_searches.is_empty() {
        let recent = disclosure(&root, "Recent searches", false);
        for term in recent_searches.iter().take(5) {
            let input = words.downgrade();
            let suggestion = button(term);
            let term = term.clone();
            suggestion.click(move |_, _| {
                if let Some(input) = input.upgrade() {
                    input.set_text(Some(&term));
                    input.focus();
                }
            });
            recent.children().add(suggestion);
        }
    }
    let reset = button("Clear filters");
    filters.children().add(reset.clone());
    {
        let fields = [
            author.clone(),
            points.clone(),
            comments.clone(),
            from_date.clone(),
            through_date.clone(),
        ];
        let choices = [kind.clone(), sort.clone(), time.clone()];
        let message = message.clone();
        reset.click(move |_, _| {
            for field in &fields {
                field.set_text(Some(""));
            }
            for choice in &choices {
                choice.set_selected_index(0);
            }
            message.set_is_visible(false);
        });
    }
    let original = query.clone();
    bind_submit(&search, &submit, move || {
        let result = (|| -> Result<SearchQuery, String> {
            let mut q = original.clone();
            q.page = 0;
            q.query = words.text().unwrap_or_default().trim().to_owned();
            q.kind = match kind.selected_index() {
                1 => SearchKind::Comments,
                2 => SearchKind::All,
                3 => SearchKind::Ask,
                4 => SearchKind::Show,
                5 => SearchKind::Polls,
                _ => SearchKind::Stories,
            };
            q.sort = if sort.selected_index() == 1 {
                SearchSort::Recent
            } else {
                SearchSort::Relevance
            };
            let value = author.text().unwrap_or_default();
            let value = value.trim();
            q.author = if value.is_empty() {
                None
            } else {
                validate_username(value).map_err(|e| e.to_string())?;
                Some(value.to_owned())
            };
            q.min_points = optional_number(&points, "Minimum points")?;
            q.min_comments = optional_number(&comments, "Minimum comments")?;
            let seconds = match time.selected_index() {
                1 => Some(86400),
                2 => Some(7 * 86400),
                3 => Some(30 * 86400),
                4 => Some(365 * 86400),
                _ => None,
            };
            if let Some(seconds) = seconds {
                q.since = Some(crate::services::now().saturating_sub(seconds));
                q.until = None;
            } else if time.selected_index() == 5 {
                q.since =
                    date_boundary(&from_date.text().unwrap_or_default(), original.since, false)?;
                q.until = date_boundary(
                    &through_date.text().unwrap_or_default(),
                    original.until,
                    true,
                )?;
                if q.since.is_none() && q.until.is_none() {
                    return Err("Enter a start or end date, or choose Any time.".into());
                }
            } else {
                q.since = None;
                q.until = None;
            }
            q.url().map_err(|e| e.to_string())?;
            Ok(q)
        })();
        match result {
            Ok(q) => on_search(q),
            Err(error) => {
                message.set_text(Some(&error));
                message.set_is_visible(true);
            }
        }
    });
    let people = disclosure(&root, "Find a person", false);
    let username = field(
        &people,
        "Hacker News username",
        "",
        "Case-sensitive username",
    );
    username.set_max_length(64);
    let message = text("", 13.0, false);
    message.set_is_visible(false);
    AutomationProperties::set_live_setting(&message, AutomationLiveSetting::Polite);
    people.children().add(message.clone());
    let open = button("Open profile");
    people.children().add(open.clone());
    bind_submit(&people, &open, move || {
        let name = username.text().unwrap_or_default();
        match validate_username(name.trim()) {
            Ok(()) => on_profile(name.trim().to_owned()),
            Err(error) => {
                message.set_text(Some(&error.to_string()));
                message.set_is_visible(true);
            }
        }
    });
    root.children()
        .add(text("Search powered by Algolia", 12.0, true));
    let scroll = ScrollViewer::new();
    scroll.set_content(Some(Control::boxed(root)));
    scroll.upcast()
}

/// Restrict Enter to text fields in this form. Both forms share the same top level.
fn bind_submit(scope: &StackPanel, button: &Button, submit: impl Fn() + 'static) {
    let submit = Rc::new(submit);
    let pointer = submit.clone();
    button.click(move |_, _| pointer());
    scope.add_handler(
        InputElement::key_down_event(),
        move |_, e: &KeyEventArgs| {
            if e.key == Key::Enter
                && e.key_modifiers == KeyModifiers::empty()
                && e.source()
                    .is_some_and(|source| source.cast::<TextBox>().is_some())
            {
                e.set_handled(true);
                submit();
            }
        },
    );
}

fn has_advanced_filters(query: &SearchQuery) -> bool {
    query.author.is_some()
        || query.min_points.is_some()
        || query.min_comments.is_some()
        || query.since.is_some()
        || query.until.is_some()
}

/// Reopening a relative-time query must preserve the original hour and minute until edited.
fn date_boundary(
    value: &str,
    original: Option<u64>,
    end_of_day: bool,
) -> Result<Option<u64>, String> {
    if original.is_some() && value.trim() == format_date(original) {
        return Ok(original);
    }
    Ok(parse_date(value)?.map(|day| if end_of_day { day + 86399 } else { day }))
}

fn optional_number(input: &TextBox, label: &str) -> Result<Option<u64>, String> {
    let value = input.text().unwrap_or_default();
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse::<u64>()
        .map(Some)
        .map_err(|_| format!("{label} must be zero or a positive whole number."))
}

fn format_date(seconds: Option<u64>) -> String {
    seconds
        .and_then(|s| i64::try_from(s).ok())
        .and_then(|s| s.checked_mul(10_000_000))
        .and_then(|ticks| DateTime::UNIX_EPOCH.try_add_ticks(ticks))
        .map(|date| format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day()))
        .unwrap_or_default()
}
fn parse_date(value: &str) -> Result<Option<u64>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let invalid = || "Use a valid date in YYYY-MM-DD format, from 1970 onward.".to_owned();
    if value.len() != 10
        || !value.is_ascii()
        || value.as_bytes()[4] != b'-'
        || value.as_bytes()[7] != b'-'
    {
        return Err(invalid());
    }
    let year = value[0..4].parse::<i32>().map_err(|_| invalid())?;
    let month = value[5..7].parse::<i32>().map_err(|_| invalid())?;
    let day = value[8..10].parse::<i32>().map_err(|_| invalid())?;
    let date = DateTime::try_new(year, month, day)
        .filter(|d| d.ticks() >= DateTime::UNIX_EPOCH.ticks())
        .ok_or_else(invalid)?;
    Ok(Some(
        ((date.ticks() - DateTime::UNIX_EPOCH.ticks()) / 10_000_000) as u64,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advanced_search_resets_filters_and_enter_submits_only_its_form() {
        use ferroui_controls::{ComboBox, Expander, Window};
        use sample_testing::Shell;
        use std::cell::RefCell;

        let shell = Shell::start(&crate::SAMPLE, 720.0, 1120.0, || crate::App::new().upcast());
        let last = Rc::new(RefCell::new(None));
        let result = last.clone();
        let search = build(
            &SearchQuery::default(),
            &[],
            Rc::new(move |q| {
                result.replace(Some(q));
            }),
            Rc::new(|_| panic!("Enter in search submitted the profile form")),
        );
        let window = Window::new();
        window.set_width(720.0);
        window.set_height(1120.0);
        window.set_content(Some(Control::boxed(&search)));
        window.show();
        shell.settle();

        let named = |name: &str| {
            window
                .get_visual_descendants()
                .filter_map(|target| target.cast::<Control>())
                .find(|target| AutomationProperties::get_name(target).as_deref() == Some(name))
                .unwrap_or_else(|| panic!("missing {name}"))
        };
        let click = |target: &Control| {
            shell.click(0, Shell::bounds_of(&window, target).center());
            shell.settle();
        };
        let input = named("Search terms").cast::<TextBox>().unwrap();
        let enter = || {
            let mut key = KeyEventArgs::new();
            key.key = Key::Enter;
            key.set_routed_event(Some(InputElement::key_down_event()));
            input.raise_event(&key);
            shell.settle();
            assert!(key.handled());
        };
        let scope = named("Search in");
        let order = named("Sort by");
        assert_eq!(
            Shell::bounds_of(&window, &scope).y,
            Shell::bounds_of(&window, &order).y
        );
        search.set_width(330.0);
        shell.settle();
        assert!(Shell::bounds_of(&window, &order).y >= Shell::bounds_of(&window, &scope).bottom());

        let filters = named("More filters").cast::<Expander>().unwrap();
        assert!(!filters.is_expanded());
        click(&filters);
        assert!(filters.is_expanded());
        input.set_text(Some("  compact UI  "));
        let author = named("Author").cast::<TextBox>().unwrap();
        author.set_text(Some("ada"));
        let published = named("Published").cast::<ComboBox>().unwrap();
        published.set_selected_index(5);
        shell.settle();
        named("From date")
            .cast::<TextBox>()
            .unwrap()
            .set_text(Some("2024-02-30"));
        enter();
        assert!(last.borrow().is_none(), "invalid date submitted");
        named("From date")
            .cast::<TextBox>()
            .unwrap()
            .set_text(Some("2024-02-29"));
        enter();
        let q = last
            .borrow_mut()
            .take()
            .expect("Enter must submit the form");
        assert_eq!(q.query, "compact UI");
        assert_eq!(q.author.as_deref(), Some("ada"));
        assert_eq!(q.since, parse_date("2024-02-29").unwrap());
        search.set_width(680.0);
        shell.settle();
        click(&named("Clear filters"));
        enter();
        assert_eq!(
            last.borrow_mut().take().unwrap(),
            SearchQuery {
                query: "compact UI".into(),
                ..Default::default()
            }
        );
        window.close();
    }

    #[test]
    fn reopening_a_relative_range_preserves_its_cutoff_until_edited() {
        assert_eq!(
            date_boundary("1970-01-01", Some(3600), false).unwrap(),
            Some(3600)
        );
        assert_eq!(
            date_boundary("1970-01-02", Some(3600), false).unwrap(),
            Some(86400)
        );
        assert_eq!(
            date_boundary("1970-01-02", None, true).unwrap(),
            Some(172799)
        );
        assert_eq!(date_boundary("", Some(3600), true).unwrap(), None);
    }
    #[test]
    fn custom_dates_validate_calendar_boundaries_and_round_trip() {
        assert_eq!(parse_date("1970-01-01").unwrap(), Some(0));
        assert!(parse_date("2023-02-29").is_err());
        assert!(parse_date("1969-12-31").is_err());
        assert!(parse_date("2024-13-01").is_err());
        assert!(parse_date("2024-2-03").is_err());
        let leap = parse_date("2024-02-29").unwrap();
        assert_eq!(format_date(leap), "2024-02-29");
        assert_eq!(parse_date(" ").unwrap(), None);
    }
}
