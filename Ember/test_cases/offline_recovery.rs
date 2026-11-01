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
            Some("Available offline. Download again")
        );

        ready(store.remove(&format!("cache:item:{missing}"))).unwrap();
        offline.set(true);
        click(&shell, &window, &download);
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
