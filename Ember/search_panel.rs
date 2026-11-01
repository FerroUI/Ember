//! Search and public-profile forms. Filters are data, never interpolated tags.
use crate::widgets::{
    combo, disclosure_with_toggle, field_pair, form_button as button, icon_button, label as text,
    section, FormValidation,
};
use ember_core::{validate_username, HnDestination, SearchKind, SearchQuery, SearchSort};
use ferroui_base::input::{InputElement, Key, KeyEventArgs, KeyModifiers};
use ferroui_base::layout::HorizontalAlignment;
use ferroui_base::media::TextTrimming;
use ferroui_base::utilities::DateTime;
use ferroui_base::{Ref, Thickness, Vector};
use ferroui_controls::automation::{AutomationLiveSetting, AutomationProperties};
use ferroui_controls::{
    Button, ColumnDefinitions, Control, Grid, ScrollViewer, StackPanel, TextBox,
};
use std::rc::Rc;

/// The longest text of a search field.
const FIELD_LENGTH: i32 = 2048;

fn field(parent: &StackPanel, label: &str, value: &str, placeholder: &str) -> Ref<TextBox> {
    crate::widgets::field(parent, label, value, placeholder, FIELD_LENGTH, false)
}

/// Back history keeps the form's raw edits, including invalid or unfinished values.
pub(super) struct Draft {
    baseline: SearchQuery,
    fields: Vec<(String, i32, i32)>,
    kind: i32,
    sort: i32,
    period: i32,
    sections: [bool; 3],
    focus: Option<usize>,
    offset: Vector,
}

pub(super) struct Form {
    pub control: Ref<Control>,
    pub capture: Box<dyn Fn() -> Draft>,
}

