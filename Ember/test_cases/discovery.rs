use super::*;

fn action(view: &ReaderView, name: &str) -> Ref<Button> {
    view.get_visual_descendants()
        .filter_map(|v| v.cast::<Button>())
        .find(|button| {
            button.is_effectively_visible()
                && AutomationProperties::get_name(button).as_deref() == Some(name)
        })
        .unwrap_or_else(|| panic!("expected an action named {name}"))
}

#[test]
fn retrying_profile_activity_recovers_only_missing_cards_in_submission_order() {
    struct PartialProfile {
        fixture: FixtureHttp,
        fail: Cell<bool>,
        requests: RefCell<Vec<String>>,
    }
    impl HttpClient for PartialProfile {
        fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, ReaderError>> {
            if url.ends_with("/user/ada.json") {
                self.requests.borrow_mut().push("profile".into());
                return Box::pin(async {
                    Ok(json!({"id":"ada","submitted":[6001,6002]}).to_string())
                });
            }
            for id in [6001, 6002] {
                if url.ends_with(&format!("/item/{id}.json")) {
                    self.requests.borrow_mut().push(id.to_string());
                    return Box::pin(async move {
                        if id == 6001 && self.fail.get() {
                            Err(ReaderError::Network("Transient activity failure".into()))
                        } else {
                            Ok(json!({"id":id,"type":"story","by":"ada","title":format!("Submission {id}")}).to_string())
                        }
                    });
                }
            }
            self.fixture.get(url)
        }
    }
    let http = Rc::new(PartialProfile {
        fixture: FixtureHttp {
            offline: Rc::new(Cell::new(false)),
        },
        fail: Cell::new(true),
        requests: RefCell::new(Vec::new()),
    });
    let (shell, window, view) =
        start_with_http(1280.0, 800.0, Rc::new(MemoryStore::default()), http.clone());
    view.get_control::<TextBox>("SearchBox")
        .set_text(Some("https://news.ycombinator.com/user?id=ada"));
    click_in_view(&shell, &window, &view.get_control::<Button>("SearchButton"));
    let cards = || {
        view.get_control::<ferroui_controls::ContentControl>("DetailHost")
            .get_visual_descendants()
            .filter_map(|v| v.cast::<crate::controls::StoryCard>())
            .collect::<Vec<_>>()
    };
    let before = cards()
        .pop()
        .expect("the readable submission remains available");
    assert_eq!(before.headline().as_deref(), Some("Submission 6002"));
    http.requests.borrow_mut().clear();
    click_in_view(&shell, &window, &action(&view, "Retry activity"));
    assert!(action(&view, "Retry activity").is_focused());
    http.fail.set(false);
    click_in_view(&shell, &window, &action(&view, "Retry activity"));
    assert_eq!(&*http.requests.borrow(), &["6001", "6001"]);
    let after = cards();
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].headline().as_deref(), Some("Submission 6001"));
    assert!(after[0].is_focused());
    assert!(std::ptr::eq(&*after[1], &*before));
    window.close();
}

