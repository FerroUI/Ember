use super::*;
use crate::controls::StoryCard;
use ferroui_base::input::{SwipeGestureEndedEventArgs, SwipeGestureEventArgs};
use ferroui_base::Vector;
use ferroui_controls::Border;

#[test]
fn swipe_preview_tracks_direction_arming_and_the_current_story_state() {
    let store = Rc::new(MemoryStore::default());
    let (shell, window, view) = start(390.0, 800.0, store.clone(), Rc::new(Cell::new(false)));
    let card = first_story(&window);
    let reveal = card
        .get_visual_descendants()
        .filter_map(|v| v.cast::<Border>())
        .find(|v| v.name().as_deref() == Some("SwipeReveal"))
        .unwrap();
    let saved = || {
        ReaderState::import_json(
            &ready(store.get("reader:state"))
                .unwrap()
                .unwrap_or_else(|| "{}".into()),
        )
        .unwrap()
        .is_saved(1001)
    };
    let drag = |dx: f64| {
        card.raise_event(&SwipeGestureEventArgs::new(
            1,
            Vector::new(-dx, 0.0),
            Vector::default(),
        ));
        shell.settle();
    };
    let release = || {
        card.raise_event(&SwipeGestureEndedEventArgs::new(1, Vector::default()));
        shell.settle();
    };
    drag(-48.0);
    assert!(reveal.is_effectively_visible());
    assert_eq!(
        card.get_value(StoryCard::swipe_label_property()).as_deref(),
        Some("Save")
    );
    assert!(!card.classes().contains("swipe-ready"));
    drag(-54.0);
    assert!(card.classes().contains("swipe-ready"));
    assert!(!saved());
    if let Some(dir) = std::env::var_os("EMBER_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        shell
            .last_frame(0)
            .save_ppm(&std::path::Path::new(&dir).join("phone-swipe-preview.ppm"))
            .unwrap();
    }
    drag(79.0);
    assert!(!card.classes().contains("swipe-ready"));
    release();
    assert!(!saved());
    assert!(!reveal.is_effectively_visible());
    drag(-120.0);
    release();
    assert!(saved());
    drag(-90.0);
    assert_eq!(
        card.get_value(StoryCard::swipe_label_property()).as_deref(),
        Some("Unsave")
    );
    drag(130.0);
    assert_eq!(
        card.get_value(StoryCard::swipe_label_property()).as_deref(),
        Some("Hide")
    );
    release();
    assert!(saved());
    assert!(view
        .get_control::<Button>("MobileHome")
        .is_effectively_visible());
    window.close();
}

#[test]
fn compact_story_rows_share_one_metadata_line_and_keep_full_accessible_descriptions() {
    let store = Rc::new(MemoryStore::default());
    let mut state = ReaderState::default();
    state.preferences.compact = true;
    state.toggle_saved(1001, 1);
    ready(store.put("reader:state", &state.export_json().unwrap())).unwrap();
    let (shell, window, _) = start(390.0, 800.0, store, Rc::new(Cell::new(false)));
    let card = first_story(&window);
    let block = |name: &str| {
        card.get_visual_descendants()
            .filter_map(|v| v.cast::<TextBlock>())
            .find(|v| v.name().as_deref() == Some(name))
            .unwrap()
    };
    let compact = block("CompactByline");
    assert!(compact.is_effectively_visible());
    assert!(!block("Byline").is_effectively_visible());
    assert!(!block("Source").is_effectively_visible());
    assert!(compact.text().unwrap().contains("example.com · ada ·"));
    assert!(AutomationProperties::get_name(&card)
        .unwrap()
        .contains("326 points, 4 comments"));
    let height = card.bounds().height;
    assert!(height >= 44.0);
    if let Some(dir) = std::env::var_os("EMBER_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        shell
            .last_frame(0)
            .save_ppm(&std::path::Path::new(&dir).join("phone-compact-feed.ppm"))
            .unwrap();
    }
    card.classes().set("compact", false);
    shell.settle();
    assert!(block("Source").is_effectively_visible());
    assert!(card.bounds().height >= height + 10.0);
    window.close();
}
