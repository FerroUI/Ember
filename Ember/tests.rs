//! End-to-end retained-control tests: actual pointer input, compositor frames,
//! responsive layout, durable state and offline recovery against a deterministic API.
use crate::{App, ReaderView, SAMPLE};
use ember_core::{BoxFuture, HttpClient, MemoryStore, ReaderError, ReaderState, Repository, Store};
use ferroui_base::Ref;
use ferroui_controls::automation::AutomationProperties;
use ferroui_controls::{Button, Control, ListBox, TextBlock, TextBox, Window};
use sample_testing::Shell;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::future::poll_fn;
use std::rc::Rc;
use std::task::{Poll, Waker};

/// Capture the startup snapshot before yielding, just as a storage transaction can finish
/// before its completion is delivered to the UI. Later writes still complete normally.
struct DelayedStartupStore {
    inner: Rc<MemoryStore>,
    started: Cell<bool>,
    released: Cell<bool>,
    waker: RefCell<Option<Waker>>,
    fail_read: bool,
}
impl DelayedStartupStore {
    fn release(&self) {
        self.released.set(true);
        if let Some(waker) = self.waker.borrow_mut().take() {
            waker.wake();
        }
    }
}
impl Store for DelayedStartupStore {
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>, ReaderError>> {
        Box::pin(async move {
            let snapshot = self.inner.get(key).await?;
            if key == "reader:state" && !self.started.replace(true) {
                poll_fn(|context| {
                    if self.released.get() {
                        Poll::Ready(())
                    } else {
                        *self.waker.borrow_mut() = Some(context.waker().clone());
                        Poll::Pending
                    }
                })
                .await;
                if self.fail_read {
                    return Err(ReaderError::Storage("delayed startup read failed".into()));
                }
            }
            Ok(snapshot)
        })
    }
    fn put<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFuture<'a, Result<(), ReaderError>> {
        self.inner.put(key, value)
    }
    fn remove<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ReaderError>> {
        self.inner.remove(key)
    }
    fn scan_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, String)>, ReaderError>> {
        self.inner.scan_prefix(prefix)
    }
}

struct FixtureHttp {
    offline: Rc<Cell<bool>>,
}
impl HttpClient for FixtureHttp {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, ReaderError>> {
        Box::pin(async move {
            if self.offline.get() {
                return Err(ReaderError::Network("fixture is offline".into()));
            }
            if url.contains("stories.json") {
                return Ok(json!((1001..=1018).collect::<Vec<_>>()).to_string());
            }
            if url.contains("/user/") {
                return Ok(json!({"id":"ada","karma":8240,"created":1451606400u64,"about":"<p>Systems, typography, and tools for thought.</p>","submitted":[1001,2001]}).to_string());
            }
            if url.contains("/api/v1/search") {
                return Ok(json!({"hits":[{"objectID":"1001","title":"Small tools, carefully made","author":"ada","points":326,"num_comments":4,"url":"https://example.com/tools","created_at_i":1792600000u64}],"page":0,"nbPages":1,"nbHits":1}).to_string());
            }
            let id = url
                .rsplit('/')
                .next()
                .unwrap_or("")
                .trim_end_matches(".json")
                .parse::<u64>()
                .unwrap_or(0);
            let value = match id {
                1001 => {
                    json!({"id":1001,"type":"story","title":"Small tools, carefully made","by":"ada","score":326,"time":1792600000u64,"descendants":4,"kids":[2001,2003],"url":"https://example.com/tools","text":"<p>There is something satisfying about software that does one thing <b>beautifully</b>.</p><p>What are the tools you keep coming back to?</p>"})
                }
                1002 => {
                    json!({"id":1002,"type":"story","title":"Ask HN: What changed the way you think about programming?","by":"turing","score":218,"time":1792596400u64,"descendants":92,"kids":[]})
                }
                1003 => {
                    json!({"id":1003,"type":"story","title":"A visual introduction to GPU text rendering","by":"glyph","score":187,"time":1792592800u64,"descendants":48,"url":"https://example.com/graphics","kids":[]})
                }
                1004 => {
                    json!({"id":1004,"type":"story","title":"Show HN: I made a tiny database for the browser","by":"lin","score":142,"time":1792589200u64,"descendants":36,"url":"https://example.com/database","kids":[]})
                }
                1005 => {
                    json!({"id":1005,"type":"story","title":"The quiet craft of building good interfaces","by":"signal","score":109,"time":1792585600u64,"descendants":24,"url":"https://example.com/design","kids":[]})
                }
                1006 => {
                    json!({"id":1006,"type":"job","title":"A small team is hiring systems engineers","by":"team","time":1792582000u64,"url":"https://example.com/jobs"})
                }
                1007..=1018 => {
                    json!({"id":id,"type":"story","title":format!("Notebook entry {}",id-1000),"by":"editor","kids":[]})
                }
                2001 => {
                    json!({"id":2001,"type":"comment","parent":1001,"by":"lin","time":1792601000u64,"kids":[2002],"text":"<p>The best tools disappear into the work. You stop thinking about the interface and start thinking about the idea.</p><blockquote>A good tool changes how you see the problem.</blockquote>"})
                }
                2002 => {
                    json!({"id":2002,"type":"comment","parent":2001,"by":"ada","time":1792602000u64,"kids":[],"text":"<p>Exactly. A small, predictable surface can still have a very deep model underneath.</p><pre><code>let idea = curiosity\n    .filter(noise)\n    .collect();</code></pre>"})
                }
                2003 => {
                    json!({"id":2003,"type":"comment","parent":1001,"by":"mira","time":1792601500u64,"kids":[2004],"text":"<p>Offline support is underrated. The feeling that your work is <i>already there</i> matters more than another feature.</p><p>I wrote about it <a href=\"https://example.com/offline\">here</a>.</p>"})
                }
                2004 => {
                    json!({"id":2004,"type":"comment","parent":2003,"by":"noor","time":1792602200u64,"kids":[],"text":"<p>And good keyboard navigation. Every action should feel close at hand.</p>"})
                }
                _ => serde_json::Value::Null,
            };
            Ok(value.to_string())
        })
    }
}