#[test]
fn opening_author_activity_uses_the_permalink_even_when_the_feed_is_a_comment_search() {
    let (shell, window, view) = start(
        1440.0,
        940.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    view.get_control::<TextBox>("SearchBox")
        .set_text(Some("predictable"));
    click(&shell, &window, &view.get_control::<Button>("SearchButton"));
    assert_eq!(title(&view), "Search results");
    click(&shell, &window, &first_story(&window));
    let detail_title = view.get_control::<TextBlock>("DetailTitle");
    assert_eq!(
        detail_title.text().as_deref(),
        Some("Small tools, carefully made")
    );
    click_in_view(&shell, &window, &action(&view, "View ada’s profile"));
    assert_eq!(detail_title.text().as_deref(), Some("ada"));
    let comment = view
        .get_control::<ferroui_controls::ContentControl>("DetailHost")
        .get_visual_descendants()
        .filter_map(|c| c.cast::<crate::controls::StoryCard>())
        .find(|card| card.headline().unwrap_or_default().starts_with("Exactly."))
        .expect("recent comment preview");
    if let Some(dir) = std::env::var_os("EMBER_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        shell
            .last_frame(0)
            .save_ppm(&std::path::Path::new(&dir).join("desktop-profile.ppm"))
            .unwrap();
    }
    click_in_view(&shell, &window, &comment);
    assert_eq!(detail_title.text().as_deref(), Some("Comment by ada"));
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert_eq!(detail_title.text().as_deref(), Some("ada"));
    click_in_view(&shell, &window, &action(&view, "All comments"));
    assert_eq!(title(&view), "Search results");
    window.close();
}

#[test]
fn masthead_validates_utf8_budget_in_place_and_identifies_permalinks() {
    let store = Rc::new(MemoryStore::default());
    let (shell, window, view) = start(1280.0, 800.0, store.clone(), Rc::new(Cell::new(false)));
    let field = view.get_control::<TextBox>("SearchBox");
    let submit = view.get_control::<Button>("SearchButton");
    let error = view.get_control::<TextBlock>("QuickSearchError");
    field.set_text(Some(&"界".repeat(683)));
    click_in_view(&shell, &window, &submit);
    assert!(error.is_effectively_visible());
    assert!(ferroui_controls::DataValidationErrors::get_has_errors(
        &field
    ));
    assert!(field.is_focused());
    assert_eq!(title(&view), "Front page");
    field.set_text(Some("https://news.ycombinator.com/user?id=ada"));
    assert!(!error.is_visible());
    assert!(!ferroui_controls::DataValidationErrors::get_has_errors(
        &field
    ));
    assert_eq!(
        AutomationProperties::get_name(&submit).as_deref(),
        Some("Open profile link")
    );
    field.set_text(Some("https://news.ycombinator.com/item?id=1001"));
    assert_eq!(
        AutomationProperties::get_name(&submit).as_deref(),
        Some("Open discussion")
    );
    field.set_text(Some("systems"));
    assert_eq!(
        AutomationProperties::get_name(&submit).as_deref(),
        Some("Search")
    );
    click_in_view(&shell, &window, &submit);
    assert_eq!(title(&view), "Search results");
    let state =
        ReaderState::import_json(&ready(store.get("reader:state")).unwrap().unwrap()).unwrap();
    assert_eq!(state.recent_searches, ["systems"]);
    window.close();
}

#[test]
fn missing_profile_can_reopen_and_correct_the_username_on_a_phone() {
    let (shell, window, view) = start(
        390.0,
        844.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    click(&shell, &window, &view.get_control::<Button>("MobileSearch"));
    let terms = || {
        view.get_control::<ferroui_controls::ContentControl>("DetailHost")
            .get_visual_descendants()
            .filter_map(|v| v.cast::<TextBox>())
            .find(|c| c.name().as_deref() == Some("SearchTerms"))
            .unwrap()
    };
    terms().set_text(Some("unfinished search"));
    let expander = view
        .get_visual_descendants()
        .filter_map(|v| v.cast::<ferroui_controls::Expander>())
        .find(|e| AutomationProperties::get_name(e).as_deref() == Some("Find a person"))
        .unwrap();
    expander.set_is_expanded(true);
    shell.settle();
    let editor = || {
        view.get_visual_descendants()
            .filter_map(|c| c.cast::<TextBox>())
            .find(|c| AutomationProperties::get_name(c).as_deref() == Some("Hacker News username"))
            .unwrap()
    };
    editor().set_text(Some("missing"));
    click_in_view(&shell, &window, &action(&view, "Open profile"));
    click_in_view(&shell, &window, &action(&view, "Edit username"));
    assert_eq!(terms().text().as_deref(), Some("unfinished search"));
    assert_eq!(editor().selected_text(), "missing");
    assert!(editor().is_focused());
    editor().set_text(Some("ada"));
    click_in_view(&shell, &window, &action(&view, "Open profile"));
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("ada")
    );
    assert!(action(&view, "All stories").is_effectively_visible());
    let back = view.get_control::<Button>("BackButton");
    click_in_view(&shell, &window, &back);
    assert_eq!(terms().text().as_deref(), Some("unfinished search"));
    assert_eq!(editor().text().as_deref(), Some("ada"));
    click_in_view(&shell, &window, &back);
    assert!(view
        .get_control::<ferroui_controls::Border>("FeedPane")
        .is_effectively_visible());
    window.close();
}

#[test]
fn explore_library_counts_update_when_storage_finishes_opening() {
    let inner = Rc::new(MemoryStore::default());
    let mut state = ReaderState::default();
    state.mark_read(1001, 1);
    ready(inner.put("reader:state", &state.export_json().unwrap())).unwrap();
    let store = Rc::new(DelayedStartupStore {
        inner,
        started: Cell::new(false),
        released: Cell::new(false),
        waker: RefCell::new(None),
        fail_read: false,
    });
    let (shell, window, view) = start(390.0, 844.0, store.clone(), Rc::new(Cell::new(false)));
    click(&shell, &window, &view.get_control::<Button>("MobileMore"));
    let history = action(&view, "Reading history");
    assert!(AutomationProperties::get_item_status(&history)
        .unwrap_or_default()
        .is_empty());
    store.release();
    for _ in 0..8 {
        shell.settle();
    }
    assert_eq!(
        AutomationProperties::get_item_status(&history).as_deref(),
        Some("1 item")
    );
    click_in_view(&shell, &window, &history);
    assert_eq!(title(&view), "Reading history");
    assert_eq!(view.get_control::<ListBox>("Stories").items().count(), 1);
    window.close();
}

#[test]
fn an_author_can_be_muted_and_restored_without_losing_the_open_profile() {
    let store = Rc::new(MemoryStore::default());
    let (shell, window, view) = start(1440.0, 940.0, store.clone(), Rc::new(Cell::new(false)));
    view.get_control::<TextBox>("SearchBox")
        .set_text(Some("https://news.ycombinator.com/user?id=ada"));
    shell.settle();
    click(&shell, &window, &view.get_control::<Button>("SearchButton"));
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("ada")
    );
    let mute = action(&view, "Mute ada in feeds")
        .cast::<ferroui_controls::primitives::ToggleButton>()
        .unwrap();
    let stories = view.get_control::<ListBox>("Stories");
    let count = stories.items().count();
    assert_eq!(mute.is_checked(), Some(false));
    click_in_view(&shell, &window, &mute);
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("ada")
    );
    assert_eq!(mute.is_checked(), Some(true));
    assert!(stories.items().count() < count);
    let persisted =
        || ReaderState::import_json(&ready(store.get("reader:state")).unwrap().unwrap()).unwrap();
    assert!(persisted().mutes.users.contains("ada"));
    assert!(persisted().recent_searches.is_empty());
    click_in_view(&shell, &window, &mute);
    assert_eq!(mute.is_checked(), Some(false));
    assert_eq!(stories.items().count(), count);
    assert!(!persisted().mutes.users.contains("ada"));
    window.close();
}

