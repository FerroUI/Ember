//! Search and public-profile forms. Filters are data, never interpolated tags.
use crate::widgets::{combo, form_button as button, label as text, section};
use ember_core::{validate_username, SearchKind, SearchQuery, SearchSort};
use ferroui_base::utilities::DateTime;
use ferroui_base::{Ref, Thickness};
use ferroui_controls::{Control, ScrollViewer, StackPanel, TextBox};
use std::rc::Rc;

/// The longest text of a search field.
const FIELD_LENGTH: i32 = 2048;

fn field(parent: &StackPanel, label: &str, value: &str, placeholder: &str) -> Ref<TextBox> {
    crate::widgets::field(parent, label, value, placeholder, FIELD_LENGTH, false)
}

pub fn build(
    query: &SearchQuery,
    on_search: Rc<dyn Fn(SearchQuery)>,
    on_profile: Rc<dyn Fn(String)>,
) -> Ref<Control> {
    let root = StackPanel::new();
    root.set_spacing(20.0);
    root.set_margin(Thickness::uniform(22.0));
    root.set_max_width(760.0);
    root.children()
        .add(text("Follow your curiosity", 28.0, false));
    root.children().add(text(
        "Search years of ideas, launches, and conversations.",
        14.0,
        true,
    ));
    let search = section(&root, "Search Hacker News");
    let words = field(
        &search,
        "Search terms",
        &query.query,
        "Ideas, projects, or a phrase…",
    );
    let kind = combo(
        &search,
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
        &search,
        "Sort by",
        &["Most relevant", "Newest first"],
        if query.sort == SearchSort::Recent {
            1
        } else {
            0
        },
    );
    let author = field(
        &search,
        "Author",
        query.author.as_deref().unwrap_or(""),
        "Any author",
    );
    let points = field(
        &search,
        "Minimum points",
        &query.min_points.map(|v| v.to_string()).unwrap_or_default(),
        "Any score",
    );
    let comments = field(
        &search,
        "Minimum comments",
        &query
            .min_comments
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "Any discussion size",
    );
    let time = combo(
        &search,
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
    dates.set_spacing(12.0);
    dates.set_is_visible(time.selected_index() == 5);
    let from_date = field(&dates, "From date", &format_date(query.since), "YYYY-MM-DD");
    let through_date = field(
        &dates,
        "Through date",
        &format_date(query.until),
        "YYYY-MM-DD",
    );
    dates.children().add(text(
        "Dates use UTC and include the full end date. Either boundary can be left empty.",
        12.0,
        true,
    ));
    search.children().add(dates.clone());
    let weak_dates = dates.downgrade();
    let weak_time = time.downgrade();
    time.selection_changed(move |_, _| {
        if let (Some(dates), Some(time)) = (weak_dates.upgrade(), weak_time.upgrade()) {
            dates.set_is_visible(time.selected_index() == 5);
        }
    });
    let message = text("", 13.0, true);
    search.children().add(message.clone());
    let submit = button("Search");
    submit.classes().add("accent");
    search.children().add(submit.clone());
    let original = query.clone();
    submit.click(move |_, _| {
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
                q.since = parse_date(&from_date.text().unwrap_or_default())?;
                q.until = parse_date(&through_date.text().unwrap_or_default())?
                    .map(|value| value + 86399);
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
            Ok(q) => {
                message.set_text(Some("Searching…"));
                on_search(q);
            }
            Err(error) => message.set_text(Some(&error)),
        }
    });
    search.children().add(text("Search is powered by Algolia. Story lists and conversations come directly from Hacker News.", 12.0, true));

    let people = section(&root, "Find a person");
    people.children().add(text(
        "Read a public profile and explore their submissions.",
        13.0,
        true,
    ));
    let username = field(
        &people,
        "Hacker News username",
        "",
        "Case-sensitive username",
    );
    let message = text("", 13.0, true);
    people.children().add(message.clone());
    let open = button("Open profile");
    people.children().add(open.clone());
    open.click(move |_, _| {
        let name = username.text().unwrap_or_default();
        let name = name.trim();
        match validate_username(name) {
            Ok(()) => on_profile(name.to_owned()),
            Err(error) => message.set_text(Some(&error.to_string())),
        }
    });
    let scroll = ScrollViewer::new();
    scroll.set_content(Some(Control::boxed(root)));
    scroll.upcast()
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
        .map_err(|_| format!("{label} must be a positive whole number, or leave it empty."))
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
