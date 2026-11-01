use super::*;
use ferroui_controls::primitives::ToggleButton;

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