#[test]
fn profile_mute_waits_for_saved_rules_before_accepting_changes() {
    let inner = Rc::new(MemoryStore::default());
    let mut state = ReaderState::default();
    state.mutes.users.insert("ada".into());
    ready(inner.put("reader:state", &state.export_json().unwrap())).unwrap();
    let store = Rc::new(DelayedStartupStore {
        inner,
        started: Cell::new(false),
        released: Cell::new(false),
        waker: RefCell::new(None),
        fail_read: false,
    });
    let (shell, window, view) = start(1440.0, 940.0, store.clone(), Rc::new(Cell::new(false)));
    view.get_control::<TextBox>("SearchBox")
        .set_text(Some("https://news.ycombinator.com/user?id=ada"));
    shell.settle();
    click(&shell, &window, &view.get_control::<Button>("SearchButton"));
    let mute = action(&view, "Mute ada in feeds")
        .cast::<ferroui_controls::primitives::ToggleButton>()
        .unwrap();
    assert!(!mute.is_enabled());
    store.release();
    for _ in 0..8 {
        shell.settle();
    }
    assert!(mute.is_enabled());
    assert_eq!(mute.is_checked(), Some(true));
    window.close();
}

#[test]
fn pending_settings_preserve_backup_edits_and_open_the_actual_saved_preferences() {
    let inner = Rc::new(MemoryStore::default());
    let mut state = ReaderState::default();
    state.preferences.font_size = 22.0;
    state.toggle_saved(1001, 1);
    ready(inner.put("reader:state", &state.export_json().unwrap())).unwrap();
    let store = Rc::new(DelayedStartupStore {
        inner,
        started: Cell::new(false),
        released: Cell::new(false),
        waker: RefCell::new(None),
        fail_read: false,
    });
    let (shell, window, view) = start(1440.0, 940.0, store.clone(), Rc::new(Cell::new(false)));
    click(&shell, &window, &view.get_control::<Button>("ThemeButton"));
    let visible = |label: &str| {
        view.get_visual_descendants()
            .filter_map(|c| c.cast::<Control>())
            .find(|c| {
                c.is_effectively_visible()
                    && AutomationProperties::get_name(c).as_deref() == Some(label)
            })
    };
    assert!(visible("Copy backup").is_none());
    assert!(visible("Apply preferences").is_none());
    let input = visible("Reading data JSON")
        .unwrap()
        .cast::<TextBox>()
        .unwrap();
    input.set_text(Some("unfinished draft"));
    assert!(visible("Import this backup").is_some());
    store.release();
    for _ in 0..8 {
        shell.settle();
    }
    let size = visible("Reading font size")
        .unwrap()
        .cast::<ferroui_controls::NumberBox>()
        .unwrap();
    assert_eq!(size.value(), 22.0);
    assert!(visible("Copy backup").is_some());
    assert_eq!(input.text().as_deref(), Some("unfinished draft"));
    window.close();
}

