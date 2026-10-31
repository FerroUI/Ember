use crate::controls::StoryCard;
use crate::markup::xaml_class;
use crate::settings::SettingsAction;
use crate::thread_view::ThreadAction;
use crate::{services, widgets};
use ember_core::{
    DataSource, FeedKind, FetchMode, HnItem, ReaderState, Repository, SearchKind, SearchQuery,
    ThreadSnapshot, ThreadSort,
};
use ferroui_base::input::gesture_recognizers::{PullGestureRecognizer, SwipeGestureRecognizer};
use ferroui_base::input::platform::ClipboardExtensions;
use ferroui_base::input::{InputElement, InputElementImpl, Key, KeyEventArgs, KeyModifiers};
use ferroui_base::interactivity::{InteractiveImpl, RoutedEventArgs, RoutedEventHandlerToken};
use ferroui_base::layout::LayoutableImpl;
use ferroui_base::rendering::composition::{ElementComposition, ICompositionObjectAnimations};
use ferroui_base::threading::{Dispatcher, DispatcherPriority, DispatcherTimer};
use ferroui_base::utilities::Uri;
use ferroui_base::{
    ferro_class, ferro_class_info, ferro_impl_classes, instantiate, BoxedValue, FerroObjectImpl,
    Ref, StyledElementImpl, Thickness, VisualImpl, WeakRef,
};
use ferroui_controls::automation::{AutomationLiveSetting, AutomationProperties};
use ferroui_controls::primitives::TemplatedControlImpl;
use ferroui_controls::templates::FuncDataTemplate;
use ferroui_controls::{
    Border, Button, ColumnDefinitions, ContentControl, ContentControlImpl, Control, ControlImpl,
    ControlImplExt, Grid, ListBox, ScrollViewer, StackPanel, TextBlock, TextBox, ToolTip, TopLevel,
    UserControl,
};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
enum FeedRoute {
    Feed(FeedKind),
    Saved,
    History,
    Hidden,
    Search(SearchQuery),
}
#[derive(Clone, Debug, PartialEq)]
enum DetailRoute {
    Welcome,
    Thread(u64),
    Profile(String),
    Settings,
    Search,
    More,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspaceLayout {
    Phone,
    CompactRail,
    LabeledRail,
    Reading,
}

ferroui_base::ferro_thread_local! {
    static ACTIVE: RefCell<Option<WeakRef<ReaderView>>> = RefCell::new(None);
    static LOCATION_CHANGED: RefCell<Option<Rc<dyn Fn(Option<u64>)>>> = RefCell::new(None);
}
pub fn set_location_changed(callback: Rc<dyn Fn(Option<u64>)>) {
    LOCATION_CHANGED.with(|v| *v.borrow_mut() = Some(callback));
}
pub fn navigate_location(location: &str) {
    ACTIVE.with(|v| {
        if let Some(view) = v.borrow().as_ref().and_then(|v| v.upgrade()) {
            if let Some(id) = item_from_location(location) {
                view.open_item_internal(id, false);
            } else {
                view.back_to_feed(false);
            }
        }
    });
}
fn item_from_location(location: &str) -> Option<u64> {
    location.split(['?', '&', '#']).find_map(|part| {
        part.strip_prefix("item=")
            .or_else(|| part.strip_prefix("id="))
            .or_else(|| part.strip_prefix("item/"))
            .or_else(|| part.strip_prefix("/item/"))
            .and_then(|id| id.parse::<u64>().ok())
            .filter(|id| *id > 0)
    })
}

#[repr(C)]
pub struct ReaderView {
    base: UserControl,
    repository: RefCell<Option<Rc<Repository>>>,
    state: RefCell<ReaderState>,
    state_ready: Cell<bool>,
    started: Cell<bool>,
    revision: Cell<u64>,
    saving: Cell<bool>,
    feed: RefCell<FeedRoute>,
    detail: RefCell<DetailRoute>,
    history: RefCell<Vec<DetailRoute>>,
    stories: RefCell<Vec<HnItem>>,
    rendered_stories: RefCell<Vec<HnItem>>,
    rendered_show_dead: Cell<bool>,
    search_story_ids: RefCell<BTreeMap<u64, u64>>,
    selected: Cell<Option<u64>>,
    // An Algolia comment result and its containing discussion have different identities.
    selected_row: Cell<Option<u64>>,
    pending_comment: Cell<Option<u64>>,
    next: Cell<Option<usize>>,
    feed_generation: Cell<u64>,
    detail_generation: Cell<u64>,
    busy: Cell<bool>,
    thread: RefCell<Option<ThreadSnapshot>>,
    thread_control: RefCell<Option<Ref<Control>>>,
    pruning: Cell<bool>,
    /// Whether the cache was pruned after a feed loaded this session (see `accept_items`).
    pruned_after_feed: Cell<bool>,
    downloading: Cell<bool>,
    thread_budget: Cell<usize>,
    thread_sort: Cell<ThreadSort>,
    find_revision: Cell<u64>,
    focus_reading: Cell<bool>,
    layout_mode: Cell<Option<WorkspaceLayout>>,
    back_handler: RefCell<Option<(WeakRef<TopLevel>, RoutedEventHandlerToken)>>,
}
ferro_class!(ReaderView: UserControl);
ferro_class_info!(ReaderView {
    new: ReaderView::new
});
ferro_impl_classes!(ReaderView: FerroObjectImpl, StyledElementImpl, VisualImpl, LayoutableImpl,
    InteractiveImpl, InputElementImpl, TemplatedControlImpl, ContentControlImpl);
xaml_class!(ReaderView, "/ReaderView.xaml");
impl ControlImpl for ReaderView {
    fn on_loaded(this: &Self, e: &RoutedEventArgs) {
        Self::parent_on_loaded(this, e);
        this.connect_platform_back();
        this.start();
        this.apply_layout();
    }
    fn on_unloaded(this: &Self, e: &RoutedEventArgs) {
        this.remember_thread();
        if let Some((top, token)) = this.back_handler.borrow_mut().take() {
            if let Some(top) = top.upgrade() {
                top.remove_handler(TopLevel::back_requested_event(), token);
            }
        }
        Self::parent_on_unloaded(this, e);
    }
}

impl ReaderView {
    pub fn construct() -> Self {
        Self {
            base: UserControl::construct(),
            repository: RefCell::new(None),
            state: RefCell::new(ReaderState::default()),
            state_ready: Cell::new(false),
            started: Cell::new(false),
            revision: Cell::new(0),
            saving: Cell::new(false),
            feed: RefCell::new(FeedRoute::Feed(FeedKind::Top)),
            detail: RefCell::new(DetailRoute::Welcome),
            history: RefCell::new(Vec::new()),
            stories: RefCell::new(Vec::new()),
            rendered_stories: RefCell::new(Vec::new()),
            rendered_show_dead: Cell::new(false),
            search_story_ids: RefCell::new(BTreeMap::new()),
            selected: Cell::new(None),
            selected_row: Cell::new(None),
            pending_comment: Cell::new(None),
            next: Cell::new(None),
            feed_generation: Cell::new(0),
            detail_generation: Cell::new(0),
            busy: Cell::new(false),
            thread: RefCell::new(None),
            thread_control: RefCell::new(None),
            pruning: Cell::new(false),
            pruned_after_feed: Cell::new(false),
            downloading: Cell::new(false),
            thread_budget: Cell::new(120),
            thread_sort: Cell::new(ThreadSort::Ranked),
            find_revision: Cell::new(0),
            focus_reading: Cell::new(false),
            layout_mode: Cell::new(None),
            back_handler: RefCell::new(None),
        }
    }
    pub fn new() -> Ref<Self> {
        let this = instantiate(Self::construct());
        this.initialize_component();
        this.wire();
        this.configure_detail();
        ACTIVE.with(|v| *v.borrow_mut() = Some(this.downgrade()));
        let weak = this.downgrade();
        this.show_content(widgets::welcome(move || {
            if let Some(v) = weak.upgrade() {
                v.show_search();
            }
        }));
        this
    }
    fn button(&self, name: &str) -> Ref<Button> {
        self.get_control::<Button>(name)
    }
    fn label(&self, name: &str) -> Ref<TextBlock> {
        self.get_control::<TextBlock>(name)
    }
    fn textbox(&self, name: &str) -> Ref<TextBox> {
        self.get_control::<TextBox>(name)
    }
    fn repo(&self) -> Option<Rc<Repository>> {
        self.repository.borrow().clone()
    }
    fn on(&self, name: &str, action: impl Fn(&ReaderView) + 'static) {
        let weak = self.to_ref().downgrade();
        self.button(name).click(move |_, _| {
            if let Some(v) = weak.upgrade() {
                action(&v)
            }
        });
    }
    fn wire(&self) {
        AutomationProperties::set_live_setting(
            &self.label("StatusText"),
            AutomationLiveSetting::Polite,
        );
        for (name, icon) in [
            ("SearchButton", "search"),
            ("CompactSearch", "search"),
            ("ThreadRefresh", "refresh"),
            ("FindButton", "search"),
            ("FocusButton", "focus_reading"),
            ("CloseFind", "dismiss"),
            ("DismissStatus", "dismiss"),
            ("RefreshButton", "refresh"),
            ("ThemeButton", "settings"),
            ("BackButton", "back"),
            ("SaveButton", "bookmark"),
            ("DownloadButton", "download"),
        ] {
            self.button(name)
                .set_content(Some(Control::boxed(&crate::icons::icon(icon))));
        }
        for (name, feed) in [
            ("NavTop", FeedKind::Top),
            ("NavNew", FeedKind::New),
            ("NavBest", FeedKind::Best),
            ("NavAsk", FeedKind::Ask),
            ("NavShow", FeedKind::Show),
            ("NavJobs", FeedKind::Jobs),
            ("MobileHome", FeedKind::Top),
            ("MobileNew", FeedKind::New),
        ] {
            self.on(name, move |v| v.select_feed(FeedRoute::Feed(feed)));
        }
        self.on("NavSaved", |v| v.select_feed(FeedRoute::Saved));
        self.on("MobileSaved", |v| v.select_feed(FeedRoute::Saved));
        self.on("NavHistory", |v| v.select_feed(FeedRoute::History));
        self.on("NavHidden", |v| v.select_feed(FeedRoute::Hidden));
        self.on("NavMore", |v| v.show_more());
        self.on("CompactSearch", |v| v.show_search());
        self.on("ThemeButton", |v| v.show_settings());
        self.on("MobileSearch", |v| v.show_search());
        self.on("MobileMore", |v| v.show_more());
        self.on("SearchButton", |v| v.submit_quick_search());
        self.on("RefreshButton", |v| v.refresh());
        self.on("ThreadRefresh", |v| {
            if let DetailRoute::Thread(id) = *v.detail.borrow() {
                v.fetch_thread(id, FetchMode::Refresh);
            }
        });
        self.on("FindButton", |v| {
            v.set_find_open(!v.get_control::<Border>("ThreadTools").is_visible())
        });
        self.on("FocusButton", |v| {
            v.set_focus_reading(!v.focus_reading.get())
        });
        self.on("CloseFind", |v| v.set_find_open(false));
        self.on("RetryButton", |v| v.refresh());
        self.on("LoadMoreButton", |v| {
            if let Some(offset) = v.next.get() {
                v.load_feed(offset, FetchMode::CacheFirst);
            }
        });
        self.on("BackButton", |v| v.go_back());
        self.on("SaveButton", |v| {
            if let Some(id) = v.selected.get() {
                v.toggle_save(id);
            }
        });
        self.on("DownloadButton", |v| v.download());
        self.on("DismissStatus", |v| {
            v.get_control::<Border>("StatusBar").set_is_visible(false)
        });
        self.on("UnreadButton", |v| {
            let mut state = v.state.borrow_mut();
            state.preferences.hide_read = !state.preferences.hide_read;
            drop(state);
            v.changed();
            v.render_stories();
        });
        let weak = self.to_ref().downgrade();
        self.property_changed(move |e| {
            if e.property() == ferroui_base::Visual::bounds_property().as_property() {
                if let Some(v) = weak.upgrade() {
                    v.apply_layout();
                }
            }
        });
        let weak = self.to_ref().downgrade();
        self.textbox("SearchBox").add_handler(
            InputElement::key_down_event(),
            move |_, e: &KeyEventArgs| {
                if e.key == Key::Enter {
                    if let Some(v) = weak
                        .upgrade()
                        .filter(|v| !v.textbox("SearchBox").is_composing_text())
                    {
                        v.submit_quick_search();
                        e.set_handled(true);
                    }
                }
            },
        );
        let weak = self.to_ref().downgrade();
        self.textbox("ThreadFind").property_changed(move |e| {
            if e.property() == TextBox::text_property().as_property() {
                if let Some(v) = weak.upgrade() {
                    let revision = v.find_revision.get().wrapping_add(1);
                    v.find_revision.set(revision);
                    let weak = v.to_ref().downgrade();
                    DispatcherTimer::run_once(
                        move || {
                            if let Some(view) = weak
                                .upgrade()
                                .filter(|view| view.find_revision.get() == revision)
                            {
                                view.render_thread();
                            }
                        },
                        Duration::from_millis(180),
                        DispatcherPriority::BACKGROUND,
                    );
                }
            }
        });
        let weak = self.to_ref().downgrade();
        self.add_handler(
            InputElement::key_down_event(),
            move |_, e: &KeyEventArgs| {
                let Some(v) = weak.upgrade() else {
                    return;
                };
                let ctrl = e
                    .key_modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::META);
                if ctrl && e.key == Key::K {
                    v.show_search();
                    e.set_handled(true);
                    return;
                }
                if ctrl && e.key == Key::F && matches!(*v.detail.borrow(), DetailRoute::Thread(_)) {
                    v.set_find_open(true);
                    e.set_handled(true);
                    return;
                }
                if e.key == Key::Escape {
                    if v.get_control::<Border>("ThreadTools").is_visible() {
                        v.set_find_open(false);
                    } else if *v.detail.borrow() != DetailRoute::Welcome {
                        v.go_back();
                    } else {
                        return;
                    }
                    e.set_handled(true);
                    return;
                }
                if e.key_modifiers != KeyModifiers::NONE
                    || !matches!(
                        *v.detail.borrow(),
                        DetailRoute::Welcome | DetailRoute::Thread(_)
                    )
                    || e.source()
                        .and_then(|s| s.cast::<ferroui_base::Visual>())
                        .is_some_and(|source| {
                            std::iter::once(source.clone())
                                .chain(source.get_visual_ancestors())
                                .any(|v| {
                                    v.cast::<TextBox>().is_some()
                                        || v.cast::<ferroui_controls::ComboBox>().is_some()
                                })
                        })
                {
                    return;
                }
                match e.key {
                    Key::F if matches!(*v.detail.borrow(), DetailRoute::Thread(_)) => {
                        v.set_focus_reading(!v.focus_reading.get())
                    }
                    Key::R => v.refresh(),
                    Key::S => {
                        if let DetailRoute::Thread(id) = *v.detail.borrow() {
                            v.toggle_save(id);
                        }
                    }
                    Key::J => v.move_selection(1),
                    Key::K => v.move_selection(-1),
                    Key::O => {
                        if let Some(item) = v
                            .current_item()
                            .filter(|_| matches!(*v.detail.borrow(), DetailRoute::Thread(_)))
                        {
                            v.open_url(
                                &item.article_url().unwrap_or_else(|| item.discussion_url()),
                            );
                        }
                    }
                    _ => return,
                };
                e.set_handled(true);
            },
        );
        let weak = self.to_ref().downgrade();
        let template = FuncDataTemplate::for_type::<HnItem>(
            move |item, _| {
                weak.upgrade()
                    .map(|v| v.story_card(item).upcast::<Control>())
            },
            false,
        );
        self.get_control::<ListBox>("Stories")
            .set_item_template(Some(template));
        let header = self.get_control::<StackPanel>("FeedHeader");
        header
            .gesture_recognizers()
            .add(PullGestureRecognizer::new());
        let pull_distance = Rc::new(Cell::new(0.0));
        let previous_summary = Rc::new(RefCell::new(None::<String>));
        let weak = self.to_ref().downgrade();
        let distance = pull_distance.clone();
        let summary = previous_summary.clone();
        header.add_handler(InputElement::pull_gesture_event(), move |_, e| {
            if let Some(view) = weak.upgrade() {
                if view.busy.get() {
                    return;
                }
                if summary.borrow().is_none() {
                    *summary.borrow_mut() = view.label("FeedSummary").text();
                }
                distance.set(e.delta().y);
                view.label("FeedSummary")
                    .set_text(Some(if e.delta().y >= 72.0 {
                        "Release to refresh"
                    } else {
                        "Pull to refresh"
                    }));
                e.set_handled(true);
            }
        });
        let weak = self.to_ref().downgrade();
        header.add_handler(InputElement::pull_gesture_ended_event(), move |_, _| {
            if let Some(view) = weak.upgrade() {
                if pull_distance.replace(0.0) >= 72.0 && !view.busy.get() {
                    view.refresh();
                } else if let Some(summary) = previous_summary.borrow().as_deref() {
                    view.label("FeedSummary").set_text(Some(summary));
                }
                previous_summary.borrow_mut().take();
            }
        });
        let weak = self.to_ref().downgrade();
        self.actual_theme_variant_changed(move || {
            let weak = weak.clone();
            Dispatcher::ui_thread().post_local(
                move || {
                    if let Some(view) = weak.upgrade() {
                        view.render_thread();
                    }
                },
                DispatcherPriority::LOADED,
            );
        });
    }
    fn connect_platform_back(&self) {
        if self.back_handler.borrow().is_some() {
            return;
        }
        if let Some(top) = TopLevel::get_top_level(Some(self)) {
            let weak = self.to_ref().downgrade();
            let token = top.back_requested(move |_, event| {
                if let Some(view) = weak.upgrade() {
                    if *view.detail.borrow() != DetailRoute::Welcome {
                        view.go_back();
                        event.set_handled(true);
                    }
                }
            });
            *self.back_handler.borrow_mut() = Some((top.downgrade(), token));
        }
    }
    fn start(&self) {
        if self.started.replace(true) {
            return;
        }
        let repo = match services::repository() {
            Ok(repo) => repo,
            Err(error) => {
                self.status(&format!("Cannot open the reading database: {error}"));
                self.empty(
                    "Storage could not be opened",
                    "Close other instances or check app storage, then restart Ember.",
                );
                return;
            }
        };
        *self.repository.borrow_mut() = Some(repo.clone());
        let weak = self.to_ref().downgrade();
        drop(Dispatcher::ui_thread().invoke_async_task_local(move || async move {
            let result = repo.load_state().await;
            if let Some(view) = weak.upgrade() {
                // A backup import can complete while storage is still opening.
                // Its newer state must win over the outstanding startup snapshot.
                if !view.state_ready.get() {
                    match result {
                        Ok(state) => {
                            crate::theme::apply(&state.preferences);
                            *view.state.borrow_mut() = state;
                            view.state_ready.set(true);
                        }
                        Err(error) => view.status(&format!(
                            "Reading state could not be restored: {error}. Saving is paused to protect your existing data. Import a backup in Settings to recover it."
                        )),
                    }
                }
                view.load_feed(0, FetchMode::CacheFirst);
                if let Some(location) = services::take_initial_location() {
                    if let Some(id) = item_from_location(&location) {
                        view.open_item_internal(id, false);
                    }
                }
            }
        }));
    }
    fn status(&self, message: &str) {
        self.label("StatusText").set_text(Some(message));
        self.get_control::<Border>("StatusBar").set_is_visible(true);
    }
    fn empty(&self, title: &str, message: &str) {
        self.label("EmptyTitle").set_text(Some(title));
        self.label("EmptyMessage").set_text(Some(message));
        self.get_control::<StackPanel>("FeedEmpty")
            .set_is_visible(self.get_control::<ListBox>("Stories").items().count() == 0);
    }
    fn changed(&self) {
        if !self.state_ready.get() {
            return;
        }
        self.revision.set(self.revision.get().wrapping_add(1));
        self.persist();
    }
    fn persist(&self) {
        if !self.state_ready.get() {
            return;
        }
        if self.saving.replace(true) {
            return;
        }
        let Some(repo) = self.repo() else {
            self.saving.set(false);
            return;
        };
        let weak = self.to_ref().downgrade();
        drop(Dispatcher::ui_thread().invoke_async_task_local(move||async move{
            loop {
                let Some(v)=weak.upgrade()else{return;};let revision=v.revision.get();let state=v.state.borrow().clone();drop(v);
                let result=repo.save_state(&state).await;
                let Some(v)=weak.upgrade()else{return;};
                if let Err(error)=result {v.saving.set(false);v.status(&format!("Changes are not saved: {error}. Retry by changing the setting or bookmark again."));return;}
                if revision==v.revision.get(){v.saving.set(false);return;}
            }
        }));
    }
    /// Starts evicting the oldest cached items beyond the limit; returns whether it started.
    fn prune_cache(&self) -> bool {
        if !self.state_ready.get() || self.downloading.get() || self.pruning.replace(true) {
            return false;
        }
        let Some(repo) = self.repo() else {
            self.pruning.set(false);
            return false;
        };
        let state = self.state.borrow().clone();
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let result = repo.prune_cache(&state).await;
                if let Some(v) = weak.upgrade() {
                    v.pruning.set(false);
                    if let Err(error) = result {
                        v.status(&format!("Cache cleanup could not finish: {error}"));
                    }
                }
            }),
        );
        true
    }
    fn select_feed(&self, route: FeedRoute) {
        if let Some(scroll) = self
            .get_control::<ListBox>("Stories")
            .scroll()
            .and_then(|s| s.cast::<ScrollViewer>())
        {
            scroll.set_offset(ferroui_base::Vector::default());
        }
        self.feed.replace(route);
        self.stories.borrow_mut().clear();
        self.search_story_ids.borrow_mut().clear();
        self.next.set(None);
        self.back_to_feed(true);
        self.load_feed(0, FetchMode::CacheFirst);
    }
    fn refresh(&self) {
        if let DetailRoute::Thread(id) = self.detail.borrow().clone() {
            self.fetch_thread(id, FetchMode::Refresh);
        }
        self.load_feed(0, FetchMode::Refresh);
    }
    fn load_feed(&self, offset: usize, mode: FetchMode) {
        let Some(repo) = self.repo() else {
            return;
        };
        let generation = self.feed_generation.get().wrapping_add(1);
        self.feed_generation.set(generation);
        self.busy.set(true);
        self.button("LoadMoreButton").set_is_enabled(false);
        self.button("RetryButton").set_is_visible(false);
        if self.stories.borrow().is_empty() {
            self.empty("Loading stories…", "Checking for stories on this device.");
        }
        let route = self.feed.borrow().clone();
        let title = match &route {
            FeedRoute::Feed(f) => f.title(),
            FeedRoute::Saved => "Saved for later",
            FeedRoute::History => "Reading history",
            FeedRoute::Hidden => "Hidden stories",
            FeedRoute::Search(_) => "Search results",
        };
        self.label("FeedTitle").set_text(Some(title));
        self.label("FeedSummary").set_text(Some("Updating…"));
        let local_ids = match &route {
            FeedRoute::Saved => self.state.borrow().saved_ids(),
            FeedRoute::History => self.state.borrow().history_ids(),
            FeedRoute::Hidden => self
                .state
                .borrow()
                .items
                .iter()
                .filter_map(|(&id, item)| item.hidden.then_some(id))
                .collect(),
            _ => Vec::new(),
        };
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                if offset == 0 && mode != FetchMode::Offline {
                    if let FeedRoute::Feed(kind) = route {
                        if let Ok(cached) = repo
                            .load_feed(kind, 0, 30, services::now(), FetchMode::Offline)
                            .await
                        {
                            if let Some(view) = weak.upgrade().filter(|v| {
                                v.feed_generation.get() == generation && !cached.items.is_empty()
                            }) {
                                view.accept_items(
                                    cached.items,
                                    0,
                                    cached.next_offset,
                                    cached.total,
                                    cached.source,
                                    None,
                                );
                            }
                        }
                    }
                }
                let result = match route {
                    FeedRoute::Feed(kind) => repo
                        .load_feed(kind, offset, 30, services::now(), mode)
                        .await
                        .map(|page| {
                            (
                                page.items,
                                page.next_offset,
                                page.total,
                                page.source,
                                page.warning,
                                BTreeMap::new(),
                            )
                        }),
                    FeedRoute::Search(mut query) => {
                        query.page = offset;
                        repo.search(&query, services::now(), mode)
                            .await
                            .map(|page| {
                                let next = (page.page + 1 < page.pages).then_some(page.page + 1);
                                (
                                    page.items,
                                    next,
                                    page.total,
                                    page.source,
                                    page.warning,
                                    page.story_ids,
                                )
                            })
                    }
                    FeedRoute::Saved | FeedRoute::History | FeedRoute::Hidden => {
                        let ids = local_ids;
                        let end = (offset + 30).min(ids.len());
                        let batch = repo
                            .load_items(
                                ids.get(offset..end).unwrap_or_default(),
                                services::now(),
                                mode,
                            )
                            .await;
                        Ok((
                            batch.items,
                            (end < ids.len()).then_some(end),
                            ids.len(),
                            batch.source,
                            batch.warning,
                            BTreeMap::new(),
                        ))
                    }
                };
                let Some(view) = weak
                    .upgrade()
                    .filter(|v| v.feed_generation.get() == generation)
                else {
                    return;
                };
                view.busy.set(false);
                view.button("LoadMoreButton").set_is_enabled(true);
                match result {
                    Ok((items, next, total, source, warning, story_ids)) => {
                        if offset == 0 {
                            view.search_story_ids.borrow_mut().clear();
                        }
                        view.search_story_ids.borrow_mut().extend(story_ids);
                        view.accept_items(items, offset, next, total, source, warning);
                    }
                    Err(error) => {
                        view.label("FeedSummary")
                            .set_text(Some("Update unavailable"));
                        view.button("RetryButton").set_is_visible(true);
                        view.button("LoadMoreButton")
                            .set_is_visible(view.next.get().is_some());
                        view.status(&format!("Could not refresh: {error}"));
                        view.empty(
                            "Couldn’t load stories",
                            "Check your connection and try again.",
                        );
                    }
                }
            }),
        );
    }
    fn accept_items(
        &self,
        items: Vec<HnItem>,
        offset: usize,
        next: Option<usize>,
        total: usize,
        source: DataSource,
        warning: Option<String>,
    ) {
        if offset == 0 {
            *self.stories.borrow_mut() = items;
        } else {
            let mut stories = self.stories.borrow_mut();
            let mut ids: BTreeSet<_> = stories.iter().map(|v| v.id).collect();
            stories.extend(items.into_iter().filter(|v| ids.insert(v.id)));
        }
        self.next.set(next);
        self.button("LoadMoreButton").set_is_visible(next.is_some());
        let route = self.feed.borrow();
        let noun = if matches!(*route, FeedRoute::Search(_)) {
            if total == 1 {
                "result"
            } else {
                "results"
            }
        } else if total == 1 {
            "story"
        } else {
            "stories"
        };
        let origin = match source {
            DataSource::Network => "Updated",
            DataSource::Cache => "On this device",
            DataSource::StaleCache => "Offline copy",
        };
        let count = match &*route {
            FeedRoute::Search(q) if !q.query.is_empty() => {
                format!("{total} {noun} for “{}”", q.query)
            }
            _ => format!("{total} {noun}"),
        };
        self.label("FeedSummary")
            .set_text(Some(&format!("{count} · {origin}")));
        drop(route);
        if let Some(warning) = warning {
            self.status(&warning);
        }
        self.render_stories();
        // Pruning reads every cached item, so it runs once a session after a feed has loaded,
        // and again after a download or a change of the cache limit, not after every refresh.
        if !self.busy.get() && !self.pruned_after_feed.get() && self.prune_cache() {
            self.pruned_after_feed.set(true);
        }
    }
    fn visible_stories(&self) -> Vec<HnItem> {
        let state = self.state.borrow();
        let route = self.feed.borrow();
        self.stories
            .borrow()
            .iter()
            .filter(|item| match &*route {
                FeedRoute::Saved => state.is_saved(item.id),
                FeedRoute::History => state.is_read(item.id),
                FeedRoute::Hidden => state.items.get(&item.id).is_some_and(|local| local.hidden),
                _ => state.is_visible(item),
            })
            .cloned()
            .collect()
    }
    fn render_stories(&self) {
        let state = self.state.borrow();
        let stories = self.visible_stories();
        let list = self.get_control::<ListBox>("Stories");
        let visibility_changed = self.rendered_show_dead.replace(state.preferences.show_dead)
            != state.preferences.show_dead;
        if visibility_changed || *self.rendered_stories.borrow() != stories {
            let scroll = list.scroll().and_then(|s| s.cast::<ScrollViewer>());
            let offset = scroll.as_ref().map(|s| s.offset()).unwrap_or_default();
            let focused = list.is_keyboard_focus_within();
            *self.rendered_stories.borrow_mut() = stories.clone();
            list.items().clear();
            for item in stories {
                list.items().add(Some(Rc::new(item) as BoxedValue));
            }
            if let Some(scroll) = scroll {
                let weak = self.to_ref().downgrade();
                let generation = self.feed_generation.get();
                Dispatcher::ui_thread().post_local(
                    move || {
                        if let Some(view) = weak
                            .upgrade()
                            .filter(|v| v.feed_generation.get() == generation)
                        {
                            scroll.set_offset(offset);
                            if focused {
                                view.get_control::<ListBox>("Stories").focus();
                            }
                        }
                    },
                    DispatcherPriority::LOADED,
                );
            }
        } else {
            // Keep containers, scroll position and keyboard focus when only local state changes.
            for card in list
                .get_visual_descendants()
                .filter_map(|v| v.cast::<StoryCard>())
            {
                if let Some(id) = card.tag().and_then(|v| v.downcast_ref::<u64>().copied()) {
                    self.update_card_state(&card, id, &state);
                }
            }
        }
        let empty = list.items().count() == 0;
        self.get_control::<StackPanel>("FeedEmpty")
            .set_is_visible(empty);
        let route = self.feed.borrow();
        let filterable = matches!(*route, FeedRoute::Feed(_) | FeedRoute::Search(_));
        if empty && !self.busy.get() {
            let (title, message) = match &*route {
                FeedRoute::Saved => (
                    "No saved stories",
                    "Bookmark a story to read it here later.",
                ),
                FeedRoute::History => (
                    "No reading history",
                    "Stories appear here when you open a discussion.",
                ),
                FeedRoute::Hidden => (
                    "No hidden stories",
                    "Stories you hide can be restored here.",
                ),
                _ if !self.stories.borrow().is_empty() => (
                    "No visible stories",
                    "Change the unread filter or mute rules, or load more stories.",
                ),
                FeedRoute::Search(_) => (
                    "No results",
                    "Try a different search term or broaden your filters.",
                ),
                _ => ("No stories yet", "Refresh to check for new stories."),
            };
            self.empty(title, message);
        }
        let unread = self.button("UnreadButton");
        unread.set_is_visible(filterable);
        unread.classes().set("active", state.preferences.hide_read);
        let label = if state.preferences.hide_read {
            "Show all stories"
        } else {
            "Show unread only"
        };
        AutomationProperties::set_name(&unread, Some(label));
        ToolTip::set_tip(&unread, Some(Rc::new(label.to_owned()) as BoxedValue));
    }
    fn story_card(&self, item: &HnItem) -> Ref<StoryCard> {
        let card = StoryCard::new();
        let state = self.state.borrow();
        card.set_tag(Some(Rc::new(item.id)));
        card.set_item(item, services::now(), state.preferences.show_dead);
        self.update_card_state(&card, item.id, &state);
        let id = item.id;
        let weak = self.to_ref().downgrade();
        card.click(move |_, _| {
            if let Some(v) = weak.upgrade() {
                v.open_item(id);
            }
        });
        self.wire_card_swipe(&card, id);
        card
    }
    fn update_card_state(&self, card: &StoryCard, id: u64, state: &ReaderState) {
        let read = state.is_read(id);
        let saved = state.is_saved(id);
        card.set_status_label(Some(if saved {
            "SAVED"
        } else if read {
            "READ"
        } else {
            ""
        }));
        card.classes().set("read", read);
        card.classes().set("saved", saved);
        card.classes().set("compact", state.preferences.compact);
        card.classes()
            .set("selected", self.selected_row.get() == Some(id));
        AutomationProperties::set_help_text(
            card,
            Some(match (saved, read) {
                (true, true) => "Saved, read. Open discussion.",
                (true, false) => "Saved. Open discussion.",
                (false, true) => "Read. Open discussion.",
                (false, false) => "Open discussion.",
            }),
        );
    }
    fn wire_card_swipe(&self, card: &StoryCard, id: u64) {
        let swipe = SwipeGestureRecognizer::new();
        swipe.set_can_horizontally_swipe(true);
        swipe.set_can_vertically_swipe(false);
        card.gesture_recognizers()
            .add(swipe.upcast::<ferroui_base::input::gesture_recognizers::GestureRecognizer>());
        let distance = Rc::new(Cell::new(0.0));
        let distance_move = distance.clone();
        card.add_handler(InputElement::swipe_gesture_event(), move |_, e| {
            // FerroUI reports content displacement; invert it to follow the finger.
            distance_move.set(distance_move.get() - e.delta().x);
        });
        let weak = self.to_ref().downgrade();
        card.add_handler(InputElement::swipe_gesture_ended_event(), move |_, _| {
            let delta = distance.replace(0.0);
            if let Some(v) = weak.upgrade() {
                if delta < -80.0 {
                    v.toggle_save(id);
                } else if delta > 80.0 {
                    v.state.borrow_mut().toggle_hidden(id);
                    v.changed();
                    v.render_stories();
                    v.status("Story hidden. Restore it from Hidden stories.");
                }
            }
        });
    }
    fn current_item(&self) -> Option<HnItem> {
        if !matches!(*self.detail.borrow(), DetailRoute::Thread(_)) {
            return None;
        }
        self.thread
            .borrow()
            .as_ref()
            .filter(|v| Some(v.root.id) == self.selected.get())
            .map(|v| v.root.clone())
    }
    fn open_item(&self, id: u64) {
        self.open_item_internal(id, true);
    }
    fn open_item_internal(&self, row: u64, publish: bool) {
        let id = self
            .search_story_ids
            .borrow()
            .get(&row)
            .copied()
            .unwrap_or(row);
        self.selected_row.set(Some(row));
        self.pending_comment.set((row != id).then_some(row));
        self.navigate(DetailRoute::Thread(id));
        self.selected.set(Some(id));
        self.thread_budget.set(120);
        self.label("DetailEyebrow").set_text(Some("DISCUSSION"));
        self.thread.borrow_mut().take();
        self.thread_control.borrow_mut().take();
        let loading = widgets::stack(16.0);
        loading.set_margin(Thickness::uniform(28.0));
        loading
            .children()
            .add(widgets::heading("Opening the conversation"));
        loading.children().add(widgets::text(
            "Checking this device for a saved copy, then connecting to Hacker News…",
            15.0,
        ));
        self.show_content(loading.upcast());
        self.update_save_button();
        self.label("DetailTitle")
            .set_text(Some("Opening the conversation…"));
        self.textbox("ThreadFind").set_text(Some(""));
        self.fetch_thread(id, FetchMode::CacheFirst);
        self.render_stories();
        if publish {
            self.publish_location(Some(id));
        }
    }
    fn fetch_thread(&self, id: u64, mode: FetchMode) {
        let Some(repo) = self.repo() else {
            return;
        };
        let generation = self.detail_generation.get().wrapping_add(1);
        self.detail_generation.set(generation);
        let budget = self.thread_budget.get();
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                if mode != FetchMode::Offline {
                    if let Ok(mut cached) = repo
                        .load_thread(id, budget, services::now(), FetchMode::Offline)
                        .await
                    {
                        if let Some(v) = weak.upgrade() {
                            if v.detail_generation.get() == generation {
                                // Missing records during a deliberate cache preview are expected;
                                // the network result below decides whether an error is actionable.
                                cached.warning = None;
                                v.accept_thread(cached);
                            }
                        }
                    }
                }
                let result = repo.load_thread(id, budget, services::now(), mode).await;
                if let Some(v) = weak.upgrade() {
                    if v.detail_generation.get() != generation {
                        return;
                    }
                    match result {
                        Ok(thread) => v.accept_thread(thread),
                        Err(error) => {
                            v.status(&format!("Discussion unavailable: {error}. Try Refresh."));
                            v.label("DetailTitle")
                                .set_text(Some("Could not load this discussion"));
                            if v.thread.borrow().is_none() {
                                let message = widgets::text("This discussion is not available on this device yet. Check your connection and tap Refresh to try again.", 17.0);
                                message.set_margin(Thickness::uniform(28.0));
                                v.show_content(message.upcast());
                            }
                        }
                    }
                }
            }),
        );
    }
    fn accept_thread(&self, thread: ThreadSnapshot) {
        let id = thread.root.id;
        if self.selected.get() != Some(id) || *self.detail.borrow() != DetailRoute::Thread(id) {
            return;
        }
        let title = if thread.root.kind == Some(ember_core::ItemKind::Job) {
            "Job listing".to_owned()
        } else {
            let count = thread.root.descendants.unwrap_or_else(|| {
                thread
                    .items
                    .values()
                    .filter(|item| {
                        item.id != id && item.kind == Some(ember_core::ItemKind::Comment)
                    })
                    .count() as u64
            });
            let noun = if count == 1 { "comment" } else { "comments" };
            format!("{count} {noun}")
        };
        self.label("DetailTitle").set_text(Some(&title));
        self.label("DetailEyebrow").set_text(Some("DISCUSSION"));
        if let Some(warning) = thread.warning.as_deref() {
            self.status(warning);
        }
        self.state.borrow_mut().mark_read(id, services::now());
        *self.thread.borrow_mut() = Some(thread);
        self.changed();
        self.render_thread();
        self.render_stories();
    }
    fn render_thread(&self) {
        if !matches!(*self.detail.borrow(), DetailRoute::Thread(_)) {
            return;
        }
        let snapshot = self.thread.borrow();
        let Some(thread) = snapshot.as_ref() else {
            return;
        };
        if Some(thread.root.id) != self.selected.get() {
            return;
        }
        let state = self.state.borrow();
        let query = self.textbox("ThreadFind").text().unwrap_or_default();
        let target = self
            .pending_comment
            .get()
            .filter(|id| thread.items.contains_key(id));
        if target.is_some() {
            self.pending_comment.set(None);
        }
        let anchor = target.or_else(|| match self.thread_control.borrow().as_ref() {
            Some(control) => crate::thread_view::viewport_anchor(control),
            None => state
                .items
                .get(&thread.root.id)
                .and_then(|local| local.scroll_anchor),
        });
        let weak = self.to_ref().downgrade();
        let control = crate::thread_view::build(
            thread,
            &state,
            &query,
            self.thread_sort.get(),
            Rc::new(move |action| {
                if let Some(v) = weak.upgrade() {
                    v.thread_action(action);
                }
            }),
        );
        drop(state);
        drop(snapshot);
        self.show_content(control.clone());
        if let Some(id) = anchor {
            crate::thread_view::restore_anchor(&control, id);
        }
        *self.thread_control.borrow_mut() = Some(control);
        self.update_save_button();
    }
    /// Keep the baseline stable while reading so newly fetched replies retain their NEW badge.
    fn remember_thread(&self) {
        let route = self.detail.borrow().clone();
        let DetailRoute::Thread(id) = route else {
            return;
        };
        let snapshot = self.thread.borrow();
        let Some(thread) = snapshot.as_ref() else {
            return;
        };
        if thread.root.id != id {
            return;
        }
        let anchor = self
            .thread_control
            .borrow()
            .as_ref()
            .and_then(|c| crate::thread_view::viewport_anchor(c));
        let mut state = self.state.borrow_mut();
        let local = state.items.entry(id).or_default();
        local.seen_comments.extend(thread.items.keys().copied());
        local.scroll_anchor = anchor;
        drop(state);
        self.changed();
    }
    fn thread_action(&self, action: ThreadAction) {
        match action {
            ThreadAction::OpenUrl(url) => self.open_url(&url),
            ThreadAction::Profile(user) => self.show_profile(user),
            ThreadAction::ToggleAll => self.toggle_collapse_all(),
            ThreadAction::ToggleCollapse(id) => {
                if let Some(root) = self.selected.get() {
                    let mut state = self.state.borrow_mut();
                    let entry = state.items.entry(root).or_default();
                    if !entry.collapsed.remove(&id) {
                        entry.collapsed.insert(id);
                    }
                    drop(state);
                    self.changed();
                    self.render_thread();
                }
            }
            ThreadAction::FocusReading => self.set_focus_reading(true),
            ThreadAction::Share => {
                if let Some(id) = self.selected.get() {
                    self.copy_text(&ember_core::item_url(id), "Discussion link copied.");
                }
            }
            ThreadAction::Hide => {
                if let Some(id) = self.selected.get() {
                    let hidden = self.state.borrow_mut().toggle_hidden(id);
                    self.changed();
                    self.render_stories();
                    self.render_thread();
                    self.status(if hidden {
                        "Story hidden. Find it under Hidden stories."
                    } else {
                        "Story restored to your feeds."
                    });
                }
            }
            ThreadAction::MuteDomain => {
                if let Some(item) = self.current_item() {
                    self.state.borrow_mut().mutes.domains.insert(item.domain());
                    self.changed();
                    self.render_stories();
                    self.status("Domain muted. Manage mutes in Settings.");
                }
            }
            ThreadAction::Reply(id) => self.open_url(&ember_core::reply_url(id)),
            ThreadAction::Vote(id) => self.open_url(&ember_core::item_url(id)),
            ThreadAction::LoadMore => {
                self.thread_budget
                    .set((self.thread_budget.get() + 200).min(10000));
                if let Some(id) = self.selected.get() {
                    self.fetch_thread(id, FetchMode::CacheFirst);
                }
            }
            ThreadAction::Sort(sort) => {
                self.thread_sort.set(sort);
                self.render_thread();
            }
            ThreadAction::Parent(id) => self.open_item(id),
        }
    }
    fn toggle_collapse_all(&self) {
        let snapshot = self.thread.borrow();
        let Some(thread) = snapshot.as_ref() else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let entry = state.items.entry(thread.root.id).or_default();
        if entry.collapsed.is_empty() {
            entry.collapsed.extend(thread.root.kids.iter().copied());
        } else {
            entry.collapsed.clear();
        }
        drop(state);
        self.changed();
        self.render_thread();
    }
    fn toggle_save(&self, id: u64) {
        if !self.state_ready.get() {
            self.status("Saving is paused because reading data could not be restored. Import a backup in Settings to recover it.");
            return;
        }
        let saved = self.state.borrow_mut().toggle_saved(id, services::now());
        self.changed();
        self.render_stories();
        self.update_save_button();
        self.status(if saved {
            "Added to Saved stories. Download the thread to keep all its comments offline."
        } else {
            "Removed from Saved stories."
        });
    }
    fn update_save_button(&self) {
        let saved = self
            .selected
            .get()
            .is_some_and(|id| self.state.borrow().is_saved(id));
        self.button("SaveButton").classes().set("active", saved);
        let tip = if saved {
            "Remove saved story (S)"
        } else {
            "Save story (S)"
        };
        ToolTip::set_tip(
            &self.button("SaveButton"),
            Some(Rc::new(tip.to_owned()) as BoxedValue),
        );
        AutomationProperties::set_name(
            &self.button("SaveButton"),
            Some(if saved {
                "Remove saved story"
            } else {
                "Save story"
            }),
        );
    }
    fn download(&self) {
        if self.downloading.get() {
            return;
        }
        if !self.state_ready.get() {
            self.status("Download is paused until reading data is restored. Import a backup in Settings first.");
            return;
        }
        let Some(id) = self.selected.get() else {
            return;
        };
        let Some(repo) = self.repo() else {
            return;
        };
        self.downloading.set(true);
        self.button("DownloadButton").set_is_enabled(false);
        self.status("Downloading discussion for offline reading…");
        let weak = self.to_ref().downgrade();
        drop(Dispatcher::ui_thread().invoke_async_task_local(move || async move {
            let result = repo.download_thread(id, 10000, services::now()).await;
            let Some(v) = weak.upgrade() else {
                return;
            };
            v.downloading.set(false);
            v.button("DownloadButton").set_is_enabled(true);
            match result {
                Ok(thread) => {
                    if thread.complete {
                        let mut ids: Vec<_> = thread.items.keys().copied().collect();
                        ids.push(thread.root.id);
                        let mut state = v.state.borrow_mut();
                        state.downloaded_threads.insert(id, ids);
                        state
                            .items
                            .entry(id)
                            .or_default()
                            .saved_at
                            .get_or_insert(services::now());
                        drop(state);
                        v.changed();
                        v.status("Discussion downloaded. It is available offline in Saved stories.");
                    } else {
                        v.status(&format!(
                            "Partial download: {} comments stored; {} still missing or pending. Retry Download to continue.",
                            thread.items.len(),
                            thread.missing_ids.len() + thread.pending_ids.len()
                        ));
                    }
                    if v.selected.get() == Some(id)
                        && matches!(*v.detail.borrow(), DetailRoute::Thread(_))
                    {
                        v.accept_thread(thread);
                    }
                    // A download adds items to the cache.
                    v.prune_cache();
                }
                Err(error) => v.status(&format!("Download could not complete: {error}")),
            }
        }));
    }
    fn navigate(&self, route: DetailRoute) {
        if *self.detail.borrow() != route {
            self.remember_thread();
        }
        let old = self.detail.replace(route.clone());
        if old != route && old != DetailRoute::Welcome {
            self.history.borrow_mut().push(old);
        }
        self.detail_generation
            .set(self.detail_generation.get().wrapping_add(1));
        self.configure_detail();
        self.apply_layout();
        self.update_navigation();
        self.animate_detail();
    }
    fn configure_detail(&self) {
        let thread = matches!(*self.detail.borrow(), DetailRoute::Thread(_));
        if !thread {
            self.focus_reading.set(false);
        }
        self.get_control::<Border>("ThreadTools")
            .set_is_visible(false);
        self.button("FindButton").classes().set("active", false);
        for name in ["SaveButton", "FindButton", "DownloadButton"] {
            self.button(name).set_is_visible(thread);
        }
        self.get_control::<Border>("DetailHeader")
            .set_is_visible(*self.detail.borrow() != DetailRoute::Welcome);
    }
    fn set_find_open(&self, open: bool) {
        self.get_control::<Border>("ThreadTools")
            .set_is_visible(open);
        self.button("FindButton").classes().set("active", open);
        if open {
            self.textbox("ThreadFind").focus();
        } else {
            self.textbox("ThreadFind").set_text(Some(""));
            self.button("FindButton").focus();
        }
    }
    fn show_content(&self, control: Ref<Control>) {
        self.get_control::<ContentControl>("DetailHost")
            .set_content(Some(Control::boxed(&control)));
    }
    fn back_to_feed(&self, publish: bool) {
        self.remember_thread();
        self.history.borrow_mut().clear();
        self.selected.set(None);
        self.selected_row.set(None);
        self.detail.replace(DetailRoute::Welcome);
        self.detail_generation
            .set(self.detail_generation.get().wrapping_add(1));
        self.configure_detail();
        let weak = self.to_ref().downgrade();
        self.show_content(widgets::welcome(move || {
            if let Some(v) = weak.upgrade() {
                v.show_search();
            }
        }));
        self.label("DetailEyebrow").set_text(Some("READER"));
        self.label("DetailTitle").set_text(Some("Discussion"));
        self.apply_layout();
        self.update_navigation();
        self.render_stories();
        if publish {
            self.publish_location(None);
        }
    }
    fn go_back(&self) {
        if self.get_control::<Border>("ThreadTools").is_visible() {
            self.set_find_open(false);
            return;
        }
        if self.focus_reading.get() {
            self.set_focus_reading(false);
            return;
        }
        let route = self.history.borrow_mut().pop();
        self.remember_thread();
        // Restoring a route must not put the outgoing page back onto the history stack.
        self.detail.replace(DetailRoute::Welcome);
        match route {
            Some(DetailRoute::Thread(id)) => {
                self.open_item_internal(id, true);
            }
            Some(DetailRoute::Profile(user)) => self.show_profile(user),
            Some(DetailRoute::Settings) => self.show_settings(),
            Some(DetailRoute::Search) => self.show_search(),
            Some(DetailRoute::More) => self.show_more(),
            _ => self.back_to_feed(true),
        }
    }
    fn publish_location(&self, id: Option<u64>) {
        LOCATION_CHANGED.with(|v| {
            if let Some(callback) = v.borrow().clone() {
                callback(id);
            }
        });
    }
    fn submit_quick_search(&self) {
        let query = self.textbox("SearchBox").text().unwrap_or_default();
        if query.trim().is_empty() {
            self.show_search();
        } else {
            self.run_search(SearchQuery {
                query,
                ..Default::default()
            });
        }
    }
    fn show_search(&self) {
        if *self.detail.borrow() == DetailRoute::Search {
            if let Some(query) = self
                .get_control::<ContentControl>("DetailHost")
                .get_visual_descendants()
                .filter_map(|node| node.cast::<TextBox>())
                .find(|input| input.name().as_deref() == Some("SearchTerms"))
            {
                query.focus();
                return;
            }
        }
        self.navigate(DetailRoute::Search);
        self.label("DetailEyebrow").set_text(Some("DISCOVER"));
        self.label("DetailTitle").set_text(Some("Search"));
        let query = match &*self.feed.borrow() {
            FeedRoute::Search(q) => q.clone(),
            _ => SearchQuery {
                query: self.textbox("SearchBox").text().unwrap_or_default(),
                ..Default::default()
            },
        };
        let weak = self.to_ref().downgrade();
        let profile = self.to_ref().downgrade();
        self.show_content(crate::search_panel::build(
            &query,
            &self.state.borrow().recent_searches,
            Rc::new(move |query| {
                if let Some(v) = weak.upgrade() {
                    v.run_search(query);
                }
            }),
            Rc::new(move |name| {
                if let Some(v) = profile.upgrade() {
                    v.show_profile(name);
                }
            }),
        ));
    }
    fn run_search(&self, mut query: SearchQuery) {
        query.query = query.query.trim().to_owned();
        query.page = 0;
        self.state.borrow_mut().remember_search(&query.query);
        self.changed();
        self.textbox("SearchBox").set_text(Some(&query.query));
        self.select_feed(FeedRoute::Search(query));
    }
    fn show_settings(&self) {
        self.navigate(DetailRoute::Settings);
        self.label("DetailEyebrow").set_text(Some("READER"));
        self.label("DetailTitle").set_text(Some("Settings"));
        let weak = self.to_ref().downgrade();
        self.show_content(crate::settings::build(
            &self.state.borrow(),
            Rc::new(move |action| {
                if let Some(v) = weak.upgrade() {
                    v.setting_action(action);
                }
            }),
        ));
    }
    fn setting_action(&self, action: SettingsAction) {
        match action {
            SettingsAction::Preferences(mut preferences) => {
                preferences.normalize();
                crate::theme::apply(&preferences);
                self.state.borrow_mut().preferences = preferences;
                self.changed();
                self.render_stories();
                self.prune_cache();
                self.status("Preferences applied.");
            }
            SettingsAction::Mutes(mutes) => {
                self.state.borrow_mut().mutes = mutes;
                self.changed();
                self.render_stories();
                self.status("Mute rules updated.");
            }
            SettingsAction::Export(show_json) => match self.state.borrow().export_json() {
                Ok(json) => {
                    show_json(&json);
                    self.copy_text(&json, "Backup copied.");
                }
                Err(error) => self.status(&error.to_string()),
            },
            SettingsAction::Import(state) => {
                crate::theme::apply(&state.preferences);
                *self.state.borrow_mut() = state;
                self.state_ready.set(true);
                self.changed();
                self.render_stories();
                self.show_settings();
                self.status("Reading library imported.");
            }
            SettingsAction::ClearHistory => {
                self.state.borrow_mut().recent_searches.clear();
                for entry in self.state.borrow_mut().items.values_mut() {
                    entry.read_at = None;
                    entry.last_opened_at = None;
                    entry.seen_comments.clear();
                    entry.scroll_anchor = None;
                }
                self.changed();
                self.render_stories();
                self.status("Reading history cleared. Saved stories are retained.");
            }
            SettingsAction::ClearCache => {
                let Some(repo) = self.repo() else {
                    return;
                };
                let weak = self.to_ref().downgrade();
                drop(Dispatcher::ui_thread().invoke_async_task_local(move||async move{let result=repo.clear_cache().await;if let Some(v)=weak.upgrade(){match result{Ok(())=>{v.state.borrow_mut().downloaded_threads.clear();v.changed();v.status("Cached content cleared. Bookmarks and preferences are retained.");},Err(error)=>v.status(&error.to_string())}}}));
            }
        }
    }
    fn show_more(&self) {
        self.navigate(DetailRoute::More);
        self.label("DetailEyebrow").set_text(Some("NAVIGATION"));
        self.label("DetailTitle").set_text(Some("Explore Ember"));
        let panel = widgets::stack(3.0);
        panel.set_max_width(620.0);
        panel.set_margin(Thickness::uniform(15.0));
        panel.children().add(widgets::muted("FEEDS"));
        for (label, icon, kind) in [
            ("Best stories", "star", FeedKind::Best),
            ("Ask HN", "chat", FeedKind::Ask),
            ("Show HN", "lightbulb", FeedKind::Show),
            ("Jobs", "briefcase", FeedKind::Jobs),
        ] {
            let weak = self.to_ref().downgrade();
            panel.children().add(
                widgets::action(label, icon, move || {
                    if let Some(v) = weak.upgrade() {
                        v.select_feed(FeedRoute::Feed(kind));
                    }
                })
                .upcast::<Control>(),
            );
        }
        panel.children().add(widgets::muted("LIBRARY"));
        for (label, icon, route) in [
            ("Reading history", "history", FeedRoute::History),
            ("Hidden stories", "eye_off", FeedRoute::Hidden),
        ] {
            let weak = self.to_ref().downgrade();
            panel.children().add(
                widgets::action(label, icon, move || {
                    if let Some(v) = weak.upgrade() {
                        v.select_feed(route.clone());
                    }
                })
                .upcast::<Control>(),
            );
        }
        let weak = self.to_ref().downgrade();
        panel.children().add(
            widgets::action("Appearance and settings", "settings", move || {
                if let Some(v) = weak.upgrade() {
                    v.show_settings();
                }
            })
            .upcast::<Control>(),
        );
        panel.children().add(widgets::muted("HACKER NEWS"));
        for (label, url) in [
            ("Sign in on Hacker News", ember_core::login_url()),
            ("Submit on Hacker News", ember_core::submit_url()),
        ] {
            let weak = self.to_ref().downgrade();
            panel.children().add(
                widgets::action(label, "open", move || {
                    if let Some(v) = weak.upgrade() {
                        v.open_url(url);
                    }
                })
                .upcast::<Control>(),
            );
        }
        panel.children().add(widgets::text("Voting, replying and submitting open Hacker News in your browser. Ember's saved stories and reading history stay on this device.",13.0).upcast::<Control>());
        self.show_content(widgets::scroll(panel.upcast::<Control>()));
    }
    fn show_profile(&self, name: String) {
        self.navigate(DetailRoute::Profile(name.clone()));
        self.label("DetailEyebrow").set_text(Some("PROFILE"));
        self.label("DetailTitle").set_text(Some(&name));
        let loading = widgets::muted("Loading profile…");
        loading.set_margin(Thickness::uniform(18.0));
        self.show_content(loading.upcast());
        let Some(repo) = self.repo() else {
            return;
        };
        let generation = self.detail_generation.get();
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let result = repo
                    .load_user(&name, services::now(), FetchMode::CacheFirst)
                    .await;
                if let Some(v) = weak.upgrade() {
                    if v.detail_generation.get() != generation {
                        return;
                    }
                    match result {
                        Ok(loaded) => {
                            if let Some(warning) = loaded.warning.as_deref() {
                                v.status(warning);
                            }
                            if let Some(user) = loaded.value {
                                let panel = widgets::stack(10.0);
                                panel.set_margin(Thickness::uniform(18.0));
                                panel.set_max_width(660.0);
                                panel
                                    .children()
                                    .add(widgets::heading(&user.id).upcast::<Control>());
                                let mut facts = Vec::new();
                                if let Some(karma) = user.karma {
                                    facts.push(format!("{karma} karma"));
                                }
                                if user.created.is_some() {
                                    facts.push(format!(
                                        "Joined {}",
                                        ember_core::relative_time(user.created, services::now())
                                    ));
                                }
                                let count = user.submitted.len();
                                facts.push(format!(
                                    "{count} {}",
                                    if count == 1 {
                                        "submission"
                                    } else {
                                        "submissions"
                                    }
                                ));
                                panel.children().add(widgets::muted(&facts.join(" · ")));
                                let link = v.to_ref().downgrade();
                                let options = crate::thread_view::render_options(&v.state.borrow());
                                panel.children().add(ember_content::render(
                                    &ember_content::HtmlDocument::parse(
                                        user.about.as_deref().unwrap_or("No public biography."),
                                    ),
                                    &options,
                                    Rc::new(move |url| {
                                        if let Some(v) = link.upgrade() {
                                            v.open_url(url);
                                        }
                                    }),
                                ));
                                for (label, icon, kind) in [
                                    ("Stories", "home", SearchKind::Stories),
                                    ("Comments", "chat", SearchKind::Comments),
                                ] {
                                    let username = user.id.clone();
                                    let target = v.to_ref().downgrade();
                                    panel.children().add(
                                        widgets::action(label, icon, move || {
                                            if let Some(v) = target.upgrade() {
                                                v.run_search(SearchQuery {
                                                    author: Some(username.clone()),
                                                    kind,
                                                    ..Default::default()
                                                });
                                            }
                                        })
                                        .upcast::<Control>(),
                                    );
                                }
                                let link = v.to_ref().downgrade();
                                let url = ember_core::user_url(&user.id);
                                panel.children().add(
                                    widgets::action("Profile on Hacker News", "open", move || {
                                        if let Some(v) = link.upgrade() {
                                            v.open_url(&url);
                                        }
                                    })
                                    .upcast::<Control>(),
                                );
                                v.show_content(widgets::scroll(panel.upcast::<Control>()));
                            } else {
                                let message = widgets::muted("This user has no public profile.");
                                message.set_margin(Thickness::uniform(18.0));
                                v.show_content(message.upcast());
                            }
                        }
                        Err(error) => {
                            v.status(&format!("Profile unavailable: {error}"));
                            let panel = widgets::stack(8.0);
                            panel.set_margin(Thickness::uniform(18.0));
                            panel
                                .children()
                                .add(widgets::heading("Couldn’t load profile"));
                            panel.children().add(widgets::muted(
                                "Check the username or your connection, then try again.",
                            ));
                            let target = v.to_ref().downgrade();
                            let retry = widgets::action("Try again", "refresh", move || {
                                if let Some(view) = target.upgrade() {
                                    view.show_profile(name.clone());
                                }
                            });
                            retry.set_horizontal_alignment(
                                ferroui_base::layout::HorizontalAlignment::Left,
                            );
                            panel.children().add(retry);
                            v.show_content(panel.upcast());
                        }
                    }
                }
            }),
        );
    }
    fn open_url(&self, url: &str) {
        let Some(url) = ember_core::safe_web_url(url) else {
            self.status("This link is not a supported web address.");
            return;
        };
        let Ok(uri) = Uri::absolute(&url) else {
            return;
        };
        let Some(top) = TopLevel::get_top_level(Some(self)) else {
            return;
        };
        let pending = top.launcher().launch_uri_async(&uri);
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                if !pending.await {
                    if let Some(v) = weak.upgrade() {
                        v.status(
                            "The link could not be opened. Check your browser's popup permission.",
                        );
                    }
                }
            }),
        );
    }
    fn copy_text(&self, text: &str, success: &str) {
        let Some(clipboard) = TopLevel::get_top_level(Some(self)).and_then(|v| v.clipboard())
        else {
            self.status("Clipboard is unavailable on this platform.");
            return;
        };
        let pending = clipboard.set_text_async(Some(text));
        let weak = self.to_ref().downgrade();
        let success = success.to_owned();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let result = pending.await;
                if let Some(v) = weak.upgrade() {
                    match result {
                        Ok(()) => v.status(&success),
                        Err(error) => v.status(&format!("Clipboard failed: {error}")),
                    }
                }
            }),
        );
    }
    fn move_selection(&self, delta: i32) {
        let stories = self.visible_stories();
        if stories.is_empty() {
            return;
        }
        let current = self
            .selected_row
            .get()
            .and_then(|id| stories.iter().position(|v| v.id == id))
            .map(|v| v as i32)
            .unwrap_or(-1);
        let index = (current + delta).clamp(0, stories.len() as i32 - 1);
        let id = stories[index as usize].id;
        drop(stories);
        self.open_item(id);
        self.get_control::<ListBox>("Stories")
            .scroll_into_view(index);
    }
    fn set_focus_reading(&self, enabled: bool) {
        if !matches!(*self.detail.borrow(), DetailRoute::Thread(_))
            || self.focus_reading.replace(enabled) == enabled
        {
            return;
        }
        let anchor = self
            .thread_control
            .borrow()
            .as_ref()
            .and_then(|c| crate::thread_view::viewport_anchor(c));
        self.apply_layout();
        if let (Some(anchor), Some(thread)) = (anchor, self.thread_control.borrow().as_ref()) {
            crate::thread_view::restore_anchor(thread, anchor);
        }
        let focus = self.button("FocusButton");
        if focus.is_effectively_visible() {
            focus.focus();
        } else {
            self.button("BackButton").focus();
        }
    }
    fn apply_layout(&self) {
        let width = if self.bounds().width > 0.0 {
            self.bounds().width
        } else {
            1440.0
        };
        let mode = if self.focus_reading.get() {
            WorkspaceLayout::Reading
        } else if width < 760.0 {
            WorkspaceLayout::Phone
        } else if width < 1120.0 {
            WorkspaceLayout::CompactRail
        } else {
            WorkspaceLayout::LabeledRail
        };
        let phone = mode == WorkspaceLayout::Phone;
        let reading = mode == WorkspaceLayout::Reading;
        let split = !phone && !reading;
        let detail = *self.detail.borrow() != DetailRoute::Welcome;
        let thread = matches!(*self.detail.borrow(), DetailRoute::Thread(_));
        self.get_control::<Border>("NavigationRail")
            .set_is_visible(split);
        self.get_control::<Border>("BottomNavigation")
            .set_is_visible(phone);
        self.get_control::<Border>("FeedPane")
            .set_is_visible(split || (phone && !detail));
        self.get_control::<Border>("DetailPane")
            .set_is_visible(split || detail);
        self.get_control::<Border>("Masthead")
            .set_is_visible(split || (phone && !detail));
        self.button("ThreadRefresh").set_is_visible(phone && thread);
        self.button("FocusButton").set_is_visible(!phone && thread);
        self.get_control::<Border>("SearchHost")
            .set_is_visible(mode == WorkspaceLayout::LabeledRail);
        self.button("CompactSearch")
            .set_is_visible(mode == WorkspaceLayout::CompactRail);
        self.label("BrandSubtitle").set_is_visible(split);
        if self.layout_mode.replace(Some(mode)) == Some(mode) {
            return;
        }
        let columns = match mode {
            WorkspaceLayout::Phone | WorkspaceLayout::Reading => "0,*,0",
            WorkspaceLayout::CompactRail => "60,290,*",
            WorkspaceLayout::LabeledRail => "172,350,*",
        };
        self.get_control::<Grid>("Workspace")
            .set_column_definitions(ColumnDefinitions::parse(columns).expect("workspace columns"));
        Grid::set_column(
            &self.get_control::<Border>("DetailPane"),
            if split { 2 } else { 1 },
        );
        self.get_control::<Grid>("ReaderColumn")
            .set_max_width(if reading { 820.0 } else { f64::INFINITY });
        let focus = self.button("FocusButton");
        let (label, icon) = if reading {
            ("Exit focus reading", "exit_focus")
        } else {
            ("Focus reading", "focus_reading")
        };
        focus.set_content(Some(Control::boxed(crate::icons::icon(icon))));
        focus.classes().set("active", reading);
        AutomationProperties::set_name(&focus, Some(label));
        ToolTip::set_tip(&focus, Some(Rc::new(format!("{label} (F)")) as BoxedValue));
        self.populate_navigation(mode == WorkspaceLayout::CompactRail);
        self.update_navigation();
    }
    fn update_navigation(&self) {
        let feed = self.feed.borrow().clone();
        let detail = self.detail.borrow().clone();
        for (name, route) in [
            ("NavTop", FeedRoute::Feed(FeedKind::Top)),
            ("MobileHome", FeedRoute::Feed(FeedKind::Top)),
            ("NavNew", FeedRoute::Feed(FeedKind::New)),
            ("MobileNew", FeedRoute::Feed(FeedKind::New)),
            ("NavBest", FeedRoute::Feed(FeedKind::Best)),
            ("NavAsk", FeedRoute::Feed(FeedKind::Ask)),
            ("NavShow", FeedRoute::Feed(FeedKind::Show)),
            ("NavJobs", FeedRoute::Feed(FeedKind::Jobs)),
            ("NavSaved", FeedRoute::Saved),
            ("MobileSaved", FeedRoute::Saved),
            ("NavHistory", FeedRoute::History),
            ("NavHidden", FeedRoute::Hidden),
        ] {
            self.button(name).classes().set(
                "active",
                feed == route
                    && !matches!(
                        detail,
                        DetailRoute::Search | DetailRoute::Settings | DetailRoute::More
                    ),
            );
        }
        self.button("MobileSearch").classes().set(
            "active",
            detail == DetailRoute::Search
                || (matches!(feed, FeedRoute::Search(_))
                    && !matches!(detail, DetailRoute::Settings | DetailRoute::More)),
        );
        self.button("NavMore")
            .classes()
            .set("active", detail == DetailRoute::More);
        self.button("MobileMore").classes().set(
            "active",
            matches!(detail, DetailRoute::Settings | DetailRoute::More)
                || (!matches!(detail, DetailRoute::Search)
                    && matches!(
                        feed,
                        FeedRoute::Feed(
                            FeedKind::Ask | FeedKind::Show | FeedKind::Jobs | FeedKind::Best
                        ) | FeedRoute::History
                            | FeedRoute::Hidden
                    )),
        );
    }
    fn populate_navigation(&self, compact: bool) {
        for (name, icon, label) in [
            ("NavTop", "home", "Front page"),
            ("NavNew", "clock", "Latest"),
            ("NavBest", "star", "Best"),
            ("NavAsk", "chat", "Ask HN"),
            ("NavShow", "lightbulb", "Show HN"),
            ("NavJobs", "briefcase", "Jobs"),
            ("NavSaved", "bookmark", "Saved for later"),
            ("NavHistory", "history", "History"),
            ("NavHidden", "eye_off", "Hidden stories"),
            ("NavMore", "more", "More"),
        ] {
            let button = self.button(name);
            ToolTip::set_tip(&button, Some(Rc::new(label.to_owned()) as BoxedValue));
            button.set_horizontal_content_alignment(if compact {
                ferroui_base::layout::HorizontalAlignment::Center
            } else {
                ferroui_base::layout::HorizontalAlignment::Left
            });
            if compact {
                button.set_padding(Thickness::uniform(0.0));
            } else {
                button.set_padding(Thickness::symmetric(10.0, 2.0));
            }
            button.set_content(Some(Control::boxed(&widgets::nav_content(
                icon, label, compact, false,
            ))));
        }
        for name in ["ExploreLabel", "LibraryLabel"] {
            self.label(name).set_is_visible(!compact);
        }
        for (name, icon, label) in [
            ("MobileHome", "home", "Home"),
            ("MobileNew", "clock", "Latest"),
            ("MobileSearch", "search", "Search"),
            ("MobileSaved", "bookmark", "Saved"),
            ("MobileMore", "more", "More"),
        ] {
            let b = self.button(name);
            b.set_padding(Thickness::symmetric(2.0, 2.0));
            b.set_horizontal_content_alignment(ferroui_base::layout::HorizontalAlignment::Center);
            b.set_content(Some(Control::boxed(&widgets::nav_content(
                icon, label, false, true,
            ))));
        }
    }
    fn animate_detail(&self) {
        if self.state.borrow().preferences.reduced_motion {
            return;
        }
        if let Some(visual) = ElementComposition::get_element_visual(
            &self.get_control::<ContentControl>("DetailHost"),
        ) {
            let animation = visual.compositor().create_scalar_key_frame_animation();
            animation.insert_key_frame(0.0, 0.2);
            animation.insert_key_frame(1.0, 1.0);
            animation.set_duration(Duration::from_millis(180));
            visual.start_animation("Opacity", &*animation);
            let translation = visual.compositor().create_vector3_key_frame_animation();
            translation.insert_key_frame(0.0, ferroui_base::numerics::Vector3::new(0.0, 12.0, 0.0));
            translation.insert_key_frame(1.0, ferroui_base::numerics::Vector3::new(0.0, 0.0, 0.0));
            translation.set_duration(Duration::from_millis(220));
            visual.start_animation("Translation", &*translation);
        }
    }
}
