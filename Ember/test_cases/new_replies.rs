use super::*;
use ferroui_controls::primitives::ToggleButton;

#[test]
fn return_visit_shows_new_nested_replies_without_changing_folded_branches() {
    let store = Rc::new(MemoryStore::default());
    let mut state = ReaderState::default();
    state.preferences.reduced_motion = true;
    let first_visit = state.items.entry(1001).or_default();
    first_visit.seen_comments.extend([1001, 2001, 2003, 2004]);
    first_visit.collapsed.insert(2001);
    ready(store.put("reader:state", &state.export_json().unwrap())).unwrap();
    let (shell, window, view) = start(1024.0, 820.0, store.clone(), Rc::new(Cell::new(false)));
    click_in_view(&shell, &window, &first_story(&window));
    let scope = || {
        view.get_visual_descendants()
            .filter_map(|node| node.cast::<ToggleButton>())
            .find(|b| b.name().as_deref() == Some("NewReplyFilter"))
            .unwrap()
    };
    let list = || {
        view.get_visual_descendants()
            .filter_map(|node| node.cast::<ListBox>())
            .find(|list| list.classes().contains("thread-list"))
            .unwrap()
    };
    let has_comment = |author: &str| {
        list()
            .get_visual_descendants()
            .filter_map(|v| v.cast::<ferroui_controls::Border>())
            .any(|b| {
                AutomationProperties::get_name(&b)
                    .is_some_and(|name| name.starts_with(&format!("Comment by {author},")))
            })
    };
    assert_eq!(list().items().count(), 4); // Header, collapsed Lin, Mira, Noor.
    click_in_view(&shell, &window, &scope());
    assert_eq!(scope().is_checked(), Some(true));
    assert!(scope().is_keyboard_focus_within());
    assert_eq!(list().items().count(), 3); // Header, Lin for context, Ada's new reply.
    assert!(has_comment("lin") && has_comment("ada") && !has_comment("mira"));
    assert!(list()
        .get_visual_descendants()
        .filter_map(|v| v.cast::<TextBlock>())
        .any(|t| t.text().as_deref() == Some("1 new reply")));
    if let Some(dir) = std::env::var_os("EMBER_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        shell
            .last_frame(0)
            .save_ppm(&std::path::Path::new(&dir).join("tablet-new-replies.ppm"))
            .unwrap();
    }

    // Find searches the full discussion, even when the current scope omits this author.
    click(&shell, &window, &view.get_control::<Button>("FindButton"));
    let field = view.get_control::<TextBox>("ThreadFind");
    field.set_text(Some("mira"));
    use ferroui_base::input::{InputElement, Key, KeyEventArgs};
    let mut enter = KeyEventArgs::new();
    enter.key = Key::Enter;
    enter.set_routed_event(Some(InputElement::key_down_event()));
    field.raise_event(&enter);
    shell.settle();
    assert_eq!(
        view.get_control::<TextBlock>("FindCount").text().as_deref(),
        Some("1 matching comment")
    );
    click(&shell, &window, &view.get_control::<Button>("CloseFind"));
    assert_eq!(scope().is_checked(), Some(true));
    assert_eq!(list().items().count(), 3);
    click_in_view(&shell, &window, &scope());
    assert_eq!(list().items().count(), 4);
    assert!(!has_comment("ada"));
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    let saved =
        ReaderState::import_json(&ready(store.get("reader:state")).unwrap().unwrap()).unwrap();
    assert!(saved.items[&1001].collapsed.contains(&2001));
    assert!(saved.items[&1001].seen_comments.contains(&2002));
    window.close();
}