#[test]
fn discussion_search_keeps_its_scope_and_opens_the_exact_comment() {
    use ferroui_base::input::{InputElement, Key, KeyEventArgs};
    use ferroui_base::interactivity::RoutedEventArgs;
    use ferroui_controls::{ComboBox, Expander, MenuFlyout, MenuItem};

    let store = Rc::new(MemoryStore::default());
    let mut initial = ReaderState::default();
    initial.preferences.reduced_motion = true;
    ready(store.put("reader:state", &initial.export_json().unwrap())).unwrap();
    let (shell, window, view) = start(390.0, 844.0, store, Rc::new(Cell::new(false)));
    click(&shell, &window, &first_story(&window));
    let menu = action(&view, "More story actions")
        .flyout()
        .unwrap()
        .cast::<MenuFlyout>()
        .unwrap();
    let entry = (0..menu.items().count())
        .filter_map(|i| {
            Control::from_boxed(&menu.items().get_at(i)?)
                .and_then(|control| control.cast::<MenuItem>())
        })
        .find(|item| {
            item.header()
                .and_then(|value| value.downcast_ref::<String>().cloned())
                .as_deref()
                == Some("Search this discussion")
        })
        .unwrap();
    entry.raise_event(&RoutedEventArgs::with_event(MenuItem::click_event()));
    shell.settle();
    let named = |name: &str| {
        view.get_visual_descendants()
            .filter_map(|v| v.cast::<Control>())
            .find(|c| AutomationProperties::get_name(c).as_deref() == Some(name))
            .unwrap_or_else(|| panic!("missing {name}"))
    };
    let input = || named("Search terms").cast::<TextBox>().unwrap();
    let enter = || {
        let mut event = KeyEventArgs::new();
        event.key = Key::Enter;
        event.set_routed_event(Some(InputElement::key_down_event()));
        input().raise_event(&event);
        for _ in 0..6 {
            shell.settle();
        }
    };
    assert!(view
        .get_visual_descendants()
        .filter_map(|v| v.cast::<TextBlock>())
        .any(|v| v.text().as_deref() == Some("Discussion 1001")));
    assert!(!named("Search in").is_enabled());
    input().set_text(Some("predictable"));
    if let Some(dir) = std::env::var_os("EMBER_CAPTURE_DIR") {
        shell.settle();
        std::fs::create_dir_all(&dir).unwrap();
        shell
            .last_frame(0)
            .save_ppm(&std::path::Path::new(&dir).join("phone-discussion-search.ppm"))
            .unwrap();
    }
    enter();
    assert!(view
        .get_control::<TextBlock>("FeedSummary")
        .text()
        .unwrap()
        .starts_with("Discussion 1001 · 1 result"));
    click_in_view(&shell, &window, &first_story(&window));
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("Comment by ada")
    );
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    click(
        &shell,
        &window,
        &view.get_control::<Button>("EditFeedSearch"),
    );
    named("More filters")
        .cast::<Expander>()
        .unwrap()
        .set_is_expanded(true);
    shell.settle();
    let scoped = named("Discussion").cast::<TextBox>().unwrap();
    assert_eq!(scoped.text().as_deref(), Some("1001"));
    scoped.set_text(Some("https://example.org/item?id=7"));
    enter();
    assert!(scoped.is_focused());
    assert!(ferroui_controls::DataValidationErrors::get_has_errors(
        &scoped
    ));
    scoped.set_text(Some(""));
    let kind = named("Search in").cast::<ComboBox>().unwrap();
    assert!(kind.is_enabled());
    kind.set_selected_index(0);
    scoped.set_text(Some("https://news.ycombinator.com/item?id=1001"));
    assert!(!kind.is_enabled());
    assert_eq!(kind.selected_index(), 1);
    scoped.set_text(Some(""));
    assert_eq!(kind.selected_index(), 0);
    window.close();
}

