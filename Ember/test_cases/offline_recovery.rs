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