fn start(
    width: f64,
    height: f64,
    store: Rc<dyn Store>,
    offline: Rc<Cell<bool>>,
) -> (Shell, Ref<Window>, Ref<ReaderView>) {
    crate::set_repository_factory(Rc::new(move || {
        Rc::new(Repository::new(
            Rc::new(FixtureHttp {
                offline: offline.clone(),
            }),
            store.clone(),
        ))
    }));
    let shell = Shell::start(&SAMPLE, width, height, || App::new().upcast());
    let view = ReaderView::new();
    let window = Window::new();
    window.set_width(width);
    window.set_height(height);
    window.set_content(Some(Control::boxed(&view)));
    window.show();
    for _ in 0..8 {
        shell.settle();
    }
    (shell, window, view)
}
fn click(shell: &Shell, window: &Window, control: &Control) {
    let bounds = Shell::bounds_of(window, control);
    assert!(
        bounds.width > 0.0 && bounds.height > 0.0,
        "control must be laid out before pointer input"
    );
    shell.click(0, bounds.center());
    for _ in 0..5 {
        shell.settle();
    }
}
/// Clicks `control` after it has been scrolled into view and laid out, and after checking
/// that the pointer at its centre reaches it: a click that misses fails here, not later as
/// a lost change.
fn click_in_view(shell: &Shell, window: &Window, control: &Control) {
    control.bring_into_view();
    for _ in 0..4 {
        shell.settle();
    }
    let reaches = |point| {
        window
            .input_hit_test(point)
            .map(|hit| hit.upcast::<ferroui_base::Visual>())
            .is_some_and(|hit| {
                control.is_visual_ancestor_of(&hit)
                    || hit
                        .cast::<Control>()
                        .is_some_and(|hit| std::ptr::eq(&*hit, control))
            })
    };
    let mut center = Shell::bounds_of(window, control).center();
    if !reaches(center) {
        // Centre the control in its scroll viewer when bringing it into view left it
        // under the edge of the viewport.
        if let Some(scroll) = control
            .get_visual_ancestors()
            .find_map(|ancestor| ancestor.cast::<ferroui_controls::ScrollViewer>())
        {
            let in_viewer = Shell::bounds_of(window, control);
            let viewer = Shell::bounds_of(window, &scroll);
            let offset = scroll.offset();
            let target =
                offset.y + (in_viewer.y - viewer.y) - (viewer.height - in_viewer.height) / 2.0;
            scroll.set_offset(ferroui_base::Vector::new(offset.x, target.max(0.0)));
            for _ in 0..4 {
                shell.settle();
            }
            center = Shell::bounds_of(window, control).center();
        }
    }
    assert!(
        reaches(center),
        "the pointer at the centre of the control must reach it before the click"
    );
    shell.click(0, center);
    for _ in 0..5 {
        shell.settle();
    }
}
fn first_story(window: &Window) -> Ref<crate::controls::StoryCard> {
    window
        .get_visual_descendants()
        .find_map(|v| v.cast::<crate::controls::StoryCard>())
        .expect("a realized story card")
}
fn title(view: &ReaderView) -> String {
    view.get_control::<TextBlock>("FeedTitle")
        .text()
        .unwrap_or_default()
}

