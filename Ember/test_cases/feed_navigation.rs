use super::*;
use ferroui_controls::{primitives::ToggleButton, ScrollViewer};

#[test]
fn unread_discovery_skips_filtered_pages_in_bounded_batches_and_preserves_loaded_rows() {
    use ferroui_base::input::{InputElement, Key, KeyEventArgs};

    struct PagedFeed;
    impl HttpClient for PagedFeed {
        fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, ReaderError>> {
            Box::pin(async move {
                if url.ends_with("topstories.json") {
                    return Ok(json!((1..=181).collect::<Vec<_>>()).to_string());
                }
                let id: u64 = url
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .trim_end_matches(".json")
                    .parse()
                    .unwrap();
                let by = if id == 61 { "muted" } else { "reader" };
                Ok(
                    json!({"id":id,"type":"story","title":format!("Story {id}"),"by":by})
                        .to_string(),
                )
            })
        }
    }

    let store = Rc::new(MemoryStore::default());
    let mut state = ReaderState::default();
    for id in 1..=181 {
        if !matches!(id, 61 | 78 | 136 | 175) {
            state.mark_read(id, id);
        }
    }
    state.toggle_hidden(78);
    state.mutes.users.insert("muted".to_owned());
    ready(store.put("reader:state", &state.export_json().unwrap())).unwrap();
    let (shell, window, view) = start_with_http(390.0, 700.0, store, Rc::new(PagedFeed));
    let rows = view.get_control::<ListBox>("Stories");
    let unread = view.get_control::<ToggleButton>("UnreadButton");
    let more = view.get_control::<Button>("LoadMoreButton");
    let toggle = || {
        let mut e = KeyEventArgs::new();
        e.key = Key::U;
        e.set_routed_event(Some(InputElement::key_down_event()));
        view.raise_event(&e);
        shell.settle();
        assert!(e.handled());
    };
    let next = || click_in_view(&shell, &window, &more);

    assert_eq!(rows.items().count(), 30);
    toggle();
    assert_eq!(
        AutomationProperties::get_name(&more).as_deref(),
        Some("Find more unread")
    );
    assert_eq!(rows.items().count(), 0);
    next(); // Three more pages, all read, muted, or hidden.
    assert_eq!(rows.items().count(), 0);
    toggle();
    assert_eq!(
        rows.items().count(),
        118,
        "the search must stop after three pages"
    );
    toggle();
    next(); // Stop at the first matching page, before story 175.
    assert_eq!(rows.items().count(), 1);
    assert_eq!(
        first_story(&window).headline().as_deref(),
        Some("Story 136")
    );
    assert_eq!(
        AutomationProperties::get_help_text(&unread).as_deref(),
        Some("1 unread story on the loaded pages")
    );
    next();
    assert_eq!(rows.items().count(), 2);
    next();
    assert!(!more.is_effectively_visible());
    assert_eq!(rows.items().count(), 2);
    toggle();
    assert_eq!(rows.items().count(), 179);
    assert_eq!(
        AutomationProperties::get_help_text(&unread).as_deref(),
        Some("2 unread stories on the loaded pages")
    );
    window.close();
}

