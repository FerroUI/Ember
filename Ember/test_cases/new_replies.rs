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
    // A profile detour advances the durable seen set when leaving the thread.
    // Back must still return to this visit, including the original catch-up scope
    // beneath Find and the selected match, rather than starting a fresh visit.
    field.set_text(Some("IDEA "));
    let submit = || {
        let mut key = KeyEventArgs::new();
        key.key = Key::Enter;
        key.set_routed_event(Some(InputElement::key_down_event()));
        field.raise_event(&key);
        shell.settle();
    };
    submit();
    submit();
    let count = view.get_control::<TextBlock>("FindCount");
    assert_eq!(count.text().as_deref(), Some("2 of 2 matching comments"));
    let author = list()
        .get_visual_descendants()
        .filter_map(|v| v.cast::<Button>())
        .find(|button| {
            AutomationProperties::get_name(button).as_deref() == Some("View ada’s profile")
        })
        .expect("second matching reply belongs to ada");
    click_in_view(&shell, &window, &author);
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("ada")
    );
    click_in_view(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert!(field.is_effectively_visible());
    assert!(field.is_focused());
    assert_eq!(field.text().as_deref(), Some("IDEA "));
    assert_eq!(count.text().as_deref(), Some("2 of 2 matching comments"));
    click(&shell, &window, &view.get_control::<Button>("CloseFind"));
    assert_eq!(scope().is_checked(), Some(true));
    assert_eq!(list().items().count(), 3);
    assert!(has_comment("ada") && !has_comment("mira"));
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

#[test]
fn keyboard_folding_and_sorting_keep_focus_on_the_replacement_control() {
    use ferroui_base::input::{InputElement, Key, KeyEventArgs};
    use ferroui_controls::ComboBox;
    let (shell, window, view) = start(
        1024.0,
        820.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    click_in_view(&shell, &window, &first_story(&window));
    let list = || {
        view.get_visual_descendants()
            .filter_map(|v| v.cast::<ListBox>())
            .find(|list| list.classes().contains("thread-list"))
            .unwrap()
    };
    let tool = |row: i32, name: &str| {
        list()
            .container_from_index(row)
            .unwrap()
            .get_visual_descendants()
            .filter_map(|v| v.cast::<Control>())
            .find(|c| c.name().as_deref() == Some(name))
            .unwrap()
    };
    let key = |c: &Control, key| {
        let mut event = KeyEventArgs::new();
        event.key = key;
        event.set_routed_event(Some(InputElement::key_down_event()));
        c.raise_event(&event);
        for _ in 0..4 {
            shell.settle();
        }
    };

    // Both top-level fixture comments have one child. A global name lookup
    // would focus Lin's first disclosure when it was Mira's control that changed.
    let second = tool(3, "FoldReply");
    second.focus();
    key(&second, Key::Enter);
    let expand = tool(3, "FoldReply");
    assert!(expand.is_focused());
    assert_eq!(
        AutomationProperties::get_name(&expand).as_deref(),
        Some("Expand comment and 1 reply")
    );
    key(&expand, Key::Enter);
    assert!(tool(3, "FoldReply").is_focused());
    let sort = tool(0, "ThreadSort");
    sort.focus();
    key(&sort, Key::Down);
    let sorted = tool(0, "ThreadSort").cast::<ComboBox>().unwrap();
    assert!(sorted.is_focused());
    assert_eq!(sorted.selected_index(), 1);
    let bulk = tool(0, "FoldAll");
    bulk.focus();
    key(&bulk, Key::Enter);
    assert!(tool(0, "FoldAll").is_focused());
    assert_eq!(list().items().count(), 3);
    window.close();
}

#[test]
fn reading_a_filtered_reply_in_context_opens_only_its_path_and_focuses_the_same_comment() {
    use ferroui_base::input::{InputElement, Key, KeyEventArgs, KeyModifiers};
    use ferroui_controls::Border;

    for (width, height, using_find) in [(390.0, 844.0, false), (1024.0, 820.0, true)] {
        let store = Rc::new(MemoryStore::default());
        let mut state = ReaderState::default();
        state.preferences.reduced_motion = true;
        let local = state.items.entry(1001).or_default();
        local.seen_comments.extend([1001, 2001, 2003, 2004]);
        local.collapsed.extend([2001, 2002, 2003]);
        ready(store.put("reader:state", &state.export_json().unwrap())).unwrap();
        let http = Rc::new(CountingHttp {
            fixture: FixtureHttp {
                offline: Rc::new(Cell::new(false)),
            },
            requests: Cell::new(0),
        });
        let (shell, window, view) = start_with_http(width, height, store.clone(), http.clone());
        click_in_view(&shell, &window, &first_story(&window));
        let requests = http.requests.get();
        let conversation = || {
            view.get_visual_descendants()
                .filter_map(|node| node.cast::<ListBox>())
                .find(|list| list.classes().contains("thread-list"))
                .unwrap()
        };

        if using_find {
            click(&shell, &window, &view.get_control::<Button>("FindButton"));
            let field = view.get_control::<TextBox>("ThreadFind");
            field.set_text(Some("predictable"));
            // Submit during debounce, before the replacement list exists.
            let mut enter = KeyEventArgs::new();
            enter.key = Key::Enter;
            enter.key_modifiers = KeyModifiers::CONTROL;
            enter.set_routed_event(Some(InputElement::key_down_event()));
            field.raise_event(&enter);
        } else {
            let filter = conversation()
                .get_visual_descendants()
                .filter_map(|node| node.cast::<ToggleButton>())
                .find(|b| b.name().as_deref() == Some("NewReplyFilter"))
                .unwrap();
            click_in_view(&shell, &window, &filter);
            let filtered = conversation();
            filtered.scroll_into_view(2);
            for _ in 0..4 {
                shell.settle();
            }
            let action = filtered
                .container_from_index(2)
                .unwrap()
                .get_visual_descendants()
                .filter_map(|node| node.cast::<Button>())
                .find(|b| AutomationProperties::get_name(b).as_deref() == Some("Read in context"))
                .unwrap();
            click_in_view(&shell, &window, &action);
        }
        for _ in 0..5 {
            shell.settle();
        }
        assert!(!view.get_control::<Border>("ThreadTools").is_visible());
        let list = conversation();
        assert_eq!(list.items().count(), 4); // Header, Lin, Ada (fully open), folded Mira.
        let target = list.container_from_index(2).unwrap();
        assert!(
            target.is_keyboard_focus_within(),
            "the activated comment must receive focus"
        );
        assert!(target
            .get_visual_descendants()
            .filter_map(|v| v.cast::<Border>())
            .any(|b| AutomationProperties::get_name(&b).as_deref()
                == Some("Comment by ada, level 2")));
        assert_eq!(http.requests.get(), requests);
        let saved =
            ReaderState::import_json(&ready(store.get("reader:state")).unwrap().unwrap()).unwrap();
        assert_eq!(
            saved.items[&1001]
                .collapsed
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![2003]
        );
        window.close();
    }
}

#[test]
fn browser_slots_for_the_same_item_restore_distinct_find_drafts_in_both_directions() {
    let (shell, window, view) = start(
        390.0,
        844.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    let field = view.get_control::<TextBox>("ThreadFind");
    let route = |item: u64, entry| {
        crate::reader_view::navigate_browser_entry(&format!("?item={item}"), entry, false);
        for _ in 0..4 {
            shell.settle();
        }
    };
    let open_find = |text| {
        click_in_view(&shell, &window, &view.get_control::<Button>("FindButton"));
        field.set_text(Some(text));
        shell.settle();
    };
    route(1001, Some(11));
    open_find("First visit");
    route(1002, Some(12));
    route(1001, Some(13));
    assert!(!field.is_effectively_visible());
    open_find("Second visit");

    // Back and Forward can each jump multiple browser entries with one event.
    route(1001, Some(11));
    assert!(field.is_effectively_visible());
    assert_eq!(field.text().as_deref(), Some("First visit"));
    route(1001, Some(13));
    assert!(field.is_effectively_visible());
    assert_eq!(field.text().as_deref(), Some("Second visit"));
    let back = view.get_control::<Button>("BackButton");
    click_in_view(&shell, &window, &back); // close Find
    click_in_view(&shell, &window, &back);
    let title = view.get_control::<TextBlock>("DetailTitle");
    assert_eq!(
        title.text().as_deref(),
        Some("Ask HN: What changed the way you think about programming?")
    );
    route(1001, Some(13));
    assert_eq!(title.text().as_deref(), Some("Small tools, carefully made"));
    route(1002, Some(12));
    route(1003, Some(14)); // fresh branch from B
    route(1001, Some(11));
    route(1003, Some(14));
    click_in_view(&shell, &window, &back);
    assert_eq!(
        title.text().as_deref(),
        Some("Ask HN: What changed the way you think about programming?")
    );
    route(1001, None);
    assert!(!field.is_effectively_visible());
    // A tab visit disconnects the local back stack. Traversing backwards to
    // older keyed entries must not add the newer pages as toolbar predecessors.
    click_in_view(&shell, &window, &view.get_control::<Button>("MobileHome"));
    for (id, slot) in [(1003, 14), (1002, 12)] {
        crate::reader_view::navigate_browser_entry(&format!("?item={id}"), Some(slot), true);
        for _ in 0..4 {
            shell.settle();
        }
    }
    click_in_view(&shell, &window, &back);
    assert!(view
        .get_control::<ListBox>("Stories")
        .is_effectively_visible());
    window.close();
}