pub(super) fn build(
    query: &SearchQuery,
    recent_searches: &[String],
    profile_lookup: Option<&str>,
    draft: Option<Draft>,
    forget_search: Rc<dyn Fn(&str)>,
    on_search: Rc<dyn Fn(SearchQuery)>,
    on_open: Rc<dyn Fn(HnDestination)>,
) -> Form {
    let query = draft.as_ref().map_or(query, |draft| &draft.baseline);
    let baseline = query.clone();
    let root = StackPanel::new();
    root.set_spacing(10.0);
    root.set_margin(Thickness::uniform(16.0));
    root.set_max_width(720.0);
    let search = section(&root, "Search Hacker News");
    let words = field(
        &search,
        "Search terms",
        &query.query,
        "Search terms or a Hacker News link",
    );
    words.set_name(Some("SearchTerms".to_owned()));
    if profile_lookup.is_none() && draft.is_none() {
        focus_on_load(&words, false);
    }
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
    let (filters_toggle, filters) =
        disclosure_with_toggle(&search, "More filters", has_advanced_filters(query));
    let author = field(
        &filters,
        "Author",
        query.author.as_deref().unwrap_or(""),
        "Any author",
    );
    let discussion = field(
        &filters,
        "Discussion",
        &query.story_id.map(|id| id.to_string()).unwrap_or_default(),
        "Item ID or Hacker News discussion link",
    );
    let previous_kind = Rc::new(std::cell::Cell::new(None));
    let selector = kind.downgrade();
    let source = discussion.downgrade();
    let saved_kind = previous_kind.clone();
    let constrain_kind = move || {
        let (Some(selector), Some(source)) = (selector.upgrade(), source.upgrade()) else {
            return;
        };
        let scoped = source.text().is_some_and(|value| !value.trim().is_empty());
        if scoped && previous_kind.get().is_none() {
            previous_kind.set(Some(selector.selected_index()));
            selector.set_selected_index(1);
        } else if !scoped {
            if let Some(index) = previous_kind.take() {
                selector.set_selected_index(index);
            }
        }
        selector.set_is_enabled(!scoped);
    };
    constrain_kind();
    discussion.property_changed(move |event| {
        if event.property() == TextBox::text_property().as_property() {
            constrain_kind();
        }
    });
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
    let trigger = submit.downgrade();
    let update_action = move |value: Option<String>| {
        let Some(button) = trigger.upgrade() else {
            return;
        };
        let label = match HnDestination::from_link(value.as_deref().unwrap_or_default()) {
            Some(HnDestination::Discussion(_)) => "Open discussion",
            Some(HnDestination::Profile(_)) => "Open profile link",
            None => "Search",
        };
        button.set_content(Some(Rc::new(label.to_owned())));
        AutomationProperties::set_name(&button, Some(label));
    };
    update_action(words.text());
    let input = words.downgrade();
    words.property_changed(move |event| {
        if event.property() == TextBox::text_property().as_property() {
            if let Some(input) = input.upgrade() {
                update_action(input.text());
            }
        }
    });
    let reset = button("Clear filters");
    filters.children().add(reset.clone());
    author.set_max_length(65);
    let summary = text("", 12.0, true);
    search.children().add(summary.clone());
    let advanced = [
        author.clone(),
        discussion.clone(),
        points.clone(),
        comments.clone(),
    ];
    let values = advanced.each_ref().map(|field| field.downgrade());
    let weak_time = time.downgrade();
    let clear = reset.downgrade();
    let update_summary = Rc::new(move || {
        let (Some(time), Some(clear)) = (weak_time.upgrade(), clear.upgrade()) else {
            return;
        };
        let mut parts = Vec::new();
        for (field, prefix) in values
            .iter()
            .zip(["Author", "Discussion", "Points ≥", "Comments ≥"])
        {
            let Some(field) = field.upgrade() else {
                continue;
            };
            let value = field.text().unwrap_or_default();
            if !value.trim().is_empty() {
                parts.push(format!("{prefix} {}", value.trim()));
            }
        }
        if let Some(period) = match time.selected_index() {
            1 => Some("Past 24 hours"),
            2 => Some("Past week"),
            3 => Some("Past month"),
            4 => Some("Past year"),
            5 => Some("Custom date range"),
            _ => None,
        } {
            parts.push(period.to_owned());
        }
        clear.set_is_enabled(!parts.is_empty());
        summary.set_text(Some(&parts.join(" · ")));
        summary.set_is_visible(!parts.is_empty());
    });
    for field in &advanced {
        let update = update_summary.clone();
        field.property_changed(move |e| {
            if e.property() == TextBox::text_property().as_property() {
                update();
            }
        });
    }
    update_summary();
    time.selection_changed(move |_, _| update_summary());
    {
        let fields = [
            author.clone(),
            discussion.clone(),
            points.clone(),
            comments.clone(),
            from_date.clone(),
            through_date.clone(),
        ];
        let time = time.clone();
        let message = message.clone();
        reset.click(move |_, _| {
            let mut check = FormValidation::default();
            for field in &fields {
                field.set_text(Some(""));
                check.check(field, Ok(()));
            }
            time.set_selected_index(0);
            message.set_is_visible(false);
        });
    }
    let mut fields = vec![
        words.clone(),
        author.clone(),
        discussion.clone(),
        points.clone(),
        comments.clone(),
        from_date.clone(),
        through_date.clone(),
    ];
    let discussion_scope_field = discussion.clone();
    let choices = [kind.clone(), sort.clone(), time.clone()];
    let original = query.clone();
    let recent_input = words.downgrade();
    let open_link = on_open.clone();
    let run: Rc<dyn Fn()> = Rc::new(move || {
        let mut check = FormValidation::default();
        let mut q = original.clone();
        q.page = 0;
        q.query = words.text().unwrap_or_default().trim().to_owned();
        if let Some(destination) = HnDestination::from_link(&q.query) {
            open_link(destination);
            return;
        }
        check.check(
            &words,
            if q.query.len() <= FIELD_LENGTH as usize {
                Ok(())
            } else {
                Err("Search terms exceed the 2,048-byte limit.".into())
            },
        );
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
        let entered = value.trim();
        let value = entered.strip_prefix('@').unwrap_or(entered);
        check.check(
            &author,
            if entered.is_empty() {
                Ok(())
            } else {
                validate_username(value)
                    .map_err(|_| "Author must be a Hacker News username.".to_owned())
            },
        );
        q.author = (!value.is_empty()).then(|| value.to_owned());
        q.story_id = check
            .check(
                &discussion,
                discussion_scope(&discussion.text().unwrap_or_default()),
            )
            .flatten();
        q.min_points = check
            .check(&points, optional_number(&points, "Minimum points"))
            .flatten();
        q.min_comments = check
            .check(&comments, optional_number(&comments, "Minimum comments"))
            .flatten();
        let seconds = match time.selected_index() {
            1 => Some(86400),
            2 => Some(7 * 86400),
            3 => Some(30 * 86400),
            4 => Some(365 * 86400),
            _ => None,
        };
        let custom = time.selected_index() == 5;
        let from = from_date.text().unwrap_or_default();
        let until = through_date.text().unwrap_or_default();
        let start = if !custom {
            Ok(None)
        } else if from.trim().is_empty() && until.trim().is_empty() {
            Err("Enter a start date or an end date.".to_owned())
        } else {
            date_boundary(&from, original.since, false).map_err(|_| {
                "Enter a valid From date in YYYY-MM-DD format, from 1970 onward.".to_owned()
            })
        };
        q.since = check.check(&from_date, start).flatten();
        let end = if custom {
            date_boundary(&until, original.until, true)
                .map_err(|_| {
                    "Enter a valid Through date in YYYY-MM-DD format, from 1970 onward.".to_owned()
                })
                .and_then(|end| {
                    if q.since.zip(end).is_some_and(|(start, end)| start > end) {
                        Err("Through date must be on or after From date.".to_owned())
                    } else {
                        Ok(end)
                    }
                })
        } else {
            Ok(None)
        };
        q.until = check.check(&through_date, end).flatten();
        if let Some(seconds) = seconds {
            q.since = Some(crate::services::now().saturating_sub(seconds));
        }
        if check.finish(&message) {
            on_search(q);
        }
    });
    bind_submit(&search, &submit, run.clone());
    let mut recent_toggle = None;
    if !recent_searches.is_empty() {
        let (toggle, recent) = disclosure_with_toggle(&root, "Recent searches", false);
        recent_toggle = Some(toggle);
        recent.children().add(text("Select a term to edit it, or use its search button to run it with the current filters.", 12.0, true));
        let rows = crate::widgets::stack(2.0);
        recent.children().add(rows.clone());
        let empty = text("No recent searches.", 13.0, true);
        empty.set_is_visible(false);
        recent.children().add(empty.clone());
        let mut listed = std::collections::BTreeSet::new();
        for term in recent_searches
            .iter()
            .take(20)
            .filter(|s| listed.insert(s.as_str()))
        {
            let row = Grid::new();
            row.set_column_definitions(
                ColumnDefinitions::parse("*,Auto,Auto").expect("recent search columns"),
            );
            row.set_column_spacing(4.0);
            let edit = button(term);
            edit.classes().add("quiet");
            edit.set_horizontal_alignment(HorizontalAlignment::Stretch);
            edit.set_horizontal_content_alignment(HorizontalAlignment::Left);
            let title = text(term, 13.0, false);
            title.set_max_lines(1);
            title.set_text_trimming(<dyn TextTrimming>::character_ellipsis());
            edit.set_content(Some(Control::boxed(title)));
            let input = recent_input.clone();
            let value = term.clone();
            edit.click(move |_, _| {
                if let Some(input) = input.upgrade() {
                    input.set_text(Some(&value));
                    input.focus();
                    input.bring_into_view();
                }
            });
            row.children().add(edit);
            let go = icon_button(&format!("Run search for {term}"), "search");
            let input = recent_input.clone();
            let value = term.clone();
            let run = run.clone();
            go.click(move |_, _| {
                if let Some(input) = input.upgrade() {
                    input.set_text(Some(&value));
                    run();
                }
            });
            Grid::set_column(&go, 1);
            row.children().add(go);
            let forget = icon_button(&format!("Remove {term} from recent searches"), "dismiss");
            let input = recent_input.clone();
            let target = row.downgrade();
            let list = rows.downgrade();
            let empty = empty.clone();
            let value = term.clone();
            let remove = forget_search.clone();
            forget.click(move |_, _| {
                let (Some(list), Some(row)) = (list.upgrade(), target.upgrade()) else {
                    return;
                };
                let children = list.children();
                let row = row.upcast::<Control>();
                let Some(index) = children.index_of(&row) else {
                    return;
                };
                remove(&value);
                children.remove(row);
                let remaining = children.count();
                empty.set_is_visible(remaining == 0);
                let next = if remaining == 0 {
                    input.upgrade().map(|field| field.upcast::<Control>())
                } else {
                    children
                        .get(index.min(remaining - 1))
                        .cast::<Grid>()
                        .map(|row| row.children().get(0))
                };
                if let Some(next) = next {
                    next.focus();
                    next.bring_into_view();
                }
            });
            Grid::set_column(&forget, 2);
            row.children().add(forget);
            rows.children().add(row);
        }
    }
    let (people_toggle, people) =
        disclosure_with_toggle(&root, "Find a person", profile_lookup.is_some());
    let username = field(
        &people,
        "Hacker News username",
        profile_lookup.unwrap_or(""),
        "Username, @name, or profile link",
    );

    if profile_lookup.is_some() && draft.is_none() {
        focus_on_load(&username, true);
    }
    let message = text("", 13.0, false);
    message.set_is_visible(false);
    AutomationProperties::set_live_setting(&message, AutomationLiveSetting::Polite);
    people.children().add(message.clone());
    let open = button("Open profile");
    people.children().add(open.clone());
    fields.push(username.clone());
    let [kind, sort, period] = choices;

    bind_submit(
        &people,
        &open,
        Rc::new(move || {
            let name = username.text().unwrap_or_default();
            let value = name.trim();
            let user = match HnDestination::from_link(value) {
                Some(HnDestination::Profile(user)) => user,
                _ => value.strip_prefix('@').unwrap_or(value).to_owned(),
            };
            let mut check = FormValidation::default();
            check.check(
                &username,
                validate_username(&user).map_err(|e| e.to_string()),
            );
            if check.finish(&message) {
                on_open(HnDestination::Profile(user));
            }
        }),
    );
    root.children()
        .add(text("Search powered by Algolia", 12.0, true));
    let scroll = ScrollViewer::new();
    scroll.set_content(Some(Control::boxed(root)));
    let sections = [Some(filters_toggle), recent_toggle, Some(people_toggle)];
    let focus_targets: Vec<_> = fields
        .iter()
        .map(|field| field.clone().upcast::<Control>())
        .chain([
            kind.clone().upcast(),
            sort.clone().upcast(),
            period.clone().upcast(),
            submit.upcast(),
            open.upcast(),
        ])
        .collect();
    if let Some(draft) = draft {
        // A scoped query locks the kind selector. Release it so that clearing the
        // restored discussion later returns to the reader's previous kind.
        discussion_scope_field.set_text(Some(""));
        kind.set_selected_index(draft.kind);
        sort.set_selected_index(draft.sort);
        period.set_selected_index(draft.period);
        for (field, (value, start, end)) in fields.iter().zip(draft.fields) {
            field.set_text(Some(&value));
            field.set_selection_start(start);
            field.set_selection_end(end);
        }
        for (section, expanded) in sections.iter().zip(draft.sections) {
            if let Some(section) = section {
                section.set_is_expanded(expanded);
            }
        }
        let target = draft
            .focus
            .and_then(|index| focus_targets.get(index))
            .unwrap_or(&focus_targets[0])
            .downgrade();
        let viewer = scroll.downgrade();
        scroll.loaded(move |_, _| {
            if let Some(target) = target.upgrade().filter(|c| c.is_effectively_visible()) {
                target.focus();
            }
            if let Some(viewer) = viewer.upgrade() {
                viewer.set_offset(draft.offset);
            }
        });
    }
    let control = scroll.clone().upcast();
    Form {
        control,
        capture: Box::new(move || Draft {
            baseline: baseline.clone(),
            fields: fields
                .iter()
                .map(|field| {
                    (
                        field.text().unwrap_or_default(),
                        field.selection_start(),
                        field.selection_end(),
                    )
                })
                .collect(),
            kind: saved_kind.get().unwrap_or_else(|| kind.selected_index()),
            sort: sort.selected_index(),
            period: period.selected_index(),
            sections: sections
                .each_ref()
                .map(|section| section.as_ref().is_some_and(|s| s.is_expanded())),
            focus: focus_targets
                .iter()
                .position(|target| target.is_keyboard_focus_within()),
            offset: scroll.offset(),
        }),
    }
}

