use super::*;

#[test]
fn retrying_an_unavailable_download_revokes_the_check_and_preserves_the_bookmark() {
    for missing in [2002, 1001] {
        let store = Rc::new(MemoryStore::default());
        let offline = Rc::new(Cell::new(false));
        let (shell, window, view) = start(1440.0, 940.0, store.clone(), offline.clone());
        let download = view.get_control::<Button>("DownloadButton");
        click(&shell, &window, &first_story(&window));
        click(&shell, &window, &download);
        assert_eq!(
            AutomationProperties::get_name(&download).as_deref(),
            Some("Manage downloaded discussion")
        );

        ready(store.remove(&format!("cache:item:{missing}"))).unwrap();
        offline.set(true);
        download_action(&shell, &window, &download, "Check offline copy");
        assert_eq!(
            AutomationProperties::get_name(&download).as_deref(),
            Some("Download thread")
        );
        let saved =
            ReaderState::import_json(&ready(store.get("reader:state")).unwrap().unwrap()).unwrap();
        assert!(saved.is_saved(1001));
        assert!(!saved.downloaded_threads.contains_key(&1001));
        window.close();
    }
}

fn download_action(shell: &Shell, window: &Window, button: &Button, label: &str) {
    use ferroui_base::interactivity::RoutedEventArgs;
    use ferroui_controls::{MenuFlyout, MenuItem};
    click_in_view(shell, window, button);
    let menu = button.flyout().unwrap().cast::<MenuFlyout>().unwrap();
    assert!(menu.is_open());
    let items = menu.items();
    let item = (0..items.count())
        .filter_map(|i| Control::from_boxed(&items.get_at(i)?))
        .filter_map(|c| c.cast::<MenuItem>())
        .find(|i| {
            i.header()
                .and_then(|h| h.downcast_ref::<String>().cloned())
                .as_deref()
                == Some(label)
        })
        .unwrap();
    item.raise_event(&RoutedEventArgs::with_event(MenuItem::click_event()));
    menu.hide();
    for _ in 0..5 {
        shell.settle();
    }
}

#[test]
fn unbookmarked_downloads_remain_readable_and_can_be_released_individually() {
    let store = Rc::new(MemoryStore::default());
    let http = Rc::new(CountingHttp {
        fixture: FixtureHttp {
            offline: Rc::new(Cell::new(false)),
        },
        requests: Cell::new(0),
    });
    let (shell, window, view) = start_with_http(390.0, 800.0, store.clone(), http.clone());
    let act = |name| click_in_view(&shell, &window, &view.get_control::<Button>(name));
    let persisted =
        || ReaderState::import_json(&ready(store.get("reader:state")).unwrap().unwrap()).unwrap();
    click_in_view(&shell, &window, &first_story(&window));
    act("DownloadButton");
    act("SaveButton");
    assert!(!persisted().is_saved(1001));
    assert!(persisted().downloaded_threads.contains_key(&1001));
    act("MobileSaved");
    act("DownloadedOnly");
    assert_eq!(title(&view), "Downloads");
    assert_eq!(view.get_control::<ListBox>("Stories").items().count(), 1);
    http.fixture.offline.set(true);
    http.requests.set(0);
    click_in_view(&shell, &window, &first_story(&window));
    let download = view.get_control::<Button>("DownloadButton");
    download_action(&shell, &window, &download, "Check offline copy");
    assert!(
        !persisted().is_saved(1001),
        "checking a verified copy cannot re-bookmark it"
    );
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("Small tools, carefully made")
    );
    download_action(&shell, &window, &download, "Stop keeping offline");
    assert!(!persisted().downloaded_threads.contains_key(&1001));
    assert_eq!(
        AutomationProperties::get_name(&download).as_deref(),
        Some("Download thread")
    );
    assert_eq!(view.get_control::<ListBox>("Stories").items().count(), 0);
    assert_eq!(http.requests.get(), 0);
    window.close();
}

