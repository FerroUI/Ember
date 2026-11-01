use super::*;
use ferroui_controls::{Border, ContentControl};

#[test]
fn a_short_landscape_window_keeps_the_same_primary_tabs_and_an_unbroken_reading_pane() {
    let (shell, window, view) = start(
        844.0,
        640.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    click(&shell, &window, &first_story(&window));
    let title = view.get_control::<TextBlock>("DetailTitle");
    let tabs = view.get_control::<Border>("BottomNavigation");
    let rail = view.get_control::<Border>("NavigationRail");
    let search = view.get_control::<Button>("MobileSearch");
    let tab_content = search.content().unwrap();
    assert!(rail.is_effectively_visible());

    // Height alone must be enough to switch an already open discussion to the short layout.
    window.set_height(390.0);
    for _ in 0..4 {
        shell.settle();
    }
    assert_eq!(title.text().as_deref(), Some("Small tools, carefully made"));
    assert!(tabs.is_effectively_visible());
    assert!(!rail.is_effectively_visible());
    assert!(!view
        .get_control::<Border>("Masthead")
        .is_effectively_visible());
    assert!(!view
        .get_control::<Border>("FeedPane")
        .is_effectively_visible());
    let navigation = Shell::bounds_of(&window, &tabs);
    let document = Shell::bounds_of(&window, &view.get_control::<ContentControl>("DetailHost"));
    assert!(navigation.width <= 64.0 && navigation.height >= 380.0);
    assert!(document.x >= navigation.right() && document.width >= 770.0);
    assert!(document.height > 320.0);
    if let Some(dir) = std::env::var_os("EMBER_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        shell
            .last_frame(0)
            .save_ppm(&std::path::Path::new(&dir).join("landscape-reader.ppm"))
            .unwrap();
    }
    click_in_view(&shell, &window, &search);
    assert_eq!(title.text().as_deref(), Some("Search"));
    window.set_width(390.0);
    window.set_height(844.0);
    for _ in 0..4 {
        shell.settle();
    }
    assert!(Rc::ptr_eq(&tab_content, &search.content().unwrap()));
    let navigation = Shell::bounds_of(&window, &tabs);
    assert!(navigation.width >= 380.0 && navigation.y > 750.0);
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert_eq!(title.text().as_deref(), Some("Small tools, carefully made"));
    window.close();
}

#[test]
fn narrow_toolbar_downloads_and_enters_focus_from_the_sticky_discussion_options() {
    use ferroui_base::interactivity::RoutedEventArgs;
    use ferroui_controls::{MenuFlyout, MenuItem};
    let store = Rc::new(MemoryStore::default());
    let (shell, window, view) = start(320.0, 540.0, store.clone(), Rc::new(Cell::new(false)));
    click(&shell, &window, &first_story(&window));
    let more = view.get_control::<Button>("ToolbarMore");
    let download = view.get_control::<Button>("DownloadButton");
    assert!(!download.is_effectively_visible());
    let menu = more.flyout().unwrap().cast::<MenuFlyout>().unwrap();
    let invoke = |name: &str| {
        click_in_view(&shell, &window, &more);
        assert!(menu.is_open());
        let items = menu.items();
        let item = (0..items.count())
            .filter_map(|i| Control::from_boxed(&items.get_at(i)?))
            .filter_map(|c| c.cast::<MenuItem>())
            .find(|item| {
                item.is_visible()
                    && item
                        .header()
                        .and_then(|h| h.downcast_ref::<String>().cloned())
                        .as_deref()
                        == Some(name)
            })
            .unwrap_or_else(|| panic!("missing option {name}"));
        item.raise_event(&RoutedEventArgs::with_event(MenuItem::click_event()));
        menu.hide();
        for _ in 0..5 {
            shell.settle();
        }
    };
    invoke("Download thread");
    let state =
        ReaderState::import_json(&ready(store.get("reader:state")).unwrap().unwrap()).unwrap();
    assert!(state.downloaded_threads.contains_key(&1001));
    invoke("Focus reading");
    assert!(!view
        .get_control::<Border>("BottomNavigation")
        .is_effectively_visible());
    let exit = view.get_control::<Button>("FocusButton");
    assert_eq!(
        AutomationProperties::get_name(&exit).as_deref(),
        Some("Exit focus reading")
    );
    click(&shell, &window, &exit);
    assert!(more.is_effectively_visible());
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("Small tools, carefully made")
    );
    window.set_width(390.0);
    shell.settle();
    assert!(download.is_effectively_visible());
    assert_eq!(
        AutomationProperties::get_name(&download).as_deref(),
        Some("Manage downloaded discussion")
    );
    window.close();
}

