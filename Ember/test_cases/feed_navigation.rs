use super::*;
use ferroui_controls::{primitives::ToggleButton, ScrollViewer};

#[test]
fn active_phone_tab_returns_to_loaded_pages_and_filters_expose_checked_state() {
    let store = Rc::new(MemoryStore::default());
    let mut state = ReaderState::default();
    for id in 3000..=3034 {
        state.toggle_saved(id, id);
    }
    ready(store.put("reader:state", &state.export_json().unwrap())).unwrap();
    let (shell, window, view) = start(390.0, 700.0, store, Rc::new(Cell::new(false)));
    let tap = |name| click_in_view(&shell, &window, &view.get_control::<Button>(name));
    let saved = view.get_control::<ToggleButton>("DownloadedOnly");
    tap("MobileSaved");
    assert_eq!(saved.is_checked(), Some(false));
    tap("DownloadedOnly");
    assert_eq!(saved.is_checked(), Some(true));
    tap("RetryButton");
    assert_eq!(saved.is_checked(), Some(false));
    tap("LoadMoreButton");
    let stories = view.get_control::<ListBox>("Stories");
    assert_eq!(stories.items().count(), 35);
    click_in_view(&shell, &window, &first_story(&window));
    tap("MobileSaved");
    assert_eq!(stories.items().count(), 35);
    assert!(!view
        .get_control::<Button>("LoadMoreButton")
        .is_effectively_visible());
    tap("MobileHome");
    let unread = view.get_control::<ToggleButton>("UnreadButton");
    tap("UnreadButton");
    assert_eq!(unread.is_checked(), Some(true));
    window.close();
}

#[test]
fn split_reading_desk_opens_next_unread_and_omits_hidden_stories() {
    let store = Rc::new(MemoryStore::default());
    let mut state = ReaderState::default();
    state.mark_read(1001, 1);
    state.toggle_hidden(1002);
    state.preferences.reduced_motion = true;
    ready(store.put("reader:state", &state.export_json().unwrap())).unwrap();
    let (shell, window, view) = start(1024.0, 768.0, store, Rc::new(Cell::new(false)));
    let tap = |name| click_in_view(&shell, &window, &view.get_control::<Button>(name));
    assert_eq!(
        view.get_control::<TextBlock>("WelcomeStoryLabel")
            .text()
            .as_deref(),
        Some("NEXT UNREAD")
    );
    assert_eq!(
        view.get_control::<TextBlock>("WelcomeHeadline")
            .text()
            .as_deref(),
        Some("A visual introduction to GPU text rendering")
    );
    if let Some(dir) = std::env::var_os("EMBER_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        shell
            .last_frame(0)
            .save_ppm(&std::path::PathBuf::from(dir).join("tablet-reading-desk.ppm"))
            .unwrap();
    }
    tap("WelcomeOpen");
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("A visual introduction to GPU text rendering")
    );
    tap("BackButton");
    assert!(view
        .get_control::<ScrollViewer>("WelcomePane")
        .is_effectively_visible());
    assert_ne!(
        view.get_control::<TextBlock>("WelcomeHeadline")
            .text()
            .as_deref(),
        Some("A visual introduction to GPU text rendering")
    );
    tap("NavHistory");
    assert_eq!(
        view.get_control::<TextBlock>("WelcomeStoryLabel")
            .text()
            .as_deref(),
        Some("LAST OPENED")
    );
    assert_eq!(
        view.get_control::<TextBlock>("WelcomeHeadline")
            .text()
            .as_deref(),
        Some("A visual introduction to GPU text rendering")
    );
    tap("NavHidden");
    assert!(!view
        .get_control::<ferroui_controls::Border>("WelcomeStory")
        .is_effectively_visible());
    tap("WelcomeSaved");
    assert_eq!(title(&view), "Saved for later");
    assert!(!view
        .get_control::<Button>("WelcomeSaved")
        .is_effectively_visible());
    tap("WelcomeSearch");
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("Search")
    );
    window.close();
}

#[test]
fn keyboard_navigation_keeps_its_place_after_opening_an_unread_story() {
    use ferroui_base::input::{InputElement, Key, KeyEventArgs};
    let store = Rc::new(MemoryStore::default());
    let mut state = ReaderState::default();
    state.preferences.hide_read = true;
    for id in 1005..=1012 {
        state.mark_read(id, id);
    }
    ready(store.put("reader:state", &state.export_json().unwrap())).unwrap();
    let (shell, window, view) = start(1440.0, 940.0, store, Rc::new(Cell::new(false)));
    let third = window
        .get_visual_descendants()
        .filter_map(|v| v.cast::<crate::controls::StoryCard>())
        .find(|card| card.tag().and_then(|id| id.downcast_ref::<u64>().copied()) == Some(1003))
        .unwrap();
    click_in_view(&shell, &window, &third);
    let key = |key| {
        let mut event = KeyEventArgs::new();
        event.key = key;
        event.set_routed_event(Some(InputElement::key_down_event()));
        view.raise_event(&event);
        shell.settle();
        assert!(event.handled());
        view.get_control::<TextBlock>("DetailTitle").text().unwrap()
    };
    assert_eq!(
        key(Key::K),
        "Ask HN: What changed the way you think about programming?"
    );
    assert_eq!(
        key(Key::J),
        "Show HN: I made a tiny database for the browser"
    );
    // There are still unread stories before this one, but J must stop at the end.
    assert_eq!(
        key(Key::J),
        "Show HN: I made a tiny database for the browser"
    );
    window.close();
}