#[test]
fn opening_a_comment_search_result_updates_unread_without_duplicating_history() {
    use ferroui_base::input::{InputElement, Key, KeyEventArgs};
    use ferroui_controls::ComboBox;

    let store = Rc::new(MemoryStore::default());
    let mut state = ReaderState::default();
    state.preferences.hide_read = true;
    state
        .items
        .entry(1001)
        .or_default()
        .collapsed
        .extend([2001, 2003]);
    ready(store.put("reader:state", &state.export_json().unwrap())).unwrap();
    let (shell, window, view) = start(390.0, 700.0, store.clone(), Rc::new(Cell::new(false)));
    click_in_view(&shell, &window, &view.get_control::<Button>("MobileSearch"));
    view.get_visual_descendants()
        .filter_map(|e| e.cast::<ComboBox>())
        .find(|e| AutomationProperties::get_name(e).as_deref() == Some("Search in"))
        .unwrap()
        .set_selected_index(1); // Comments
    let terms = view
        .get_visual_descendants()
        .filter_map(|e| e.cast::<TextBox>())
        .find(|e| AutomationProperties::get_name(e).as_deref() == Some("Search terms"))
        .unwrap();
    terms.set_text(Some("predictable"));
    let mut enter = KeyEventArgs::new();
    enter.key = Key::Enter;
    enter.set_routed_event(Some(InputElement::key_down_event()));
    terms.raise_event(&enter);
    for _ in 0..5 {
        shell.settle();
    }
    click_in_view(&shell, &window, &first_story(&window));
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("Small tools, carefully made")
    );
    click_in_view(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert_eq!(view.get_control::<ListBox>("Stories").items().count(), 0);
    let raw = ready(store.get("reader:state")).unwrap().unwrap();
    let state = ReaderState::import_json(&raw).unwrap();
    assert!(state.is_read(2002));
    assert_eq!(state.history_ids(), [1001]);
    assert_eq!(state.items[&1001].collapsed, [2003].into_iter().collect());
    window.close();
}

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

#[test]
fn library_find_scans_beyond_loaded_pages_and_restores_browsing_without_network() {
    use ferroui_base::input::{InputElement, Key, KeyEventArgs};
    use ferroui_base::Vector;
    let store = Rc::new(MemoryStore::default());
    let http = Rc::new(CountingHttp {
        fixture: FixtureHttp {
            offline: Rc::new(Cell::new(false)),
        },
        requests: Cell::new(0),
    });
    let mut state = ReaderState::default();
    for id in 5000..5090 {
        state.toggle_saved(id, id);
        let headline = if id == 5000 {
            "The lost atlas &amp; its margin".to_owned()
        } else {
            format!("Atlas volume {id}")
        };
        let record = json!({"fetched_at":crate::services::now(),"value":{"id":id,"type":"story","title":headline,"by":"lin","url":"https://journal.example/atlas"}});
        ready(store.put(&format!("cache:item:{id}"), &record.to_string())).unwrap();
    }
    state.toggle_saved(4999, 0); // A foreign bookmark restored without its payload.
    ready(store.put("reader:state", &state.export_json().unwrap())).unwrap();
    let (shell, window, view) = start_with_http(390.0, 720.0, store.clone(), http.clone());
    let tap = |name| click_in_view(&shell, &window, &view.get_control::<Button>(name));
    let stories = view.get_control::<ListBox>("Stories");
    tap("MobileSaved");
    tap("LoadMoreButton");
    assert_eq!(stories.items().count(), 60);
    let scroll = stories.scroll().unwrap().cast::<ScrollViewer>().unwrap();
    scroll.set_offset(Vector::new(0.0, 610.0));
    shell.settle();
    tap("LibraryFindButton");
    let initial_offset = scroll.offset().y;
    assert!(initial_offset > 400.0);
    let field = view.get_control::<TextBox>("LibraryFind");
    let query = |value| {
        field.set_text(Some(value));
        let mut enter = KeyEventArgs::new();
        enter.key = Key::Enter;
        enter.set_routed_event(Some(InputElement::key_down_event()));
        field.raise_event(&enter);
        for _ in 0..5 {
            shell.settle();
        }
        assert!(enter.handled());
    };
    // A cache TTL can expire while Find is open. Closing it should restore the
    // already loaded browse window without refetching its pages.
    let raw = ready(store.get("cache:item:5089")).unwrap().unwrap();
    let mut old: serde_json::Value = serde_json::from_str(&raw).unwrap();
    old["fetched_at"] = json!(1);
    ready(store.put("cache:item:5089", &old.to_string())).unwrap();
    let requests = http.requests.get();
    query("ATLAS  lin   journal.example");
    assert_eq!(stories.items().count(), 30);
    tap("LoadMoreButton");
    assert_eq!(stories.items().count(), 60);
    query("lost atlas & margin");
    assert_eq!(stories.items().count(), 1);
    assert_eq!(
        first_story(&window).headline().as_deref(),
        Some("The lost atlas & its margin")
    );
    assert_eq!(
        view.get_control::<TextBlock>("FeedSummary")
            .text()
            .as_deref(),
        Some("1 match · 1 unavailable on this device")
    );
    if let Some(dir) = std::env::var_os("EMBER_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        shell
            .last_frame(0)
            .save_ppm(&std::path::Path::new(&dir).join("phone-library-find.ppm"))
            .unwrap();
    }
    field.focus();
    let mut down = KeyEventArgs::new();
    down.key = Key::Down;
    down.set_routed_event(Some(InputElement::key_down_event()));
    field.raise_event(&down);
    shell.settle();
    assert!(down.handled());
    assert!(first_story(&window).is_focused());
    assert!(field.is_effectively_visible());
    field.focus();
    query("no such title");
    assert_eq!(
        view.get_control::<TextBlock>("EmptyTitle")
            .text()
            .as_deref(),
        Some("No matches on this device")
    );
    tap("RetryButton");
    assert!(!field.is_effectively_visible());
    assert_eq!(stories.items().count(), 60);
    assert!((scroll.offset().y - initial_offset).abs() < 2.0);
    assert_eq!(http.requests.get(), requests);
    window.close();
}