fn ready<T>(mut future: BoxFuture<'_, T>) -> T {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(value) => value,
        std::task::Poll::Pending => panic!("MemoryStore operations should complete immediately"),
    }
}

#[test]
fn backup_import_survives_a_delayed_startup_snapshot_or_error() {
    for fail_read in [false, true] {
        let inner = Rc::new(MemoryStore::default());
        let mut original = ReaderState::default();
        original.toggle_saved(1002, 1);
        ready(inner.put("reader:state", &original.export_json().unwrap())).unwrap();
        let store = Rc::new(DelayedStartupStore {
            inner: inner.clone(),
            started: Cell::new(false),
            released: Cell::new(false),
            waker: RefCell::new(None),
            fail_read,
        });
        let (shell, window, view) = start(1440.0, 940.0, store.clone(), Rc::new(Cell::new(false)));
        assert!(
            store.started.get(),
            "startup storage read must be suspended"
        );
        assert!(!store.released.get());

        click(&shell, &window, &view.get_control::<Button>("ThemeButton"));
        let input = view
            .get_visual_descendants()
            .filter_map(|node| node.cast::<TextBox>())
            .find(|input| {
                AutomationProperties::get_name(input).as_deref() == Some("Reading data JSON")
            })
            .expect("real Settings backup input");
        let mut imported = ReaderState::default();
        imported.toggle_saved(1001, 2);
        input.set_text(Some(&imported.export_json().unwrap()));
        for _ in 0..4 {
            shell.settle();
        }
        let import = view
            .get_visual_descendants()
            .filter_map(|node| node.cast::<Button>())
            .find(|button| {
                AutomationProperties::get_name(button).as_deref() == Some("Import this backup")
            })
            .expect("real Settings import action");
        click_in_view(&shell, &window, &import);
        // The import was accepted before the store is read: a missed click and a lost write
        // would otherwise look the same.
        assert_eq!(
            view.get_control::<TextBlock>("StatusText")
                .text()
                .as_deref(),
            Some("Reading library imported.")
        );
        let persisted = ready(inner.get("reader:state")).unwrap().unwrap();
        assert_eq!(
            ReaderState::import_json(&persisted).unwrap().saved_ids(),
            vec![1001]
        );
        assert!(
            !store.released.get(),
            "import must finish before startup resumes"
        );

        store.release();
        for _ in 0..8 {
            shell.settle();
        }
        assert_eq!(
            view.get_control::<TextBlock>("StatusText")
                .text()
                .as_deref(),
            Some("Reading library imported."),
            "a superseded startup result must not show a stale restore error"
        );
        click(&shell, &window, &view.get_control::<Button>("NavSaved"));
        assert_eq!(view.get_control::<ListBox>("Stories").items().count(), 1);
        assert_eq!(
            first_story(&window).headline().as_deref(),
            Some("Small tools, carefully made")
        );
        // Opening the imported story writes read tracking, proving the imported state remains
        // authoritative for subsequent saves as well as the visible library.
        click(&shell, &window, &first_story(&window));
        let persisted = ready(inner.get("reader:state")).unwrap().unwrap();
        let persisted = ReaderState::import_json(&persisted).unwrap();
        assert_eq!(persisted.saved_ids(), vec![1001]);
        assert!(persisted.is_read(1001));
        window.close();
    }
}

#[test]
fn unreadable_saved_state_is_never_replaced_by_automatic_read_tracking() {
    let store = Rc::new(MemoryStore::default());
    let original = "{a damaged backup that must remain recoverable";
    ready(store.put("reader:state", original)).unwrap();
    let (shell, window, view) = start(1440.0, 940.0, store.clone(), Rc::new(Cell::new(false)));
    click(&shell, &window, &first_story(&window));
    click(&shell, &window, &view.get_control::<Button>("SaveButton"));
    window.close();
    shell.settle();
    assert_eq!(
        ready(store.get("reader:state")).unwrap().as_deref(),
        Some(original)
    );
}

