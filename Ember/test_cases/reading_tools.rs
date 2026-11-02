use super::*;

#[test]
fn inline_story_and_profile_links_open_in_ember_and_preserve_back_history() {
    let store = Rc::new(MemoryStore::default());
    let now = crate::services::now();
    for (key, value) in [
        (
            "cache:item:1001",
            json!({"id":1001,"type":"story","title":"Typesetting a pocket guide","by":"ada","text":"<p>Read <a href='/user?id=ada'>Ada’s profile notes</a> for the recipe.</p>","kids":[]}),
        ),
        (
            "cache:user:ada",
            json!({"id":"ada","karma":8240,"created":1451606400,"about":"<p>My <a href='/item?id=2002'>original comment</a> and the layout function:</p><pre>fn column(width: f64, gutter: f64, count: f64) -&gt; f64 {\n    (width - gutter * (count - 1.0)) / count\n}</pre>","submitted":[2002]}),
        ),
    ] {
        ready(store.put(key, &json!({"fetched_at":now,"value":value}).to_string())).unwrap();
    }
    let (shell, window, view) = start(360.0, 720.0, store, Rc::new(Cell::new(false)));
    let detail = view.get_control::<TextBlock>("DetailTitle");
    let tap = |label: &str| {
        let button = view
            .get_visual_descendants()
            .filter_map(|c| c.cast::<Button>())
            .find(|b| {
                b.is_effectively_visible()
                    && AutomationProperties::get_name(b).as_deref() == Some(label)
            })
            .unwrap_or_else(|| panic!("expected {label}"));
        click_in_view(&shell, &window, &button);
    };
    click(&shell, &window, &first_story(&window));
    assert_eq!(detail.text().as_deref(), Some("Typesetting a pocket guide"));
    tap("Ada’s profile notes");
    assert_eq!(detail.text().as_deref(), Some("ada"));
    tap("Wrap code lines");
    if let Some(dir) = std::env::var_os("EMBER_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        shell
            .last_frame(0)
            .save_ppm(&std::path::Path::new(&dir).join("phone-code-tools.ppm"))
            .unwrap();
    }
    tap("original comment");
    assert_eq!(detail.text().as_deref(), Some("Comment by ada"));
    tap("Back");
    assert_eq!(detail.text().as_deref(), Some("ada"));
    tap("Back");
    assert_eq!(detail.text().as_deref(), Some("Typesetting a pocket guide"));
    window.close();
}

#[test]
fn nested_reply_actions_skip_descendants_and_open_an_independent_comment_thread() {
    use ferroui_base::interactivity::RoutedEventArgs;
    use ferroui_controls::{MenuFlyout, MenuItem};

    let store = Rc::new(MemoryStore::default());
    let now = crate::services::now();
    // Ada is both the story submitter and the parent of two replies. The first
    // reply has a child, so its next sibling is not the next visible row.
    for (id, parent, author, kids) in [
        (2001, 1001, "ada", vec![2002, 2005]),
        (2002, 2001, "ada", vec![2006]),
        (2005, 2001, "lin", vec![]),
        (2006, 2002, "mira", vec![]),
    ] {
        let payload = json!({"fetched_at":now,"value":{
            "id":id,"type":"comment","parent":parent,"by":author,"kids":kids,
            "text":format!("<p>Reply {id}</p>")
        }});
        ready(store.put(&format!("cache:item:{id}"), &payload.to_string())).unwrap();
    }
    let (shell, window, view) = start(390.0, 700.0, store, Rc::new(Cell::new(false)));
    click_in_view(&shell, &window, &first_story(&window));
    let conversation = || {
        view.get_visual_descendants()
            .filter_map(|node| node.cast::<ListBox>())
            .find(|list| list.classes().contains("thread-list"))
            .unwrap()
    };
    let invoke = |index: i32, command: &str| {
        let list = conversation();
        list.scroll_into_view(index);
        for _ in 0..4 {
            shell.settle();
        }
        let row = list.container_from_index(index).unwrap();
        let button = row
            .get_visual_descendants()
            .filter_map(|node| node.cast::<Button>())
            .find(|b| AutomationProperties::get_name(b).as_deref() == Some("More comment actions"))
            .unwrap();
        let menu = button.flyout().unwrap().cast::<MenuFlyout>().unwrap();
        menu.show_at(&button);
        shell.settle();
        let items = menu.items();
        let action = (0..items.count())
            .filter_map(|i| Control::from_boxed(&items.get_at(i)?))
            .filter_map(|c| c.cast::<MenuItem>())
            .find(|item| {
                item.header()
                    .and_then(|v| v.downcast_ref::<String>().cloned())
                    .as_deref()
                    == Some(command)
            })
            .unwrap_or_else(|| panic!("missing {command}"));
        action.raise_event(&RoutedEventArgs::with_event(MenuItem::click_event()));
        menu.hide();
        for _ in 0..5 {
            shell.settle();
        }
    };
    let has_op = |index| {
        conversation()
            .container_from_index(index)
            .unwrap()
            .get_visual_descendants()
            .any(|node| AutomationProperties::get_name(&node).as_deref() == Some("Original poster"))
    };
    invoke(2, "Next reply to this parent");
    assert!(conversation()
        .container_from_index(4)
        .unwrap()
        .is_keyboard_focus_within());
    invoke(4, "Previous reply to this parent");
    assert!(conversation()
        .container_from_index(2)
        .unwrap()
        .is_keyboard_focus_within());
    assert!(has_op(2));
    // The final reply to this parent has no children. Fold it through the
    // overflow, then expand from its disclosure and keep focus in the same row.
    invoke(4, "Fold comment");
    let row_tool = |index, name: &str| {
        conversation()
            .container_from_index(index)
            .unwrap()
            .get_visual_descendants()
            .filter_map(|v| v.cast::<Button>())
            .find(|b| b.name().as_deref() == Some(name))
            .unwrap()
    };
    let expand = row_tool(4, "FoldReply");
    assert!(expand.is_focused());
    assert_eq!(
        AutomationProperties::get_name(&expand).as_deref(),
        Some("Expand comment")
    );
    click_in_view(&shell, &window, &expand);
    assert!(row_tool(4, "CommentMenu").is_focused());
    click_in_view(&shell, &window, &row_tool(4, "CommentPermalink"));
    let title = view.get_control::<TextBlock>("DetailTitle");
    assert_eq!(title.text().as_deref(), Some("Comment by lin"));
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert_eq!(title.text().as_deref(), Some("Small tools, carefully made"));
    invoke(1, "Open comment thread");
    assert_eq!(title.text().as_deref(), Some("Comment by ada"));
    assert!(
        !has_op(1),
        "a repeat author in a standalone reply thread is not the story's known submitter"
    );
    invoke(1, "Back to opening comment");
    assert!(conversation()
        .container_from_index(0)
        .unwrap()
        .is_keyboard_focus_within());
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert_eq!(title.text().as_deref(), Some("Small tools, carefully made"));
    window.close();
}

