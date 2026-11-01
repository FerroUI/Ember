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
        .set_text(Some("profile-context"));
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