fn focus_on_load(field: &TextBox, reveal: bool) {
    let input = field.to_ref().downgrade();
    field.loaded(move |_, _| {
        if let Some(input) = input.upgrade() {
            input.focus();
            if reveal {
                input.select_all();
                input.bring_into_view();
            }
        }
    });
}

/// Restrict Enter to text fields in this form. Both forms share the same top level.
fn bind_submit(scope: &StackPanel, button: &Button, submit: Rc<dyn Fn()>) {
    let pointer = submit.clone();
    button.click(move |_, _| pointer());
    scope.add_handler(
        InputElement::key_down_event(),
        move |_, e: &KeyEventArgs| {
            if e.key == Key::Enter
                && e.key_modifiers == KeyModifiers::empty()
                && e.source().is_some_and(|source| {
                    source
                        .cast::<TextBox>()
                        .is_some_and(|input| !input.is_composing_text())
                })
            {
                e.set_handled(true);
                submit();
            }
        },
    );
}

fn discussion_scope(value: &str) -> Result<Option<u64>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let id = match HnDestination::from_link(value) {
        Some(HnDestination::Discussion(id)) => Some(id),
        _ if value.bytes().all(|c| c.is_ascii_digit()) => {
            value.parse::<u64>().ok().filter(|id| *id > 0)
        }
        _ => None,
    };
    id.map(Some)
        .ok_or_else(|| "Enter a positive item ID or a Hacker News discussion link.".to_owned())
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
        || !value.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        })
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
        let opened = Rc::new(RefCell::new(None));
        let destination = opened.clone();
        let forgotten = Rc::new(RefCell::new(Vec::new()));
        let forgotten_terms = forgotten.clone();
        let search = build(
            &SearchQuery::default(),
            &["adaptive layout".to_owned(), "older query".to_owned()],
            None,
            None,
            Rc::new(move |term| forgotten_terms.borrow_mut().push(term.to_owned())),
            Rc::new(move |q| {
                result.replace(Some(q));
            }),
            Rc::new(move |target| {
                destination.replace(Some(target));
            }),
        );
        let window = Window::new();
        window.set_width(720.0);
        window.set_height(1120.0);
        window.set_content(Some(Control::boxed(&search.control)));
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
        input.set_text(Some("https://news.ycombinator.com/item?id=2002"));
        assert!(named("Open discussion").is_effectively_visible());
        enter();
        assert_eq!(*opened.borrow(), Some(HnDestination::Discussion(2002)));
        assert!(last.borrow().is_none());
        input.set_text(Some("https://news.ycombinator.com/user?id=aDa"));
        enter();
        assert_eq!(
            *opened.borrow(),
            Some(HnDestination::Profile("aDa".to_owned()))
        );
        input.set_text(Some(""));
        let scope = named("Search in");
        let order = named("Sort by");
        assert_eq!(
            Shell::bounds_of(&window, &scope).y,
            Shell::bounds_of(&window, &order).y
        );
        search.control.set_width(330.0);
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
        let date = named("From date").cast::<TextBox>().unwrap();
        date.set_text(Some("2024-02-30"));
        author.set_text(Some("invalid author!"));
        filters.set_is_expanded(false);
        shell.settle();
        enter();
        assert!(
            filters.is_expanded(),
            "validation reveals the hidden fields"
        );
        assert!(author.is_focused());
        assert_eq!(author.text().as_deref(), Some("invalid author!"));
        assert!(ferroui_controls::DataValidationErrors::get_has_errors(
            &author
        ));
        assert!(ferroui_controls::DataValidationErrors::get_has_errors(
            &date
        ));
        assert!(last.borrow().is_none(), "invalid fields submitted");
        author.set_text(Some("@"));
        enter();
        assert!(author.is_focused());
        author.set_text(Some("@ada"));
        enter();
        assert!(date.is_focused());
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
        search.control.set_width(680.0);
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
        filters.set_is_expanded(false);
        let recent = named("Recent searches").cast::<Expander>().unwrap();
        recent.set_is_expanded(true);
        shell.settle();
        click(&named("adaptive layout"));
        assert_eq!(input.text().as_deref(), Some("adaptive layout"));
        assert!(last.borrow().is_none());
        named("Sort by")
            .cast::<ComboBox>()
            .unwrap()
            .set_selected_index(1);
        click(&named("Run search for adaptive layout"));
        let q = last
            .borrow_mut()
            .take()
            .expect("quick replay submits the recent term");
        assert_eq!(q.query, "adaptive layout");
        assert_eq!(q.sort, SearchSort::Recent);
        click(&named("Remove adaptive layout from recent searches"));
        assert!(named("older query").is_focused());
        click(&named("Remove older query from recent searches"));
        assert_eq!(&*forgotten.borrow(), &["adaptive layout", "older query"]);
        assert!(input.is_focused());
        assert_eq!(input.text().as_deref(), Some("adaptive layout"));
        window.close();
    }

    #[test]
    fn profile_recovery_opens_the_editor_with_the_failed_username_selected() {
        use ferroui_controls::Window;
        use sample_testing::Shell;
        use std::cell::RefCell;
        let shell = Shell::start(&crate::SAMPLE, 330.0, 520.0, || crate::App::new().upcast());
        let submitted = Rc::new(RefCell::new(None));
        let result = submitted.clone();
        let form = build(
            &SearchQuery::default(),
            &[],
            Some("TypoUser"),
            None,
            Rc::new(|_| {}),
            Rc::new(|_| panic!("profile lookup submitted a content search")),
            Rc::new(move |user| {
                result.replace(Some(user));
            }),
        );
        let window = Window::new();
        window.set_width(330.0);
        window.set_height(520.0);
        window.set_content(Some(Control::boxed(form.control)));
        window.show();
        shell.settle();
        let username = window
            .get_visual_descendants()
            .filter_map(|c| c.cast::<TextBox>())
            .find(|c| AutomationProperties::get_name(c).as_deref() == Some("Hacker News username"))
            .unwrap();
        assert!(username.is_focused());
        assert_eq!(username.selected_text(), "TypoUser");
        let bounds = Shell::bounds_of(&window, &username);
        assert!(
            bounds.y >= 0.0 && bounds.bottom() <= window.bounds().height,
            "profile field off screen: {bounds:?}"
        );
        username.set_text(Some("https://news.ycombinator.com/user?id=Ada"));
        let mut key = KeyEventArgs::new();
        key.key = Key::Enter;
        key.set_routed_event(Some(InputElement::key_down_event()));
        username.raise_event(&key);
        shell.settle();
        assert_eq!(
            *submitted.borrow(),
            Some(HnDestination::Profile("Ada".into()))
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
        assert!(parse_date("2024-+2-03").is_err());
        let leap = parse_date("2024-02-29").unwrap();
        assert_eq!(format_date(leap), "2024-02-29");
        assert_eq!(parse_date(" ").unwrap(), None);
    }
}