#[test]
fn measured_storage_updates_after_reviewed_cleanup_and_keeps_the_bookmarked_download() {
    use ferroui_controls::Expander;
    let store = Rc::new(MemoryStore::default());
    let (shell, window, view) = start(1280.0, 940.0, store.clone(), Rc::new(Cell::new(false)));
    click_in_view(&shell, &window, &first_story(&window));
    click(
        &shell,
        &window,
        &view.get_control::<Button>("DownloadButton"),
    );
    click(&shell, &window, &view.get_control::<Button>("ThemeButton"));
    let maintenance = view
        .get_visual_descendants()
        .filter_map(|v| v.cast::<Expander>())
        .find(|e| AutomationProperties::get_name(e).as_deref() == Some("Clear local data"))
        .unwrap();
    maintenance.set_is_expanded(true);
    shell.settle();
    let tap = |name| {
        let button = view
            .get_visual_descendants()
            .filter_map(|v| v.cast::<Button>())
            .find(|b| {
                b.is_effectively_visible()
                    && AutomationProperties::get_name(b).as_deref() == Some(name)
            })
            .unwrap_or_else(|| panic!("visible option {name}"));
        click_in_view(&shell, &window, &button);
    };
    tap("Check storage use");
    let report = view
        .get_visual_descendants()
        .filter_map(|v| v.cast::<TextBlock>())
        .find(|t| t.text().unwrap_or_default().contains("of cached data"))
        .unwrap();
    assert!(report.text().unwrap().ends_with("1 download"));
    tap("Clean temporary cache");
    tap("Cancel Clean temporary cache");
    assert!(ready(store.get("cache:item:1002")).unwrap().is_some());
    tap("Clean temporary cache");
    tap("Clean temporary data");
    assert!(ready(store.get("cache:item:1002")).unwrap().is_none());
    assert!(ready(store.get("cache:item:2004")).unwrap().is_some());
    assert!(report.text().unwrap().ends_with("5 items · 1 download"));
    if let Some(dir) = std::env::var_os("EMBER_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        shell
            .last_frame(0)
            .save_ppm(&std::path::Path::new(&dir).join("desktop-offline-storage.ppm"))
            .unwrap();
    }
    window.close();
}

#[test]
fn settings_back_dismisses_transient_data_before_leaving_and_clears_visible_history() {
    use ferroui_base::interactivity::RoutedEventArgs;
    use ferroui_controls::{Expander, TopLevel};

    let store = Rc::new(MemoryStore::default());
    let mut prior = ReaderState::default();
    prior.mark_read(1002, 1);
    prior.items.entry(1003).or_default().hidden = true;
    ready(store.put("reader:state", &prior.export_json().unwrap())).unwrap();
    let (shell, window, view) = start(1280.0, 900.0, store.clone(), Rc::new(Cell::new(false)));
    click_in_view(&shell, &window, &first_story(&window));
    click_in_view(&shell, &window, &view.get_control::<Button>("SaveButton"));
    click_in_view(&shell, &window, &view.get_control::<Button>("NavHistory"));
    let rows = view.get_control::<ListBox>("Stories");
    assert_eq!(rows.items().count(), 2);
    click_in_view(&shell, &window, &view.get_control::<Button>("ThemeButton"));
    let named = |name: &str| {
        view.get_visual_descendants()
            .filter_map(|v| v.cast::<Control>())
            .find(|c| AutomationProperties::get_name(c).as_deref() == Some(name))
            .unwrap()
    };
    let tap = |name: &str| click_in_view(&shell, &window, &named(name));
    let back = view.get_control::<Button>("BackButton");
    tap("Copy backup");
    let generated = named("Generated backup JSON").cast::<TextBox>().unwrap();
    assert!(generated.is_focused());
    let maintenance = named("Clear local data").cast::<Expander>().unwrap();
    maintenance.set_is_expanded(true);
    shell.settle();
    tap("Clear reading history");
    maintenance.set_is_expanded(false);
    shell.settle();
    maintenance.set_is_expanded(true);
    shell.settle();
    assert!(!named("Remove reading history").is_effectively_visible());
    assert!(named("Clear reading history").is_enabled());

    tap("Clear reading history");
    back.focus();
    let request = RoutedEventArgs::with_event(TopLevel::back_requested_event());
    window.raise_event(&request);
    shell.settle();
    assert!(request.handled());
    assert!(named("Clear reading history").is_focused());
    assert!(!named("Remove reading history").is_effectively_visible());
    assert!(generated.is_effectively_visible());
    click_in_view(&shell, &window, &back);
    assert!(!generated.is_effectively_visible());
    assert_eq!(generated.text().as_deref(), Some(""));
    assert!(named("Copy backup").is_focused());
    assert_eq!(rows.items().count(), 2);

    tap("Clear reading history");
    tap("Remove reading history");
    assert_eq!(rows.items().count(), 0);
    let saved =
        ReaderState::import_json(&ready(store.get("reader:state")).unwrap().unwrap()).unwrap();
    assert!(saved.history_ids().is_empty());
    assert!(saved.is_saved(1001));
    assert!(
        !saved.items.contains_key(&1002),
        "Backups retained an ID whose only state was its visit"
    );
    assert!(saved.items[&1003].hidden);
    window.close();
}