#[test]
fn back_from_a_profile_restores_unsubmitted_search_edits_and_the_phone_viewport() {
    use ferroui_base::input::{InputElement, Key, KeyEventArgs};
    use ferroui_controls::{ComboBox, ContentControl, Expander, ScrollViewer};

    let (shell, window, view) = start(
        390.0,
        650.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    click(&shell, &window, &view.get_control::<Button>("MobileSearch"));
    let named = |name: &str| {
        view.get_control::<ContentControl>("DetailHost")
            .get_visual_descendants()
            .filter_map(|node| node.cast::<Control>())
            .find(|control| AutomationProperties::get_name(control).as_deref() == Some(name))
            .unwrap_or_else(|| panic!("missing {name}"))
    };
    let field = |name: &str| named(name).cast::<TextBox>().unwrap();
    let filters = named("More filters").cast::<Expander>().unwrap();
    filters.set_is_expanded(true);
    shell.settle();
    field("Search terms").set_text(Some("  unfinished query  "));
    named("Search in")
        .cast::<ComboBox>()
        .unwrap()
        .set_selected_index(4);
    field("Discussion").set_text(Some("1001"));
    field("Minimum points").set_text(Some("1x"));
    let terms = field("Search terms");
    let mut shortcut = KeyEventArgs::new();
    shortcut.key = Key::K;
    shortcut.key_modifiers = ferroui_base::input::KeyModifiers::CONTROL;
    shortcut.set_routed_event(Some(InputElement::key_down_event()));
    field("Minimum points").raise_event(&shortcut);
    shell.settle();
    assert!(shortcut.handled());
    assert!(std::ptr::eq(&*terms, &*field("Search terms")));
    assert!(terms.is_focused());
    named("Find a person")
        .cast::<Expander>()
        .unwrap()
        .set_is_expanded(true);
    shell.settle();
    let name = field("Hacker News username");
    name.set_text(Some("@ada"));
    name.bring_into_view();
    name.focus();
    name.set_selection_start(1);
    name.set_selection_end(4);
    shell.settle();
    let viewer = || {
        view.get_control::<ContentControl>("DetailHost")
            .get_visual_descendants()
            .find_map(|c| c.cast::<ScrollViewer>())
            .unwrap()
    };
    let position = viewer().offset().y;
    assert!(
        position > 0.0,
        "the test must leave the top of the search form"
    );
    let mut enter = KeyEventArgs::new();
    enter.key = Key::Enter;
    enter.set_routed_event(Some(InputElement::key_down_event()));
    name.raise_event(&enter);
    for _ in 0..6 {
        shell.settle();
    }
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("ada")
    );
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert_eq!(
        field("Search terms").text().as_deref(),
        Some("  unfinished query  ")
    );
    assert_eq!(field("Minimum points").text().as_deref(), Some("1x"));
    assert!(named("More filters")
        .cast::<Expander>()
        .unwrap()
        .is_expanded());
    assert!(named("Find a person")
        .cast::<Expander>()
        .unwrap()
        .is_expanded());
    let name = field("Hacker News username");
    assert_eq!(name.text().as_deref(), Some("@ada"));
    assert!(name.is_focused());
    assert_eq!(name.selected_text(), "ada");
    assert!((viewer().offset().y - position).abs() < 2.0);
    field("Discussion").set_text(Some(""));
    assert_eq!(
        named("Search in")
            .cast::<ComboBox>()
            .unwrap()
            .selected_index(),
        4
    );
    window.close();
}

