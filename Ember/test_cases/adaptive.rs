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