#[test]
fn returning_to_a_public_feed_restores_pages_and_scroll_despite_a_late_response() {
    use ferroui_base::Vector;
    #[derive(Default)]
    struct DelayedLatest {
        release: Cell<bool>,
        waiting: RefCell<Option<Waker>>,
    }
    impl HttpClient for DelayedLatest {
        fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, ReaderError>> {
            Box::pin(async move {
                if url.ends_with("topstories.json") {
                    return Ok(json!((1..=75).collect::<Vec<_>>()).to_string());
                }
                if url.ends_with("newstories.json") {
                    poll_fn(|cx| {
                        if self.release.get() {
                            Poll::Ready(())
                        } else {
                            self.waiting.replace(Some(cx.waker().clone()));
                            Poll::Pending
                        }
                    })
                    .await;
                    return Ok(json!([901, 902]).to_string());
                }
                let id: u64 = url
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .trim_end_matches(".json")
                    .parse()
                    .unwrap();
                Ok(json!({"id":id,"type":"story","title":format!("Feed story {id}")}).to_string())
            })
        }
    }
    let http = Rc::new(DelayedLatest::default());
    let (shell, window, view) =
        start_with_http(390.0, 720.0, Rc::new(MemoryStore::default()), http.clone());
    let tap = |name| click_in_view(&shell, &window, &view.get_control::<Button>(name));
    let list = view.get_control::<ListBox>("Stories");
    tap("LoadMoreButton");
    assert_eq!(list.items().count(), 60);
    let scroll = list.scroll().unwrap().cast::<ScrollViewer>().unwrap();
    scroll.set_offset(Vector::new(0.0, 1480.0));
    shell.settle();
    let position = scroll.offset().y;
    assert!(position > 1000.0);
    tap("MobileNew");
    assert!(http.waiting.borrow().is_some());
    tap("MobileHome");
    assert_eq!(list.items().count(), 60);
    assert!((scroll.offset().y - position).abs() < 2.0);
    http.release.set(true);
    http.waiting.borrow_mut().take().unwrap().wake();
    for _ in 0..6 {
        shell.settle();
    }
    assert_eq!(title(&view), "Front page");
    assert_eq!(list.items().count(), 60);
    assert!((scroll.offset().y - position).abs() < 2.0);
    tap("LoadMoreButton");
    let last = list.items().get_at(74).unwrap();
    assert_eq!(last.downcast_ref::<ember_core::HnItem>().unwrap().id, 75);
    assert!(!view
        .get_control::<Button>("LoadMoreButton")
        .is_effectively_visible());
    tap("MobileNew");
    assert_eq!(list.items().count(), 2);
    tap("MobileHome");
    assert_eq!(list.items().count(), 75);
    tap("MobileHome");
    assert_eq!(scroll.offset().y, 0.0);
    window.close();
}