#[test]
fn a_feed_download_can_be_canceled_after_navigation_and_retried_without_marking_read() {
    use ferroui_base::input::ContextRequestedEventArgs;
    use ferroui_base::interactivity::RoutedEventArgs;
    use ferroui_controls::{Border, MenuItem};

    struct DelayedReplies {
        fixture: FixtureHttp,
        hold: Cell<bool>,
        waiting: Cell<bool>,
        waker: RefCell<Option<Waker>>,
    }
    impl HttpClient for DelayedReplies {
        fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, ReaderError>> {
            if self.hold.get() && url.ends_with("/item/2001.json") {
                self.waiting.set(true);
                return Box::pin(async move {
                    poll_fn(|cx| {
                        if !self.hold.get() {
                            return Poll::Ready(());
                        }
                        self.waker.replace(Some(cx.waker().clone()));
                        Poll::Pending
                    })
                    .await;
                    self.fixture.get(url).await
                });
            }
            self.fixture.get(url)
        }
    }
    let http = Rc::new(DelayedReplies {
        fixture: FixtureHttp {
            offline: Rc::new(Cell::new(false)),
        },
        hold: Cell::new(true),
        waiting: Cell::new(false),
        waker: RefCell::default(),
    });
    let store = Rc::new(MemoryStore::default());
    let (shell, window, view) = start_with_http(390.0, 844.0, store.clone(), http.clone());
    let command = |label: &str| {
        let card = first_story(&window);
        card.raise_event(&ContextRequestedEventArgs::new());
        let menu = card.context_menu().unwrap();
        let items = menu.items();
        let action = (0..items.count())
            .filter_map(|i| Control::from_boxed(&items.get_at(i)?))
            .filter_map(|c| c.cast::<MenuItem>())
            .find(|item| {
                item.header()
                    .is_some_and(|h| h.downcast_ref::<String>().map(String::as_str) == Some(label))
            })
            .unwrap();
        menu.close();
        action.raise_event(&RoutedEventArgs::with_event(MenuItem::click_event()));
        shell.settle();
    };
    command("Download for offline reading");
    assert!(http.waiting.get());
    assert!(view
        .get_control::<TextBlock>("TransferCount")
        .text()
        .unwrap()
        .contains("1 item loaded"));
    click_in_view(&shell, &window, &view.get_control::<Button>("MobileSaved"));
    let transfer = view.get_control::<Border>("TransferBar");
    assert!(transfer.is_effectively_visible());
    click_in_view(
        &shell,
        &window,
        &view.get_control::<Button>("CancelTransfer"),
    );
    assert!(!transfer.is_effectively_visible());
    assert_eq!(
        view.get_control::<TextBlock>("StatusText")
            .text()
            .as_deref(),
        Some("Download canceled.")
    );
    let state = || {
        ready(store.get("reader:state"))
            .unwrap()
            .map(|json| ReaderState::import_json(&json).unwrap())
            .unwrap_or_default()
    };
    assert!(!state().is_saved(1001) && !state().downloaded_threads.contains_key(&1001));
    click_in_view(&shell, &window, &view.get_control::<Button>("MobileHome"));
    command("Download for offline reading");
    command("Save for later");
    command("Remove from saved");
    http.hold.set(false);
    http.waker.borrow_mut().take().unwrap().wake();
    for _ in 0..5 {
        shell.settle();
    }
    let saved = state();
    assert!(
        !saved.is_saved(1001),
        "completion must honor the latest bookmark edit"
    );
    assert!(saved.downloaded_threads.contains_key(&1001));
    assert!(!saved.is_read(1001));
    assert!(!view
        .get_control::<Border>("DetailPane")
        .is_effectively_visible());
    window.close();
}
