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
            json!({"id":"ada","karma":8240,"about":"<p>My <a href='/item?id=2002'>original comment</a> and the layout function:</p><pre>fn column(width: f64, gutter: f64, count: f64) -&gt; f64 {\n    (width - gutter * (count - 1.0)) / count\n}</pre>","submitted":[]}),
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