#[test]
fn an_unavailable_permalink_can_be_retried_from_the_detail_pane() {
    let offline = Rc::new(Cell::new(true));
    let (shell, window, view) = start(
        320.0,
        440.0,
        Rc::new(MemoryStore::default()),
        offline.clone(),
    );
    crate::navigate_location("?item=1001");
    for _ in 0..5 {
        shell.settle();
    }
    let retry = view
        .get_visual_descendants()
        .filter_map(|v| v.cast::<Button>())
        .find(|b| AutomationProperties::get_name(b).as_deref() == Some("Retry discussion"))
        .expect("a failed deep link should include a reachable retry");
    offline.set(false);
    click_in_view(&shell, &window, &retry);
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("Small tools, carefully made")
    );
    window.close();
}

#[test]
fn history_traversal_keeps_find_intact_and_back_does_not_reopen_the_forward_story() {
    let (shell, window, view) = start(
        390.0,
        844.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    let title = view.get_control::<TextBlock>("DetailTitle");
    let navigate = |url| {
        crate::navigate_location(url);
        for _ in 0..4 {
            shell.settle();
        }
    };
    click(&shell, &window, &first_story(&window));
    navigate("#item/1002");
    click_in_view(&shell, &window, &view.get_control::<Button>("FindButton"));
    let find = view.get_control::<TextBox>("ThreadFind");
    find.set_text(Some("needle"));
    navigate("#item/1002");
    assert_eq!(find.text().as_deref(), Some("needle"));
    assert!(find.is_effectively_visible());
    navigate("?id=1001");
    assert_eq!(title.text().as_deref(), Some("Small tools, carefully made"));
    click_in_view(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert!(view
        .get_control::<Border>("FeedPane")
        .is_effectively_visible());
    window.close();
}

#[test]
fn library_and_discussion_find_follow_the_pane_receiving_the_shortcut() {
    use ferroui_base::input::{InputElement, Key, KeyEventArgs, KeyModifiers};
    use ferroui_base::interactivity::RoutedEventArgs;
    use ferroui_controls::TopLevel;
    let store = Rc::new(MemoryStore::default());
    let (shell, window, view) = start(1440.0, 960.0, store, Rc::new(Cell::new(false)));
    click(&shell, &window, &first_story(&window));
    click_in_view(&shell, &window, &view.get_control::<Button>("SaveButton"));
    click_in_view(&shell, &window, &view.get_control::<Button>("NavSaved"));
    let row = first_story(&window);
    click_in_view(&shell, &window, &row);
    let key = |origin: &Control, key, modifiers| {
        let mut event = KeyEventArgs::new();
        event.key = key;
        event.key_modifiers = modifiers;
        event.set_routed_event(Some(InputElement::key_down_event()));
        origin.raise_event(&event);
        shell.settle();
        assert!(event.handled());
    };
    key(&row, Key::F, KeyModifiers::CONTROL);
    assert!(view.get_control::<TextBox>("LibraryFind").is_focused());
    assert!(!view
        .get_control::<TextBox>("ThreadFind")
        .is_effectively_visible());
    key(
        &view.get_control::<TextBox>("LibraryFind"),
        Key::Escape,
        KeyModifiers::NONE,
    );
    key(
        &view.get_control::<Button>("BackButton"),
        Key::F,
        KeyModifiers::META,
    );
    assert!(view.get_control::<TextBox>("ThreadFind").is_focused());
    // Return to the library and use OS Back to dismiss the local search first.
    click_in_view(&shell, &window, &view.get_control::<Button>("BackButton"));
    click_in_view(&shell, &window, &view.get_control::<Button>("BackButton"));
    click_in_view(
        &shell,
        &window,
        &view.get_control::<Button>("LibraryFindButton"),
    );
    let back = RoutedEventArgs::with_event(TopLevel::back_requested_event());
    window.raise_event(&back);
    shell.settle();
    assert!(back.handled());
    assert!(view.get_control::<Button>("LibraryFindButton").is_focused());
    window.close();
}