#[test]
fn resubmitting_unchanged_search_results_keeps_loaded_pages_and_scroll_position() {
    use ferroui_base::Vector;
    use ferroui_controls::ScrollViewer;
    struct PagedSearch(FixtureHttp);
    impl HttpClient for PagedSearch {
        fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, ReaderError>> {
            if !url.contains("/api/v1/search") {
                return self.0.get(url);
            }
            Box::pin(async move {
                let second = url.split(['?', '&']).any(|p| p == "page=1");
                let ids = if second { 3030..3032 } else { 3000..3030 };
                let hits: Vec<_> = ids.map(|id| json!({
                    "objectID": id.to_string(), "title": format!("Result {id}"), "author": "ada"
                })).collect();
                Ok(
                    json!({"hits": hits, "page": usize::from(second), "nbPages":2,"nbHits":32})
                        .to_string(),
                )
            })
        }
    }
    let (shell, window, view) = start_with_http(
        1280.0,
        700.0,
        Rc::new(MemoryStore::default()),
        Rc::new(PagedSearch(FixtureHttp {
            offline: Rc::new(Cell::new(false)),
        })),
    );
    view.get_control::<TextBox>("SearchBox")
        .set_text(Some("testing"));
    click(&shell, &window, &view.get_control::<Button>("SearchButton"));
    click_in_view(
        &shell,
        &window,
        &view.get_control::<Button>("LoadMoreButton"),
    );
    let stories = view.get_control::<ListBox>("Stories");
    assert_eq!(stories.items().count(), 32);
    let scroll = stories.scroll().unwrap().cast::<ScrollViewer>().unwrap();
    scroll.set_offset(Vector::new(0.0, 425.0));
    shell.settle();
    let before = scroll.offset().y;
    assert!(before > 0.0);
    click(
        &shell,
        &window,
        &view.get_control::<Button>("EditFeedSearch"),
    );
    let submit = view
        .get_control::<ferroui_controls::ContentControl>("DetailHost")
        .get_visual_descendants()
        .filter_map(|node| node.cast::<Button>())
        .find(|button| AutomationProperties::get_name(button).as_deref() == Some("Search"))
        .unwrap();
    click_in_view(&shell, &window, &submit);
    assert_eq!(stories.items().count(), 32);
    assert!((scroll.offset().y - before).abs() < 2.0);
    window.close();
}

#[test]
fn preference_edits_survive_reselecting_settings_and_follow_the_live_unread_filter() {
    use ferroui_controls::{CheckBox, ComboBox, NumberBox};
    let store = Rc::new(MemoryStore::default());
    let mut library = ReaderState::default();
    library.preferences.theme = ember_core::ThemeMode::Custom;
    library.preferences.custom_background = Some("#29405D".into());
    ready(store.put("reader:state", &library.export_json().unwrap())).unwrap();
    let (shell, window, view) = start(1440.0, 960.0, store.clone(), Rc::new(Cell::new(false)));
    let tap = |button: &Control| click_in_view(&shell, &window, button);
    let settings = view.get_control::<Button>("ThemeButton");
    tap(&settings);
    let named = |label: &str| {
        view.get_visual_descendants()
            .filter_map(|v| v.cast::<Control>())
            .find(|c| AutomationProperties::get_name(c).as_deref() == Some(label))
            .unwrap_or_else(|| panic!("missing {label}"))
    };
    let accent = named("Accent color").cast::<TextBox>().unwrap();
    let apply = named("Apply preferences");
    let discard = named("Discard preference edits");
    assert!(!apply.is_enabled());
    assert!(!discard.is_visible());
    tap(&named("Reset theme & type"));
    let number = named("Reading font size").cast::<NumberBox>().unwrap();
    let edit = number
        .get_visual_descendants()
        .find_map(|v| v.cast::<TextBox>())
        .unwrap();
    edit.set_text(Some("unfinished"));
    tap(&settings);
    assert!(std::ptr::eq(
        &*accent,
        &*named("Accent color").cast::<TextBox>().unwrap()
    ));
    assert_eq!(edit.text().as_deref(), Some("unfinished"));
    assert!(apply.is_enabled());
    tap(&view.get_control::<Button>("UnreadButton"));
    assert_eq!(
        named("Hide read stories")
            .cast::<CheckBox>()
            .unwrap()
            .is_checked(),
        Some(true)
    );
    tap(&discard);
    assert_eq!(edit.text().as_deref(), Some("16"));
    assert_eq!(
        named("Theme").cast::<ComboBox>().unwrap().selected_index(),
        3
    );
    assert_eq!(
        named("Custom background")
            .cast::<TextBox>()
            .unwrap()
            .text()
            .as_deref(),
        Some("#29405D")
    );
    assert_eq!(
        named("Hide read stories")
            .cast::<CheckBox>()
            .unwrap()
            .is_checked(),
        Some(true)
    );
    assert!(!apply.is_enabled());
    assert!(!discard.is_visible());
    accent.set_text(Some("#12789A"));
    tap(&apply);
    let applied =
        ReaderState::import_json(&ready(store.get("reader:state")).unwrap().unwrap()).unwrap();
    assert_eq!(applied.preferences.accent, "#12789A");
    assert_eq!(
        applied.preferences.custom_background.as_deref(),
        Some("#29405D")
    );
    assert!(applied.preferences.hide_read);
    window.close();
}