#[test]
fn phone_back_restores_more_and_thread_without_history_cycles() {
    let (shell, window, view) = start(
        390.0,
        844.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    click(&shell, &window, &view.get_control::<Button>("MobileMore"));
    let settings = view
        .get_visual_descendants()
        .filter_map(|node| node.cast::<Button>())
        .find(|button| {
            AutomationProperties::get_name(button).as_deref() == Some("Appearance and settings")
        })
        .unwrap();
    click_in_view(&shell, &window, &settings);
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("Explore Ember")
    );
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert!(view
        .get_control::<ferroui_controls::Border>("FeedPane")
        .is_effectively_visible());
    click(&shell, &window, &first_story(&window));
    click(&shell, &window, &view.get_control::<Button>("MobileMore"));
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert!(view
        .get_control::<Button>("SaveButton")
        .is_effectively_visible());
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert!(view
        .get_control::<ferroui_controls::Border>("FeedPane")
        .is_effectively_visible());
    window.close();
}

#[test]
fn unsaving_removes_the_card_from_the_visible_saved_list() {
    let (shell, window, view) = start(
        1440.0,
        940.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    click(&shell, &window, &first_story(&window));
    click(&shell, &window, &view.get_control::<Button>("SaveButton"));
    click(&shell, &window, &view.get_control::<Button>("NavSaved"));
    click(&shell, &window, &first_story(&window));
    click(&shell, &window, &view.get_control::<Button>("SaveButton"));
    assert_eq!(
        view.get_control::<ferroui_controls::ListBox>("Stories")
            .items()
            .count(),
        0
    );
    assert!(view
        .get_control::<ferroui_controls::StackPanel>("FeedEmpty")
        .is_effectively_visible());
    window.close();
}

#[test]
fn opening_a_story_preserves_feed_position_and_platform_back_returns_to_the_feed() {
    use ferroui_base::interactivity::RoutedEventArgs;
    use ferroui_controls::{ListBox, ScrollViewer, TopLevel};
    let (shell, window, view) = start(
        390.0,
        844.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    let stories = view.get_control::<ListBox>("Stories");
    stories.scroll_into_view(stories.items().count() as i32 - 1);
    shell.settle();
    let scroll = stories
        .scroll()
        .and_then(|v| v.cast::<ScrollViewer>())
        .unwrap();
    let before = scroll.offset();
    assert!(before.y > 0.0, "test must begin away from the feed's top");
    let card = window
        .get_visual_descendants()
        .filter_map(|v| v.cast::<crate::controls::StoryCard>())
        .find(|card| {
            let bounds = Shell::bounds_of(&window, card);
            bounds.y > 250.0 && bounds.bottom() < 740.0
        })
        .expect("visible card in scrolled feed");
    click(&shell, &window, &card);
    let event = RoutedEventArgs::with_event(TopLevel::back_requested_event());
    window.raise_event(&event);
    shell.settle();
    assert!(
        event.handled(),
        "platform Back is consumed inside a discussion"
    );
    assert!(view
        .get_control::<ferroui_controls::Border>("FeedPane")
        .is_effectively_visible());
    assert!(
        (scroll.offset().y - before.y).abs() < 1.0,
        "read state must not reset the feed scroll position"
    );
    let event = RoutedEventArgs::with_event(TopLevel::back_requested_event());
    window.raise_event(&event);
    assert!(
        !event.handled(),
        "platform Back remains available to the OS on the home screen"
    );
    window.close();
}

#[test]
fn desktop_pointer_navigation_save_and_library_are_connected() {
    let (shell, window, view) = start(
        1440.0,
        940.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    assert_eq!(title(&view), "Front page");
    click(&shell, &window, &first_story(&window));
    assert!(view
        .get_control::<Button>("SaveButton")
        .is_effectively_visible());
    assert!(view
        .get_control::<TextBlock>("DetailTitle")
        .text()
        .unwrap_or_default()
        .contains("comments"));
    click(&shell, &window, &view.get_control::<Button>("SaveButton"));
    click(&shell, &window, &view.get_control::<Button>("NavSaved"));
    assert_eq!(title(&view), "Saved for later");
    assert_eq!(
        first_story(&window).status_label().as_deref(),
        Some("SAVED")
    );
    assert!(
        shell.take_binding_reports().is_empty(),
        "compiled templates must have no binding failures"
    );
    window.close();
}

#[test]
fn phone_back_navigation_and_offline_reopen_keep_saved_content() {
    let store = Rc::new(MemoryStore::default());
    let offline = Rc::new(Cell::new(false));
    {
        let (shell, window, view) = start(390.0, 844.0, store.clone(), offline.clone());
        assert!(view
            .get_control::<ferroui_controls::Border>("BottomNavigation")
            .is_effectively_visible());
        assert!(!view
            .get_control::<ferroui_controls::Border>("NavigationRail")
            .is_effectively_visible());
        click(&shell, &window, &first_story(&window));
        assert!(!view
            .get_control::<ferroui_controls::Border>("FeedPane")
            .is_effectively_visible());
        click(&shell, &window, &view.get_control::<Button>("SaveButton"));
        click(
            &shell,
            &window,
            &view.get_control::<Button>("DownloadButton"),
        );
        click(&shell, &window, &view.get_control::<Button>("BackButton"));
        assert!(view
            .get_control::<ferroui_controls::Border>("FeedPane")
            .is_effectively_visible());
        window.close();
    }
    offline.set(true);
    let (shell, window, view) = start(390.0, 844.0, store, offline);
    click(&shell, &window, &view.get_control::<Button>("MobileSaved"));
    assert_eq!(title(&view), "Saved for later");
    click(&shell, &window, &first_story(&window));
    assert!(view
        .get_control::<TextBlock>("DetailTitle")
        .text()
        .unwrap_or_default()
        .contains("4 comments"));
    window.close();
}

#[test]
fn tablet_search_and_phone_find_remain_reachable_across_resize() {
    let (shell, window, view) = start(
        800.0,
        680.0,
        Rc::new(MemoryStore::default()),
        Rc::new(Cell::new(false)),
    );
    let search = view.get_control::<Button>("CompactSearch");
    assert!(search.is_effectively_visible());
    click(&shell, &window, &search);
    assert_eq!(
        view.get_control::<TextBlock>("DetailTitle")
            .text()
            .as_deref(),
        Some("Search")
    );
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    click(&shell, &window, &first_story(&window));
    window.set_width(360.0);
    shell.settle();
    assert!(!view
        .get_control::<ferroui_controls::Border>("Masthead")
        .is_effectively_visible());
    let find = view.get_control::<TextBox>("ThreadFind");
    assert!(!find.is_effectively_visible());
    click(&shell, &window, &view.get_control::<Button>("FindButton"));
    assert!(find.is_effectively_visible());
    find.set_text(Some("tools"));
    click(&shell, &window, &view.get_control::<Button>("CloseFind"));
    assert!(!find.is_effectively_visible());
    assert!(find.text().unwrap_or_default().is_empty());
    click(&shell, &window, &view.get_control::<Button>("BackButton"));
    assert!(view
        .get_control::<ferroui_controls::Border>("FeedPane")
        .is_effectively_visible());
    window.close();
}

#[test]
fn capture_real_control_tree_in_desktop_and_phone_themes() {
    for (width, height, dark, name) in [
        (1440.0, 940.0, false, "desktop-light"),
        (1440.0, 940.0, true, "desktop-dark"),
        (390.0, 844.0, false, "phone-light"),
        (390.0, 844.0, true, "phone-dark"),
        (1024.0, 820.0, false, "tablet-custom"),
    ] {
        let (shell, window, view) = start(
            width,
            height,
            Rc::new(MemoryStore::default()),
            Rc::new(Cell::new(false)),
        );
        if dark {
            ferroui_controls::Application::current()
                .unwrap()
                .set_requested_theme_variant(Some(ferroui_base::styling::ThemeVariant::dark()));
        }
        if name == "tablet-custom" {
            crate::theme::apply(&ember_core::Preferences {
                theme: ember_core::ThemeMode::Custom,
                accent: "#2DD4BF".to_owned(),
                custom_background: Some("#092F2A".to_owned()),
                ..Default::default()
            });
        }
        if width > 900.0 {
            click(&shell, &window, &first_story(&window));
        }
        shell.settle();
        assert!(
            shell.last_frame(0).colors((0.0, 0.0, width, height)) > 30,
            "the reader must render its text, cards, icons and controls"
        );
        if let Some(path) = std::env::var_os("EMBER_CAPTURE_DIR") {
            let path = std::path::PathBuf::from(path);
            std::fs::create_dir_all(&path).unwrap();
            shell
                .last_frame(0)
                .save_ppm(&path.join(format!("{name}.ppm")))
                .unwrap();
            if width < 900.0 {
                click(&shell, &window, &first_story(&window));
                shell.settle();
                shell
                    .last_frame(0)
                    .save_ppm(&path.join(format!("{name}-thread.ppm")))
                    .unwrap();
            }
        }
        assert!(view.bounds().width <= width + 1.0);
        assert!(
            shell.take_binding_reports().is_empty(),
            "theme and template bindings must resolve"
        );
        window.close();
    }
}