#[test]
fn keyboard_story_actions_follow_the_focused_feed_row_in_split_view() {
    use ferroui_base::input::{InputElement, Key, KeyEventArgs};
    let store = Rc::new(MemoryStore::default());
    let (shell, window, view) = start(1440.0, 940.0, store.clone(), Rc::new(Cell::new(false)));
    let list = view.get_control::<ListBox>("Stories");
    click_in_view(&shell, &window, &first_story(&window));
    let second = list.container_from_index(1).unwrap();
    let card = second
        .get_visual_descendants()
        .find_map(|node| node.cast::<crate::controls::StoryCard>())
        .unwrap();
    assert!(card.focus());
    let key = |row: &Control, key| {
        let mut event = KeyEventArgs::new();
        event.key = key;
        event.set_routed_event(Some(InputElement::key_down_event()));
        row.raise_event(&event);
        for _ in 0..4 {
            shell.settle();
        }
        assert!(event.handled());
    };
    key(&card, Key::S);
    let state =
        || ReaderState::import_json(&ready(store.get("reader:state")).unwrap().unwrap()).unwrap();
    assert!(state().is_saved(1002));
    assert!(!state().is_saved(1001));
    click_in_view(&shell, &window, &view.get_control::<Button>("UndoStory"));
    assert!(!state().is_saved(1002));
    let discussion = view
        .get_control::<ferroui_controls::ContentControl>("DetailHost")
        .get_visual_descendants()
        .find_map(|node| node.cast::<ListBox>())
        .unwrap();
    key(&discussion.container_from_index(0).unwrap(), Key::S);
    assert!(state().is_saved(1001));
    assert!(!state().is_saved(1002));
    let second = list.container_from_index(1).unwrap();
    assert!(second.focus());
    key(&second, Key::J);
    let detail = view.get_control::<TextBlock>("DetailTitle");
    assert_eq!(
        detail.text().as_deref(),
        Some("A visual introduction to GPU text rendering")
    );
    let back = view.get_control::<Button>("BackButton");
    assert!(back.is_focused());
    key(&back, Key::J);
    assert_eq!(
        detail.text().as_deref(),
        Some("Show HN: I made a tiny database for the browser")
    );
    window.close();
}

#[test]
fn paginating_and_hiding_a_story_only_notify_the_affected_rows() {
    use ferroui_base::input::ContextRequestedEventArgs;
    use ferroui_base::interactivity::RoutedEventArgs;
    use ferroui_controls::MenuItem;

    let store = Rc::new(MemoryStore::default());
    let now = crate::services::now();
    let ids: Vec<u64> = (4000..4300).collect();
    ready(store.put(
        "cache:feed:topstories",
        &json!({"fetched_at":now,"value":ids}).to_string(),
    ))
    .unwrap();
    for &id in &ids {
        let item = json!({"id":id,"type":"story","title":format!("Story {id}"),"url":format!("https://example.org/{id}")});
        ready(store.put(
            &format!("cache:item:{id}"),
            &json!({"fetched_at":now,"value":item}).to_string(),
        ))
        .unwrap();
    }
    let (shell, window, view) = start(1280.0, 800.0, store, Rc::new(Cell::new(true)));
    let list = view.get_control::<ListBox>("Stories");
    assert_eq!(list.items().count(), 30);
    let original = list.items().get_at(0).unwrap();
    let second = list.items().get_at(1).unwrap();
    let changes = Rc::new(Cell::new((0_usize, 0_usize, 0_usize)));
    let counters = changes.clone();
    list.items().add_collection_changed(Rc::new(move |e| {
        let (events, added, removed) = counters.get();
        counters.set((
            events + 1,
            added + e.new_items.len(),
            removed + e.old_items.len(),
        ));
    }));
    let started = std::time::Instant::now();
    for _ in 0..9 {
        click_in_view(
            &shell,
            &window,
            &view.get_control::<Button>("LoadMoreButton"),
        );
    }
    let append_time = started.elapsed();
    let appended = changes.replace((0, 0, 0));
    assert_eq!(list.items().count(), 300);
    let retained = Rc::ptr_eq(&original, &list.items().get_at(0).unwrap());

    let card = first_story(&window);
    card.raise_event(&ContextRequestedEventArgs::new());
    let menu = card.context_menu().unwrap();
    let entries = menu.items();
    let hide = (0..entries.count())
        .filter_map(|i| Control::from_boxed(&entries.get_at(i)?))
        .filter_map(|c| c.cast::<MenuItem>())
        .find(|e| {
            e.header().is_some_and(|h| {
                h.downcast_ref::<String>().map(String::as_str) == Some("Hide story")
            })
        })
        .unwrap();
    menu.close();
    hide.raise_event(&RoutedEventArgs::with_event(MenuItem::click_event()));
    shell.settle();
    assert_eq!(list.items().count(), 299);
    eprintln!("300-story feed, nine page appends: {append_time:?}, notifications/additions/removals {appended:?}; hide one row: {:?}", changes.get());
    assert_eq!(appended, (9, 270, 0));
    assert_eq!(changes.get(), (1, 0, 1));
    assert!(retained);
    assert!(Rc::ptr_eq(&second, &list.items().get_at(0).unwrap()));
    window.close();
}

