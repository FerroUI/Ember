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
fn missing_profile_can_reopen_and_correct_the_username_on_a_phone() {
    let (shell, window, view) = start(
        390.0,
        844.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    click(&shell, &window, &view.get_control::<Button>("MobileSearch"));
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
    assert_eq!(editor().text().as_deref(), Some("missing"));
    editor().set_text(Some("ada"));
    click_in_view(&shell, &window, &action(&view, "Open profile"));
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("ada")
    );
    assert!(action(&view, "All stories").is_effectively_visible());
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

    let (shell, window, view) = start(
        390.0,
        844.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
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