#[test]
fn back_restores_the_extended_discussion_and_its_original_comment_search_row() {
    use crate::thread_view::{restore_position, viewport_position, ReadingPosition};
    use ferroui_base::input::{InputElement, Key, KeyEventArgs};
    use ferroui_controls::ComboBox;

    let store = Rc::new(MemoryStore::default());
    let now = crate::services::now();
    let cache = |id: u64, item| {
        let data = json!({"fetched_at":now, "value":item}).to_string();
        ready(store.put(&format!("cache:item:{id}"), &data)).unwrap();
    };
    let kids: Vec<u64> = std::iter::once(2001).chain(5000..5160).collect();
    cache(
        1001,
        json!({"id":1001,"type":"story","title":"A long discussion","kids":kids}),
    );
    for id in 5000..5160 {
        cache(
            id,
            json!({"id":id, "type":"comment", "parent":1001,
            "by":format!("reader{id}"), "text":"A reply further into the discussion."}),
        );
    }
    let (shell, window, view) = start(1024.0, 820.0, store, Rc::new(Cell::new(false)));
    click_in_view(
        &shell,
        &window,
        &view.get_control::<Button>("CompactSearch"),
    );
    view.get_visual_descendants()
        .filter_map(|v| v.cast::<ComboBox>())
        .find(|c| AutomationProperties::get_name(c).as_deref() == Some("Search in"))
        .unwrap()
        .set_selected_index(1);
    let field = view
        .get_visual_descendants()
        .filter_map(|v| v.cast::<TextBox>())
        .find(|f| AutomationProperties::get_name(f).as_deref() == Some("Search terms"))
        .unwrap();
    field.set_text(Some("predictable"));
    let mut enter = KeyEventArgs::new();
    enter.key = Key::Enter;
    enter.set_routed_event(Some(InputElement::key_down_event()));
    field.raise_event(&enter);
    for _ in 0..5 {
        shell.settle();
    }
    let source = first_story(&window);
    assert_eq!(
        source.tag().and_then(|v| v.downcast_ref::<u64>().copied()),
        Some(2002)
    );
    click_in_view(&shell, &window, &source);

    let discussion = || {
        view.get_visual_descendants()
            .filter_map(|node| node.cast::<ListBox>())
            .find(|list| list.classes().contains("thread-list"))
            .unwrap()
    };
    let list = discussion();
    list.scroll_into_view(list.items().count() as i32 - 1);
    for _ in 0..5 {
        shell.settle();
    }
    let load = list
        .get_visual_descendants()
        .filter_map(|v| v.cast::<Button>())
        .find(|b| AutomationProperties::get_name(b).as_deref() == Some("Load more comments"))
        .unwrap();
    click_in_view(&shell, &window, &load);
    let expected = ReadingPosition::start(5135);
    restore_position(&discussion(), expected, false);
    for _ in 0..5 {
        shell.settle();
    }
    assert_eq!(viewport_position(&discussion()), Some(expected));

    for browser_back in [false, true] {
        crate::navigate_location("?item=1002");
        for _ in 0..5 {
            shell.settle();
        }
        if browser_back {
            crate::navigate_location("?item=1001");
        } else {
            click_in_view(&shell, &window, &view.get_control::<Button>("BackButton"));
        }
        for _ in 0..5 {
            shell.settle();
        }
        let list = discussion();
        assert_eq!(viewport_position(&list), Some(expected));
        assert!(
            list.get_visual_descendants()
                .filter_map(|node| node.cast::<Button>())
                .any(|b| AutomationProperties::get_name(&b).as_deref()
                    == Some("View reader5135’s profile")),
            "the restored anchor must contain the loaded comment, not a placeholder"
        );
        assert!(
            first_story(&window).classes().contains("selected"),
            "keep the original Algolia row selected"
        );
    }
    window.close();
}