#[test]
fn later_feed_pages_recover_only_failed_ids_in_rank_order_and_retain_offline_viewport() {
    #[derive(Default)]
    struct InterruptedFeed {
        phase: Cell<u8>,
        offline: Cell<bool>,
        requested: RefCell<Vec<u64>>,
    }
    impl HttpClient for InterruptedFeed {
        fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, ReaderError>> {
            Box::pin(async move {
                if self.offline.get() {
                    if url.ends_with("/item/3.json") {
                        return Ok(
                            json!({"id":3,"type":"story","title":"Updated online"}).to_string()
                        );
                    }
                    return Err(ReaderError::Network("offline".into()));
                }
                if url.ends_with("/topstories.json") {
                    if self.phase.get() == 3 {
                        return Ok("[2,999]".to_owned());
                    }
                    let mut ranking: Vec<_> = (1..=65).collect();
                    ranking.swap(30, 31);
                    return Ok(json!(ranking).to_string());
                }
                if url.ends_with("/newstories.json") {
                    return Ok("[901]".to_owned());
                }
                let id: u64 = url
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .trim_end_matches(".json")
                    .parse()
                    .unwrap();
                self.requested.borrow_mut().push(id);
                match (self.phase.get(), id) {
                    (0, 31 | 32 | 37 | 38) | (1, 31 | 32) | (3, 2) => Err(ReaderError::Network("temporarily down".into())),
                    (_, 38) => Ok("null".to_owned()),
                    _ => Ok(json!({"id":id,"type":"story","by":"reader","title":format!("Article {id}")}).to_string()),
                }
            })
        }
    }

    let http = Rc::new(InterruptedFeed::default());
    let store = Rc::new(MemoryStore::default());
    let (shell, window, view) = start_with_http(390.0, 844.0, store.clone(), http.clone());
    let tap = |name| click_in_view(&shell, &window, &view.get_control::<Button>(name));
    let list = view.get_control::<ListBox>("Stories");
    let retry = view.get_control::<Button>("RetryFeedItems");
    assert_eq!(
        view.get_control::<TextBlock>("FeedSummary")
            .text()
            .as_deref(),
        Some("30 of 65 loaded · Updated")
    );
    tap("LoadMoreButton");
    assert_eq!(list.items().count(), 56);
    assert_eq!(
        AutomationProperties::get_name(&retry).as_deref(),
        Some("Retry 4 unavailable stories")
    );
    tap("LoadMoreButton");
    assert_eq!(list.items().count(), 61);
    assert!(!view
        .get_control::<Button>("LoadMoreButton")
        .is_effectively_visible());
    let summary = view.get_control::<TextBlock>("FeedSummary");
    assert_eq!(
        summary.text().as_deref(),
        Some("61 of 65 loaded · 4 unavailable")
    );
    assert!(
        !view
            .get_control::<ferroui_controls::Border>("StatusBar")
            .is_effectively_visible(),
        "the recovery command already explains a transient per-row failure"
    );
    if let Some(dir) = std::env::var_os("EMBER_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        shell
            .last_frame(0)
            .save_ppm(&std::path::Path::new(&dir).join("phone-feed-recovery.ppm"))
            .unwrap();
    }
    tap("MobileNew");
    assert!(!retry.is_effectively_visible());
    tap("MobileHome");
    assert!(retry.is_effectively_visible());
    assert_eq!(
        summary.text().as_deref(),
        Some("61 of 65 loaded · 4 unavailable")
    );
    let scroll = list.scroll().unwrap().cast::<ScrollViewer>().unwrap();
    scroll.set_offset(ferroui_base::Vector::new(0.0, 1050.0));
    shell.settle();
    let before = scroll.offset().y;
    assert!(before > 500.0);
    // Pruning can evict a payload which is still visible in memory, and another
    // visit can update a cached title without changing the saved feed ranking.
    ready(store.remove("cache:item:1")).unwrap();
    let changed = json!({"fetched_at": crate::services::now(), "value": {"id":2, "type":"story", "by":"reader", "title":"Revised title"}});
    ready(store.put("cache:item:2", &changed.to_string())).unwrap();
    http.offline.set(true);
    tap("RefreshButton");
    assert_eq!(
        list.items().count(),
        61,
        "a first-page offline fallback must not discard subsequent pages"
    );
    assert!((scroll.offset().y - before).abs() < 2.0);
    assert_eq!(
        summary.text().as_deref(),
        Some("61 of 65 loaded · 4 unavailable · Offline copy")
    );
    assert_eq!(
        list.items()
            .get_at(0)
            .unwrap()
            .downcast_ref::<ember_core::HnItem>()
            .unwrap()
            .id,
        1
    );
    assert_eq!(
        list.items()
            .get_at(1)
            .unwrap()
            .downcast_ref::<ember_core::HnItem>()
            .unwrap()
            .title
            .as_deref(),
        Some("Revised title")
    );
    assert_eq!(
        list.items()
            .get_at(2)
            .unwrap()
            .downcast_ref::<ember_core::HnItem>()
            .unwrap()
            .title
            .as_deref(),
        Some("Updated online")
    );
    http.offline.set(false);
    http.phase.set(1);
    http.requested.borrow_mut().clear();
    tap("RetryFeedItems");
    assert_eq!(http.requested.borrow().as_slice(), [32, 31, 37, 38]);
    assert_eq!(list.items().count(), 62);
    assert_eq!(
        summary.text().as_deref(),
        Some("62 of 65 loaded · 2 unavailable")
    );
    assert_eq!(
        AutomationProperties::get_name(&retry).as_deref(),
        Some("Retry 2 unavailable stories")
    );

    retry.focus();
    http.phase.set(2);
    http.requested.borrow_mut().clear();
    tap("RetryFeedItems");
    assert_eq!(http.requested.borrow().as_slice(), [32, 31]);
    assert!(!retry.is_effectively_visible());
    assert_eq!(summary.text().as_deref(), Some("64 of 65 loaded · Updated"));
    let ids: Vec<_> = (0..list.items().count())
        .map(|i| {
            list.items()
                .get_at(i)
                .unwrap()
                .downcast_ref::<ember_core::HnItem>()
                .unwrap()
                .id
        })
        .collect();
    let mut expected: Vec<_> = (1..=65).filter(|&id| id != 38).collect();
    expected.swap(30, 31);
    assert_eq!(ids, expected);
    let focused = list
        .get_visual_descendants()
        .filter_map(|row| row.cast::<crate::controls::StoryCard>())
        .find(|card| card.is_focused())
        .expect("focused recovered story");
    assert_eq!(focused.headline().as_deref(), Some("Article 32"));

    // A genuinely new ranking still applies even if one item needs a cache fallback.
    http.phase.set(3);
    tap("RefreshButton");
    assert_eq!(list.items().count(), 2);
    assert_eq!(
        list.items()
            .get_at(0)
            .unwrap()
            .downcast_ref::<ember_core::HnItem>()
            .unwrap()
            .id,
        2
    );
    assert_eq!(
        list.items()
            .get_at(1)
            .unwrap()
            .downcast_ref::<ember_core::HnItem>()
            .unwrap()
            .id,
        999
    );
    window.close();
}
