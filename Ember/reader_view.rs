mod discovery;
mod feed_rows;
mod library_find;
mod navigation;
mod offline;

use crate::controls::{StoryCard, StorySummary};
use crate::markup::xaml_class;
use crate::settings::SettingsAction;
use crate::thread_view::{Discussion, ReadingPosition, ThreadAction};
use crate::{services, widgets};
use ember_core::{
    DataSource, FeedKind, FetchMode, HnItem, ReaderState, Repository, SearchKind, SearchQuery,
    ThreadSnapshot, ThreadSort,
};
use feed_rows::{FeedRows, FeedSummaryState};
use ferroui_base::collections::FerroList;
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
use ferroui_controls::primitives::{TemplatedControlImpl, ToggleButton};
use ferroui_controls::templates::FuncDataTemplate;
use ferroui_controls::{
    Border, Button, ColumnDefinitions, ContentControl, ContentControlImpl, ContextMenu, Control,
    ControlImpl, ControlImplExt, Grid, ListBox, MenuItem, ScrollViewer, StackPanel, TextBlock,
    TextBox, ToolTip, TopLevel, UserControl,
};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
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
impl FeedRoute {
    fn is_library(&self) -> bool {
        matches!(self, Self::Saved | Self::History | Self::Hidden)
    }
    fn includes(&self, item: &HnItem, state: &ReaderState, downloaded_only: bool) -> bool {
        match self {
            Self::Saved if downloaded_only => state.downloaded_threads.contains_key(&item.id),
            Self::Saved => state.is_saved(item.id),
            Self::History => state
                .items
                .get(&item.id)
                .is_some_and(|s| s.last_opened_at.is_some()),
            Self::Hidden => state.items.get(&item.id).is_some_and(|s| s.hidden),
            _ => state.is_visible(item),
        }
    }
}

// Only the six public feeds have resumable sessions. Local shelves are rebuilt from
// current state and search result identities belong to their individual query.
struct FeedWindow {
    stories: FeedRows,
    next: Option<usize>,
    summary: String,
    position: ferroui_base::Vector,
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

const INITIAL_THREAD_BUDGET: usize = 120;
const MAX_BROWSER_VISITS: usize = 24;

#[derive(Clone)]
struct ThreadVisit {
    root: u64,
    source_row: u64,
    pending_comment: Option<u64>,
    budget: usize,
    browser_entry: Option<u64>,
    presentation: Option<Rc<ThreadPresentation>>,
}

impl ThreadVisit {
    fn new(root: u64, source_row: u64, browser_entry: Option<u64>) -> Self {
        Self {
            root,
            source_row,
            pending_comment: (source_row != root).then_some(source_row),
            budget: INITIAL_THREAD_BUDGET,
            browser_entry,
            presentation: None,
        }
    }
}

struct ThreadPresentation {
    position: Option<ReadingPosition>,
    seen: Rc<BTreeSet<u64>>,
    find: Option<FindVisit>,
    new_replies_origin: Option<ReadingPosition>,
    sort: ThreadSort,
    focus_reading: bool,
}

struct FindVisit {
    text: String,
    match_id: Option<u64>,
    origin: Option<ReadingPosition>,
}

/// Entries popped from the app stack can still be visited via browser Forward.
#[derive(Default)]
struct BrowserVisits(VecDeque<ThreadVisit>);

impl BrowserVisits {
    fn remember(&mut self, visit: &ThreadVisit) {
        let Some(entry) = visit.browser_entry else {
            return;
        };
        self.0.retain(|old| old.browser_entry != Some(entry));
        self.0.push_back(visit.clone());
        if self.0.len() > MAX_BROWSER_VISITS {
            self.0.pop_front();
        }
    }

    fn get(&self, entry: u64, root: u64) -> Option<ThreadVisit> {
        self.0
            .iter()
            .rev()
            .find(|visit| visit.browser_entry == Some(entry) && visit.root == root)
            .cloned()
    }
}

enum HistoryEntry {
    Route(DetailRoute),
    Thread(ThreadVisit),
    Search(Box<crate::search_panel::Draft>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CacheWork {
    Idle,
    Pruning,
    Cleaning,
    Downloading(u64),
    Clearing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FeedLoad {
    Ready,
    Loading,
    Unavailable,
    PageUnavailable,
    StorageUnavailable,
}

#[derive(Clone, Copy)]
enum EmptyAction {
    Retry,
    FrontPage,
    EditSearch,
    ShowRead,
    AllSaved,
    ClearLibraryFind,
    Settings,
}

impl EmptyAction {
    fn label(self) -> &'static str {
        match self {
            Self::Retry => "Try again",
            Self::FrontPage => "Browse front page",
            Self::EditSearch => "Edit search",
            Self::ShowRead => "Show read stories",
            Self::AllSaved => "View all saved stories",
            Self::Settings => "Reading settings",
            Self::ClearLibraryFind => "Clear search",
        }
    }
}

#[derive(Clone, Copy)]
enum StoryFlag {
    Saved,
    Hidden,
    Read,
}

#[derive(Clone, Copy)]
enum UndoStoryEdit {
    Saved { id: u64, saved_at: Option<u64> },
    Hidden { id: u64, hidden: bool },
    Read { id: u64, read_at: Option<u64> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspaceLayout {
    Phone,
    ShortLandscape,
    CompactRail,
    LabeledRail,
    Reading,
}

ferroui_base::ferro_thread_local! {
    static INITIAL_BROWSER_ENTRY: Cell<Option<u64>> = Cell::new(None);
    static ACTIVE: RefCell<Option<WeakRef<ReaderView>>> = RefCell::new(None);
    static LOCATION_CHANGED: RefCell<Option<Rc<dyn Fn(Option<u64>)>>> = RefCell::new(None);
}
pub fn set_location_changed(callback: Rc<dyn Fn(Option<u64>)>) {
    LOCATION_CHANGED.with(|v| *v.borrow_mut() = Some(callback));
}
/// Associate the already displayed reader with the host's committed history slot.
pub fn set_browser_entry(entry: Option<u64>) {
    ACTIVE.with(|active| {
        if let Some(view) = active.borrow().as_ref().and_then(|view| view.upgrade()) {
            view.browser_entry.set(entry);
        } else {
            INITIAL_BROWSER_ENTRY.with(|initial| initial.set(entry));
        }
    });
}

pub fn navigate_browser_entry(location: &str, entry: Option<u64>) {
    visit_location(location, entry, false);
}

pub fn navigate_location(location: &str) {
    visit_location(location, None, true);
}

fn visit_location(location: &str, entry: Option<u64>, match_by_item: bool) {
    ACTIVE.with(|active| {
        let Some(view) = active.borrow().as_ref().and_then(|v| v.upgrade()) else {
            return;
        };
        let Some(id) = item_from_location(location) else {
            if *view.detail.borrow() != DetailRoute::Welcome {
                view.return_to_feed(false);
            }
            view.browser_entry.set(entry);
            return;
        };
        let same_thread = *view.detail.borrow() == DetailRoute::Thread(id);
        if same_thread && (match_by_item || (entry.is_some() && view.browser_entry.get() == entry))
        {
            return;
        }
        // A permalink can occur more than once in a browser's history, with
        // different Find sessions and reading positions. External unmarked links
        // start a fresh visit. Native item-only traversal retains its old behavior.
        let matches = |visit: &ThreadVisit| {
            visit.root == id && (match_by_item || (entry.is_some() && visit.browser_entry == entry))
        };
        let previous = view
            .history
            .borrow()
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, route)| match route {
                HistoryEntry::Thread(visit) if matches(visit) => Some((index, visit.clone())),
                _ => None,
            });
        if let Some((index, visit)) = previous {
            view.remember_thread();
            view.history.borrow_mut().truncate(index);
            view.detail.replace(DetailRoute::Welcome);
            view.present_discussion(visit, false);
        } else {
            let visit = entry
                .and_then(|key| view.browser_visits.borrow().get(key, id))
                .unwrap_or_else(|| ThreadVisit::new(id, id, entry));
            if same_thread {
                if let Some(outgoing) = view.remember_thread() {
                    view.history
                        .borrow_mut()
                        .push(HistoryEntry::Thread(outgoing));
                }
                view.detail.replace(DetailRoute::Welcome);
            }
            view.present_discussion(visit, false);
        }
    });
}
fn item_from_location(location: &str) -> Option<u64> {
    location.split(['?', '&', '#']).find_map(|part| {
        part.strip_prefix("item=")
            .or_else(|| part.strip_prefix("id="))
            .or_else(|| part.strip_prefix("item/"))
            .or_else(|| part.strip_prefix("/item/"))
            .filter(|id| id.bytes().all(|byte| byte.is_ascii_digit()))
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
    undo_story: Cell<Option<UndoStoryEdit>>,
    saving: Cell<bool>,
    feed: RefCell<FeedRoute>,
    detail: RefCell<DetailRoute>,
    history: RefCell<Vec<HistoryEntry>>,
    browser_entry: Cell<Option<u64>>,
    browser_visits: RefCell<BrowserVisits>,
    search_form: RefCell<Option<crate::search_panel::Form>>,
    stories: RefCell<FeedRows>,
    feed_windows: RefCell<BTreeMap<FeedKind, FeedWindow>>,
    accepted_feed_summary: RefCell<Option<String>>,
    rendered_stories: Rc<FerroList<Option<BoxedValue>>>,
    feed_summary: RefCell<FeedSummaryState>,
    rendered_show_dead: Cell<bool>,
    search_story_ids: RefCell<BTreeMap<u64, u64>>,
    selected: Cell<Option<u64>>,
    // An Algolia comment result and its containing discussion have different identities.
    selected_row: Cell<Option<u64>>,
    pending_comment: Cell<Option<u64>>,
    next: Cell<Option<usize>>,
    feed_generation: Cell<u64>,
    detail_generation: Cell<u64>,
    feed_load: Cell<FeedLoad>,
    initial_feed_pending: Cell<bool>,
    empty_action: Cell<Option<EmptyAction>>,
    explore_counts: RefCell<Vec<discovery::ExploreCount>>,
    profile_mute: RefCell<Option<Ref<ToggleButton>>>,
    pending_settings: RefCell<Option<crate::settings::RestorePreferences>>,
    settings_back: RefCell<Option<Rc<crate::settings::SettingsBack>>>,
    live_settings: RefCell<Option<crate::settings::LivePreferences>>,
    downloaded_only: Cell<bool>,
    library_find: RefCell<library_find::LibraryFind>,
    thread: RefCell<Option<Rc<Discussion>>>,
    thread_control: RefCell<Option<Ref<Control>>>,
    cache_work: Cell<CacheWork>,
    transfer: RefCell<Option<offline::Transfer>>,
    storage_report: RefCell<Option<Rc<dyn Fn(&str)>>>,
    storage_request: Cell<u64>,
    /// Whether the cache was pruned after a feed loaded this session (see `accept_items`).
    pruned_after_feed: Cell<bool>,
    library_generation: Cell<u64>,
    thread_budget: Cell<usize>,
    thread_sort: Cell<ThreadSort>,
    // New badges are relative to entering the visit. Its immutable baseline is
    // shared with virtual rows and history, even after durable seen IDs advance.
    visit_seen: RefCell<Rc<BTreeSet<u64>>>,
    resume_viewport: Cell<Option<ReadingPosition>>,
    find_revision: Cell<u64>,
    applied_find_query: RefCell<String>,
    find_match_id: Cell<Option<u64>>,
    // Unfiltered reading position (the root identifies the story header).
    find_origin: Cell<Option<ReadingPosition>>,
    new_replies_origin: Cell<Option<ReadingPosition>>,
    focus_reading: Cell<bool>,
    layout_mode: Cell<Option<WorkspaceLayout>>,
    layout_update_queued: Cell<bool>,
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
            undo_story: Cell::new(None),
            saving: Cell::new(false),
            feed: RefCell::new(FeedRoute::Feed(FeedKind::Top)),
            detail: RefCell::new(DetailRoute::Welcome),
            history: RefCell::new(Vec::new()),
            browser_entry: Cell::new(INITIAL_BROWSER_ENTRY.with(|initial| initial.take())),
            browser_visits: RefCell::default(),
            search_form: RefCell::new(None),
            stories: RefCell::default(),
            feed_windows: RefCell::default(),
            accepted_feed_summary: RefCell::default(),
            rendered_stories: Rc::new(FerroList::new()),
            feed_summary: RefCell::default(),
            rendered_show_dead: Cell::new(false),
            search_story_ids: RefCell::new(BTreeMap::new()),
            selected: Cell::new(None),
            selected_row: Cell::new(None),
            pending_comment: Cell::new(None),
            next: Cell::new(None),
            feed_generation: Cell::new(0),
            detail_generation: Cell::new(0),
            feed_load: Cell::new(FeedLoad::Ready),
            initial_feed_pending: Cell::new(false),
            empty_action: Cell::new(None),
            explore_counts: RefCell::new(Vec::new()),
            profile_mute: RefCell::new(None),
            pending_settings: RefCell::new(None),
            settings_back: RefCell::new(None),
            live_settings: RefCell::new(None),
            downloaded_only: Cell::new(false),
            library_find: RefCell::default(),
            thread: RefCell::new(None),
            thread_control: RefCell::new(None),
            cache_work: Cell::new(CacheWork::Idle),
            transfer: RefCell::new(None),
            storage_report: RefCell::default(),
            storage_request: Cell::new(0),
            pruned_after_feed: Cell::new(false),
            library_generation: Cell::new(0),
            thread_budget: Cell::new(INITIAL_THREAD_BUDGET),
            thread_sort: Cell::new(ThreadSort::Ranked),
            visit_seen: RefCell::default(),
            resume_viewport: Cell::new(None),
            find_revision: Cell::new(0),
            applied_find_query: RefCell::new(String::new()),
            find_match_id: Cell::new(None),
            find_origin: Cell::new(None),
            new_replies_origin: Cell::new(None),
            focus_reading: Cell::new(false),
            layout_mode: Cell::new(None),
            layout_update_queued: Cell::new(false),
            back_handler: RefCell::new(None),
        }
    }
    pub fn new() -> Ref<Self> {
        let this = instantiate(Self::construct());
        this.initialize_component();
        this.populate_navigation();
        this.wire();
        this.configure_detail();
        ACTIVE.with(|v| *v.borrow_mut() = Some(this.downgrade()));
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
        self.wire_library_find();
        AutomationProperties::set_live_setting(
            &self.label("StatusText"),
            AutomationLiveSetting::Polite,
        );
        AutomationProperties::set_live_setting(
            &self.label("FindCount"),
            AutomationLiveSetting::Polite,
        );
        for (name, icon) in [
            ("SearchButton", "search"),
            ("CompactSearch", "search"),
            ("EditFeedSearch", "search"),
            ("LibraryFindButton", "search"),
            ("CloseLibraryFind", "dismiss"),
            ("ToolbarMore", "more"),
            ("FindButton", "search"),
            ("FocusButton", "focus_reading"),
            ("CloseFind", "dismiss"),
            ("FindPrevious", "chevron_up"),
            ("FindNext", "chevron_down"),
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
        self.on("WelcomeOpen", |v| {
            if let Some(id) = v
                .button("WelcomeOpen")
                .tag()
                .and_then(|v| v.downcast_ref::<u64>().copied())
            {
                v.open_item(id);
            }
        });
        self.on("WelcomeSearch", |v| v.show_search());
        self.on("WelcomeSaved", |v| v.select_feed(FeedRoute::Saved));
        self.on("NavSaved", |v| v.select_feed(FeedRoute::Saved));
        self.on("MobileSaved", |v| v.select_feed(FeedRoute::Saved));
        self.on("NavHistory", |v| v.select_feed(FeedRoute::History));
        self.on("NavHidden", |v| v.select_feed(FeedRoute::Hidden));
        self.on("NavMore", |v| v.show_more());
        self.on("CompactSearch", |v| v.show_search());
        self.on("EditFeedSearch", |v| v.show_search());
        self.on("ThemeButton", |v| v.show_settings());
        self.on("MobileSearch", |v| v.show_search());
        self.on("MobileMore", |v| v.show_more());
        self.on("SearchButton", |v| v.submit_quick_search());
        self.on("RefreshButton", |v| v.refresh());
        self.wire_phone_toolbar();
        self.on("FindButton", |v| {
            v.set_find_open(!v.get_control::<Border>("ThreadTools").is_visible())
        });
        self.on("FocusButton", |v| {
            v.set_focus_reading(!v.focus_reading.get())
        });
        self.on("FindNext", |v| v.step_find(1));
        self.on("FindPrevious", |v| v.step_find(-1));
        self.on("CloseFind", |v| v.set_find_open(false));
        self.on("RetryButton", |v| match v.empty_action.get() {
            Some(EmptyAction::Retry) => v.load_feed(0, FetchMode::Refresh),
            Some(EmptyAction::FrontPage) => v.select_feed(FeedRoute::Feed(FeedKind::Top)),
            Some(EmptyAction::EditSearch) => v.show_search(),
            Some(EmptyAction::ClearLibraryFind) => v.set_library_find_open(false),
            Some(EmptyAction::ShowRead) => v.set_unread_only(false),
            Some(EmptyAction::AllSaved) => v.set_downloaded_only(false),
            Some(EmptyAction::Settings) => v.show_settings(),
            None => {}
        });
        self.on("DownloadedOnly", |v| {
            v.set_downloaded_only(
                v.get_control::<ToggleButton>("DownloadedOnly").is_checked() == Some(true),
            );
        });
        self.on("RetryFeedItems", Self::retry_feed_items);
        self.on("LoadMoreButton", |v| {
            if let Some(offset) = v.next.get() {
                // A deliberate unread search can cross fully read or muted pages.
                // Bound each tap so a heavily filtered feed cannot drain the entire API.
                let pages =
                    if !v.feed.borrow().is_library() && v.state.borrow().preferences.hide_read {
                        3
                    } else {
                        1
                    };
                v.load_feed_pages(offset, FetchMode::CacheFirst, pages);
            }
        });
        self.on("BackButton", |v| v.go_back());
        self.on("SaveButton", |v| {
            if let Some(id) = v.selected.get() {
                v.toggle_story_flag(id, StoryFlag::Saved);
            }
        });
        self.on("DownloadButton", |v| {
            // A verified copy has a native action menu. Unverified discussions download directly.
            if v.button("DownloadButton").flyout().is_none() {
                v.download();
            }
        });
        self.on("CancelTransfer", Self::cancel_transfer);
        self.on("UndoStory", |v| v.undo_story_edit());
        self.on("DismissStatus", |v| {
            v.undo_story.set(None);
            v.get_control::<Border>("StatusBar").set_is_visible(false)
        });
        self.on("UnreadButton", |v| {
            v.set_unread_only(
                v.get_control::<ToggleButton>("UnreadButton").is_checked() == Some(true),
            );
        });
        let weak = self.to_ref().downgrade();
        self.size_changed(move |_, _| {
            let Some(view) = weak.upgrade() else {
                return;
            };
            if view.layout_update_queued.replace(true) {
                return;
            }
            // Bounds change during arrange. Let that pass finish before changing columns
            // or hiding controls it has already queued, and use the latest viewport width.
            let weak = view.downgrade();
            Dispatcher::ui_thread().post_local(
                move || {
                    if let Some(view) = weak.upgrade() {
                        view.layout_update_queued.set(false);
                        if view.is_loaded() {
                            view.apply_layout();
                        }
                    }
                },
                DispatcherPriority::LOADED,
            );
        });
        self.wire_quick_search();
        let weak = self.to_ref().downgrade();
        let find_field = self.textbox("ThreadFind");
        let find_help = "Enter for the next match, Shift+Enter for the previous match, Ctrl or Command+Enter to read the match in context.";
        AutomationProperties::set_help_text(&find_field, Some(find_help));
        ToolTip::set_tip(&find_field, Some(Rc::new(find_help.to_owned())));
        find_field.add_handler(
            InputElement::key_down_event(),
            move |_, e: &KeyEventArgs| {
                if e.key != Key::Enter {
                    return;
                }
                let Some(view) = weak
                    .upgrade()
                    .filter(|v| !v.textbox("ThreadFind").is_composing_text())
                else {
                    return;
                };
                match e.key_modifiers {
                    KeyModifiers::NONE => view.step_find(1),
                    KeyModifiers::SHIFT => view.step_find(-1),
                    KeyModifiers::CONTROL | KeyModifiers::META => {
                        // Apply the field value before resolving the ID, even when
                        // Enter arrives before the debounced search has run.
                        view.step_find(0);
                        if let Some(id) = view
                            .find_match_id
                            .get()
                            .filter(|_| !view.find_query().is_empty())
                        {
                            view.read_in_context(id);
                        }
                    }
                    _ => return,
                }
                e.set_handled(true);
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
                                if view.applied_find_query.borrow().as_str() != view.find_query() {
                                    view.render_thread();
                                }
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
                let ctrl = matches!(e.key_modifiers, KeyModifiers::CONTROL | KeyModifiers::META);
                let source = e.source().and_then(|s| s.cast::<ferroui_base::Visual>());
                if source.as_ref().is_some_and(|source| {
                    std::iter::once(source.clone())
                        .chain(source.get_visual_ancestors())
                        .filter_map(|v| v.cast::<TextBox>())
                        .any(|field| field.is_composing_text())
                }) {
                    return;
                }
                if ctrl && e.key == Key::Z && v.undo_story.get().is_some() {
                    let editor = source.as_ref().is_some_and(|source| {
                        std::iter::once(source.clone())
                            .chain(source.get_visual_ancestors())
                            .any(|node| node.cast::<TextBox>().is_some())
                    });
                    if !editor {
                        v.undo_story_edit();
                        e.set_handled(true);
                    }
                    return;
                }
                if ctrl && e.key == Key::K {
                    v.show_search();
                    e.set_handled(true);
                    return;
                }
                let in_library = v.feed.borrow().is_library()
                    && (*v.detail.borrow() == DetailRoute::Welcome
                        || source.as_ref().is_some_and(|s| {
                            v.get_control::<Border>("FeedPane").is_visual_ancestor_of(s)
                        }));
                if ctrl && e.key == Key::F {
                    if in_library {
                        v.set_library_find_open(true);
                    } else if matches!(*v.detail.borrow(), DetailRoute::Thread(_)) {
                        v.set_find_open(true);
                    } else {
                        return;
                    }
                    e.set_handled(true);
                    return;
                }
                if in_library
                    && e.key == Key::Escape
                    && e.key_modifiers == KeyModifiers::NONE
                    && v.get_control::<Grid>("LibraryFindBar").is_visible()
                {
                    v.set_library_find_open(false);
                    e.set_handled(true);
                    return;
                }
                if e.key == Key::Escape && e.key_modifiers == KeyModifiers::NONE {
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
                // A split view can keep a different discussion open while keyboard
                // focus moves down the feed. Resolve the routed row before the detail.
                let stories = v.get_control::<ListBox>("Stories");
                let feed_value = stories.get_container_from_event_source(source.as_ref())
                    .and_then(|container| stories.item_from_container(&container));
                let feed_row = feed_value
                    .as_ref()
                    .and_then(|value| value.downcast_ref::<HnItem>());
                match e.key {
                    Key::F if matches!(*v.detail.borrow(), DetailRoute::Thread(_)) => {
                        v.set_focus_reading(!v.focus_reading.get())
                    }
                    Key::R => v.refresh(),
                    Key::S => {
                        let id = feed_row.map(|item| item.id).or_else(|| {
                            if let DetailRoute::Thread(id) = *v.detail.borrow() {
                                Some(id)
                            } else {
                                None
                            }
                        });
                        let Some(id) = id else { return };
                        v.toggle_story_flag(id, StoryFlag::Saved);
                    }
                    Key::J | Key::K => {
                        v.move_selection(e.key == Key::J, feed_row.map(|item| item.id))
                    }
                    Key::U
                        if !v.feed.borrow().is_library()
                            && v.get_control::<Border>("FeedPane").is_effectively_visible() =>
                    {
                        let enabled = !v.state.borrow().preferences.hide_read;
                        v.set_unread_only(enabled);
                    }
                    Key::O => {
                        let Some(item) = feed_row.cloned().or_else(|| v.current_item()) else {
                            return;
                        };
                        let article =
                            item.visible_article_url(v.state.borrow().preferences.show_dead);
                        v.open_url(&article.unwrap_or_else(|| item.discussion_url()));
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
        let stories = self.get_control::<ListBox>("Stories");
        stories.set_item_template(Some(template));
        stories.set_items_source(Some(self.rendered_stories.clone().into()));
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
                if view.feed_load.get() == FeedLoad::Loading {
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
                if pull_distance.replace(0.0) >= 72.0 && view.feed_load.get() != FeedLoad::Loading {
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
                    } else if view
                        .get_control::<Grid>("LibraryFindBar")
                        .is_effectively_visible()
                    {
                        view.set_library_find_open(false);
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
                self.feed_load.set(FeedLoad::StorageUnavailable);
                self.render_stories();
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
                            crate::theme::apply(&state.preferences, None);
                            *view.state.borrow_mut() = state;
                            view.state_ready.set(true);
                            view.update_explore_counts();
                            view.update_profile_mute();
                            if let Some(restore) = view.pending_settings.borrow_mut().take() {
                                view.live_settings.replace(Some(restore(&view.state.borrow())));
                            }
                        }
                        Err(error) => view.status(&format!(
                            "Reading state could not be restored: {error}. Saving is paused to protect your existing data. Import a backup in Settings to recover it."
                        )),
                    }
                }
                if let Some(id) = services::take_initial_location()
                    .as_deref()
                    .and_then(item_from_location)
                {
                    // Give a permalink the network slots first. Single-pane readers
                    // need the feed only when they go Back or expand the window.
                    view.initial_feed_pending.set(true);
                    view.present_discussion(ThreadVisit::new(id, id, view.browser_entry.get()), false);
                } else {
                    view.load_feed(0, FetchMode::CacheFirst);
                }
            }
        }));
    }
    fn status(&self, message: &str) {
        self.undo_story.set(None);
        self.button("UndoStory").set_is_visible(false);
        self.label("StatusText").set_text(Some(message));
        self.get_control::<Border>("StatusBar").set_is_visible(true);
    }
    fn set_unread_only(&self, enabled: bool) {
        self.state.borrow_mut().preferences.hide_read = enabled;
        if let Some(settings) = self.live_settings.borrow().as_ref() {
            settings.set_hide_read(enabled);
        }
        self.changed();
        self.render_stories();
    }
    fn set_downloaded_only(&self, enabled: bool) {
        self.downloaded_only.set(enabled);
        self.next.set(None);
        self.button("LoadMoreButton").set_is_visible(false);
        // Re-page the chosen collection before fetching any rows.
        self.load_feed(0, FetchMode::CacheFirst);
        self.render_stories();
    }
    fn downloaded_shelf(&self) -> bool {
        self.downloaded_only.get() && *self.feed.borrow() == FeedRoute::Saved
    }
    fn empty(&self, title: &str, message: &str, action: Option<EmptyAction>) {
        self.label("EmptyTitle").set_text(Some(title));
        self.label("EmptyMessage").set_text(Some(message));
        self.empty_action.set(action);
        let button = self.button("RetryButton");
        button.set_is_visible(action.is_some());
        if let Some(action) = action {
            button.set_content(Some(Rc::new(action.label().to_owned())));
            AutomationProperties::set_name(&button, Some(action.label()));
        }
    }
    fn changed(&self) {
        if !self.state_ready.get() {
            return;
        }
        self.update_explore_counts();
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
        drop(Dispatcher::ui_thread().invoke_async_task_local(move || async move {
            loop {
                let Some(view) = weak.upgrade() else { return };
                let revision = view.revision.get();
                let save = repo.save_state(&view.state.borrow());
                drop(view);
                let result = save.await;
                let Some(view) = weak.upgrade() else { return };
                if let Err(error) = result {
                    view.saving.set(false);
                    view.status(&format!("Changes are not saved: {error}. Retry by changing the setting or bookmark again."));
                    return;
                }
                if revision == view.revision.get() {
                    view.saving.set(false);
                    return;
                }
            }
        }));
    }
    /// Starts evicting the oldest cached items beyond the limit; returns whether it started.
    fn prune_cache(&self) -> bool {
        if !self.state_ready.get() || self.cache_work.get() != CacheWork::Idle {
            return false;
        }
        let Some(repo) = self.repo() else {
            return false;
        };
        self.set_cache_work(CacheWork::Pruning);
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let Some(view) = weak.upgrade() else { return };
                let prune = repo.prune_cache(&view.state.borrow());
                drop(view);
                let result = prune.await;
                if let Some(v) = weak.upgrade() {
                    v.set_cache_work(CacheWork::Idle);
                    if let Err(error) = result {
                        v.status(&format!("Cache cleanup could not finish: {error}"));
                    }
                }
            }),
        );
        true
    }
    fn select_feed(&self, route: FeedRoute) {
        let current_collection =
            route == *self.feed.borrow() && !matches!(route, FeedRoute::Search(_));
        if current_collection && *self.detail.borrow() != DetailRoute::Welcome {
            self.back_to_feed(true);
            return;
        }
        let scroll = self.story_scroll();
        if !current_collection {
            if let FeedRoute::Feed(kind) = *self.feed.borrow() {
                if let Some(summary) = self.accepted_feed_summary.take() {
                    self.feed_windows.borrow_mut().insert(
                        kind,
                        FeedWindow {
                            stories: self.stories.take(),
                            next: self.next.get(),
                            summary,
                            position: scroll.as_ref().map(|s| s.offset()).unwrap_or_default(),
                        },
                    );
                }
            }
        }
        if let Some(scroll) = scroll {
            scroll.set_offset(ferroui_base::Vector::default());
        }
        if current_collection && !self.stories.borrow().is_empty() {
            return;
        }
        self.reset_library_find();
        self.search_story_ids.borrow_mut().clear();
        self.accepted_feed_summary.take();
        self.feed.replace(route.clone());
        if let FeedRoute::Feed(kind) = route {
            let window = self.feed_windows.borrow_mut().remove(&kind);
            if let Some(window) = window {
                // Responses for the outgoing tab, including pagination, must not
                // replace the resumed window. Load more continues at its own cursor.
                self.feed_generation
                    .set(self.feed_generation.get().wrapping_add(1));
                self.feed_load.set(FeedLoad::Ready);
                self.stories.replace(window.stories);
                self.next.set(window.next);
                self.button("LoadMoreButton").set_is_enabled(true);
                self.button("LoadMoreButton")
                    .set_is_visible(window.next.is_some());
                self.label("FeedTitle").set_text(Some(kind.title()));
                self.label("FeedSummary").set_text(Some(&window.summary));
                self.accepted_feed_summary.replace(Some(window.summary));
                self.back_to_feed(true);
                self.restore_feed_scroll(window.position);
                return;
            }
        }
        self.stories.borrow_mut().clear();
        self.next.set(None);
        self.back_to_feed(true);
        self.load_feed(0, FetchMode::CacheFirst);
    }

    fn story_scroll(&self) -> Option<Ref<ScrollViewer>> {
        self.get_control::<ListBox>("Stories")
            .scroll()?
            .cast::<ScrollViewer>()
    }

    fn restore_feed_scroll(&self, position: ferroui_base::Vector) {
        let weak = self.to_ref().downgrade();
        let generation = self.feed_generation.get();
        Dispatcher::ui_thread().post_local(
            move || {
                if let Some(scroll) = weak
                    .upgrade()
                    .filter(|v| v.feed_generation.get() == generation)
                    .and_then(|v| v.story_scroll())
                {
                    scroll.set_offset(position);
                }
            },
            DispatcherPriority::LOADED,
        );
    }

    fn forget_feed_windows(&self) {
        self.feed_windows.borrow_mut().clear();
        self.accepted_feed_summary.take();
    }

    fn refresh(&self) {
        if let DetailRoute::Thread(id) = self.detail.borrow().clone() {
            self.fetch_thread(id, FetchMode::Refresh);
            if !self.get_control::<Border>("FeedPane").is_effectively_visible() {
                return;
            }
        }
        self.load_feed(0, FetchMode::Refresh);
    }
    fn load_feed(&self, offset: usize, mode: FetchMode) {
        self.load_feed_pages(offset, mode, 1);
    }
    fn load_feed_pages(&self, offset: usize, mode: FetchMode, pages_left: usize) {
        self.initial_feed_pending.set(false);
        let Some(repo) = self.repo() else {
            return;
        };
        let generation = self.feed_generation.get().wrapping_add(1);
        self.feed_generation.set(generation);
        self.feed_load.set(FeedLoad::Loading);
        self.button("LoadMoreButton").set_is_enabled(false);
        if self.stories.borrow().is_empty() {
            self.empty(
                "Loading stories…",
                "Checking for stories on this device.",
                None,
            );
        }
        let route = self.feed.borrow().clone();
        let title = match &route {
            FeedRoute::Feed(f) => f.title(),
            FeedRoute::Saved if self.downloaded_only.get() => "Downloads",
            FeedRoute::Saved => "Saved for later",
            FeedRoute::History => "Reading history",
            FeedRoute::Hidden => "Hidden stories",
            FeedRoute::Search(_) => "Search results",
        };
        self.label("FeedTitle").set_text(Some(title));
        self.label("FeedSummary").set_text(Some(
            if offset > 0 && !route.is_library() && self.state.borrow().preferences.hide_read {
                "Looking for unread stories…"
            } else {
                "Updating…"
            },
        ));
        // Refresh a local collection without discarding already loaded pages.
        let local_page_size = if offset == 0 {
            self.stories.borrow().len().max(30)
        } else {
            30
        };
        let downloaded_shelf = self.downloaded_shelf();
        let local_ids = match &route {
            FeedRoute::Saved if downloaded_shelf => self.state.borrow().downloaded_ids(),
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
        let needs_preview = self.stories.borrow().is_empty();
        self.button("RetryFeedItems").set_is_enabled(false);
        if route.is_library() && !self.library_find.borrow().query.is_empty() {
            self.scan_library(repo, local_ids, offset, local_page_size, generation);
            return;
        }
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                if needs_preview && offset == 0 && mode != FetchMode::Offline {
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
                                    &[],
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
                                page.failed_slots,
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
                                    Vec::new(),
                                    page.story_ids,
                                )
                            })
                    }
                    FeedRoute::Saved | FeedRoute::History | FeedRoute::Hidden => {
                        let ids = local_ids;
                        let end = offset.saturating_add(local_page_size).min(ids.len());
                        let batch = repo
                            .load_items(
                                ids.get(offset..end).unwrap_or_default(),
                                services::now(),
                                if downloaded_shelf {
                                    FetchMode::Offline
                                } else {
                                    mode
                                },
                            )
                            .await;
                        Ok((
                            batch.items,
                            (end < ids.len()).then_some(end),
                            ids.len(),
                            batch.source,
                            batch.warning,
                            batch.failed_slots,
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
                view.feed_load.set(FeedLoad::Ready);
                view.button("LoadMoreButton").set_is_enabled(true);
                match result {
                    Ok((items, next, total, source, warning, failed_slots, story_ids)) => {
                        // A stale first-page fallback must not truncate pages the reader already has open.
                        if offset == 0 && mode == FetchMode::Refresh && !needs_preview
                            && matches!(*view.feed.borrow(), FeedRoute::Feed(_))
                            && source == DataSource::StaleCache && warning.is_some()
                            && failed_slots.is_empty()
                            && view.stories.borrow().len() > items.len()
                            && items.iter().zip(view.stories.borrow().iter()).all(|(item, old)| item == old.as_ref())
                        {
                            let summary = format!("{total} stories · Offline copy");
                            view.label("FeedSummary").set_text(Some(&summary));
                            view.accepted_feed_summary.replace(Some(summary));
                            view.status(warning.as_deref().unwrap_or_default());
                            view.render_stories();
                            return;
                        }
                        if offset == 0 {
                            if items.is_empty() && total > 0 && warning.is_some() {
                                view.feed_load.set(FeedLoad::Unavailable);
                            }
                            view.search_story_ids.borrow_mut().clear();
                        }
                        view.search_story_ids.borrow_mut().extend(story_ids);
                        let previous_len = view.stories.borrow().len();
                        let continue_at = next.filter(|&cursor| {
                            pages_left > 1 && cursor > offset && warning.is_none()
                        });
                        view.accept_items(items, offset, next, total, source, warning, &failed_slots);
                        if let Some(cursor) = continue_at {
                            let no_new_unread = {
                                let state = view.state.borrow();
                                state.preferences.hide_read
                                    && !view.stories.borrow()[previous_len..]
                                        .iter()
                                        .any(|item| state.is_visible(item))
                            };
                            if no_new_unread && view.feed_generation.get() == generation {
                                view.load_feed_pages(cursor, mode, pages_left - 1);
                            }
                        }
                    }
                    Err(error) => {
                        let (state, message) = if offset == 0 {
                            (FeedLoad::Unavailable, "Update unavailable")
                        } else {
                            (FeedLoad::PageUnavailable, "Couldn’t load the next page")
                        };
                        view.label("FeedSummary").set_text(Some(message));
                        view.feed_load.set(state);
                        view.button("LoadMoreButton")
                            .set_is_visible(view.next.get().is_some());
                        view.status(&format!("{message}: {error}"));
                        view.render_stories();
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
        failed_slots: &[(usize, u64)],
    ) {
        let append_at = (offset != 0).then(|| self.stories.borrow().len());
        self.stories.borrow_mut().accept_page(items, failed_slots, offset == 0);
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
        let summary = match &*route {
            FeedRoute::Search(SearchQuery {
                story_id: Some(id), ..
            }) => format!("Discussion {id} · {count} · {origin}"),
            _ => format!("{count} · {origin}"),
        };
        self.label("FeedSummary").set_text(Some(&summary));
        if matches!(*route, FeedRoute::Feed(_)) {
            // Keep the last accepted summary while a subsequent page is still in flight.
            self.accepted_feed_summary
                .replace((!self.stories.borrow().is_empty() || !failed_slots.is_empty()).then_some(summary));
        }
        drop(route);
        if let Some(warning) = warning {
            self.status(&warning);
        }
        self.project_stories(append_at);
        // Pruning reads every cached item, so it runs once a session after a feed has loaded,
        // and again after a download or a change of the cache limit, not after every refresh.
        if self.feed_load.get() != FeedLoad::Loading
            && !self.pruned_after_feed.get()
            && self.prune_cache()
        {
            self.pruned_after_feed.set(true);
        }
    }
    fn render_stories(&self) {
        self.project_stories(None);
    }

    fn project_stories(&self, append_at: Option<usize>) {
        let state = self.state.borrow();
        let loaded = self.stories.borrow();
        let route = self.feed.borrow();
        let filterable = !route.is_library();
        let downloaded_only = self.downloaded_only.get();
        let revision = self.revision.get();
        // A state edit while a page was loading may change membership or unread counts.
        let previous = self.feed_summary.take();
        let start = append_at.filter(|&n| previous.loaded == n && previous.revision == revision);
        let mut summary = if start.is_some() {
            previous
        } else {
            FeedSummaryState::default()
        };
        let history = *route == FeedRoute::History;
        let mut seek_unread = !history
            && summary
                .suggested
                .as_ref()
                .is_none_or(|s| state.is_read(s.id));
        let mut visible = Vec::new();
        for item in &loaded[start.unwrap_or(0)..] {
            if !route.includes(item, &state, downloaded_only) {
                continue;
            }
            visible.push(item);
            let unread = !state.is_read(item.id);
            summary.unread += usize::from(unread);
            // Library routes can contain muted or deliberately hidden entries.
            if (summary.suggested.is_none() || (seek_unread && unread))
                && (filterable || state.passes_content_filters(item))
            {
                summary.suggested = Some(item.clone());
                seek_unread = !history && !unread;
            }
        }
        summary.loaded = loaded.len();
        summary.revision = revision;
        let unread_count = summary.unread;
        if *self.detail.borrow() == DetailRoute::Welcome {
            self.update_welcome(summary.suggested.as_deref(), &state);
        }
        self.feed_summary.replace(summary);
        let list = self.get_control::<ListBox>("Stories");
        let visibility_changed = self.rendered_show_dead.replace(state.preferences.show_dead)
            != state.preferences.show_dead;
        if start.is_some() {
            self.rendered_stories.add_range(
                visible
                    .into_iter()
                    .map(|item| Some(item.clone() as BoxedValue)),
            );
        } else {
            self.sync_story_items(&list, &visible);
            self.refresh_story_cards(None, visibility_changed);
        }
        let missing = loaded.missing_count();
        let retry = self.button("RetryFeedItems");
        retry.set_is_visible(missing != 0);
        if missing != 0 {
            let label = if missing == 1 {
                "Retry 1 unavailable story".to_owned()
            } else {
                format!("Retry {missing} unavailable stories")
            };
            retry.set_content(Some(Rc::new(label.clone())));
            AutomationProperties::set_name(&retry, Some(&label));
            retry.set_is_enabled(self.feed_load.get() != FeedLoad::Loading);
        }
        let empty = list.items().count() == 0;
        self.get_control::<StackPanel>("FeedEmpty")
            .set_is_visible(empty);
        if empty {
            use EmptyAction::*;
            let (title, message, action) = match self.feed_load.get() {
                FeedLoad::Loading => (
                    "Loading stories…",
                    "Checking for stories on this device.",
                    None,
                ),
                FeedLoad::StorageUnavailable => (
                    "Storage could not be opened",
                    "Check app storage and restart Ember.",
                    None,
                ),
                FeedLoad::Unavailable if self.downloaded_shelf() => (
                    "Offline copy unavailable",
                    "Open the saved story while connected to download its discussion again.",
                    Some(AllSaved),
                ),
                FeedLoad::PageUnavailable => (
                    "Couldn’t load the next page",
                    "Your place is saved. Retry this page below.",
                    None,
                ),
                _ if missing != 0 => (
                    "Stories unavailable",
                    "Retry the unavailable stories below.",
                    None,
                ),
                FeedLoad::Unavailable => (
                    "Couldn’t load stories",
                    "Check your connection and try again.",
                    Some(Retry),
                ),
                FeedLoad::Ready if !self.library_find.borrow().query.is_empty() => (
                    "No matches on this device",
                    "Try another title, author, or domain. Search uses locally available stories and comments.",
                    Some(ClearLibraryFind),
                ),
                FeedLoad::Ready => match &*route {
                    FeedRoute::Saved if self.downloaded_only.get() => (
                        "No downloaded discussions",
                        "Open a discussion and choose Download to keep a copy on this device.",
                        Some(AllSaved),
                    ),
                    FeedRoute::Saved => (
                        "No saved stories",
                        "Use the bookmark on a discussion to keep it here.",
                        Some(FrontPage),
                    ),
                    FeedRoute::History => (
                        "No reading history",
                        "Stories appear here when you open a discussion.",
                        Some(FrontPage),
                    ),
                    FeedRoute::Hidden => (
                        "No hidden stories",
                        "Stories you hide can be restored here.",
                        Some(FrontPage),
                    ),
                    _ if !self.stories.borrow().is_empty()
                        && state.preferences.hide_read
                        && self.stories.borrow().iter().any(|item| {
                            state.is_read(item.id) && state.passes_content_filters(item)
                        }) =>
                    {
                        (
                            "No unread stories here",
                            if self.next.get().is_some() {
                                "All visible stories on these pages are read. Find more unread stories below."
                            } else {
                                "You’ve read all the stories that match your filters."
                            },
                            Some(ShowRead),
                        )
                    }
                    _ if !self.stories.borrow().is_empty() => (
                        "No visible stories",
                        "Review your filters and mute rules in reading settings.",
                        Some(Settings),
                    ),
                    FeedRoute::Search(_) => (
                        "No results",
                        "Try another term or broaden your filters.",
                        Some(EditSearch),
                    ),
                    _ => (
                        "No stories yet",
                        "Refresh to check for new stories.",
                        Some(Retry),
                    ),
                },
            };
            self.empty(title, message, action);
        }
        self.button("LibraryFindButton")
            .set_is_visible(route.is_library());
        self.button("EditFeedSearch")
            .set_is_visible(matches!(*route, FeedRoute::Search(_)));
        let downloads = self.get_control::<ToggleButton>("DownloadedOnly");
        downloads.set_is_checked(Some(self.downloaded_only.get()));
        downloads.set_is_visible(*route == FeedRoute::Saved);
        downloads
            .classes()
            .set("active", self.downloaded_only.get());
        let filter_label = if self.downloaded_only.get() {
            "Show all saved stories"
        } else {
            "Show downloaded discussions only"
        };
        AutomationProperties::set_name(&downloads, Some(filter_label));
        ToolTip::set_tip(
            &downloads,
            Some(Rc::new(filter_label.to_owned()) as BoxedValue),
        );
        let unread = self.get_control::<ToggleButton>("UnreadButton");
        unread.set_is_checked(Some(state.preferences.hide_read));
        unread.set_is_visible(filterable);
        unread.classes().set("active", state.preferences.hide_read);
        if filterable {
            let count = format!("Unread · {unread_count}");
            if !unread
                .content()
                .is_some_and(|old| old.downcast_ref::<String>() == Some(&count))
            {
                unread.set_content(Some(Rc::new(count)));
            }
            let noun = if unread_count == 1 {
                "story"
            } else {
                "stories"
            };
            AutomationProperties::set_help_text(
                &unread,
                Some(&format!("{unread_count} unread {noun} on the loaded pages")),
            );
        }
        let more = self.button("LoadMoreButton");
        let action = if self.feed_load.get() == FeedLoad::PageUnavailable {
            "Retry page"
        } else if filterable && state.preferences.hide_read {
            "Find more unread"
        } else {
            "Load more"
        };
        if !more
            .content()
            .is_some_and(|old| old.downcast_ref::<String>().map(String::as_str) == Some(action))
        {
            more.set_content(Some(Rc::new(action.to_owned())));
        }
        AutomationProperties::set_name(&more, Some(action));
        let label = if state.preferences.hide_read {
            "Show all stories"
        } else {
            "Show unread only"
        };
        AutomationProperties::set_name(&unread, Some(label));
        ToolTip::set_tip(&unread, Some(Rc::new(format!("{label} (U)")) as BoxedValue));
    }
    fn sync_story_items(&self, list: &ListBox, stories: &[&Rc<HnItem>]) {
        // Rows are immutable. Preserve the common prefix and suffix; an append
        // or filter edit then changes just one notified range in the bound list.
        let (prefix, removed, added, tail_append) = {
            let previous = self.rendered_stories.snapshot();
            let same = |(old, new): (&Option<BoxedValue>, &&Rc<HnItem>)| {
                old.as_ref()
                    .and_then(|value| value.downcast_ref::<HnItem>())
                    .is_some_and(|item| std::ptr::eq(item, new.as_ref()))
            };
            let prefix = previous
                .iter()
                .zip(stories)
                .take_while(|&pair| same(pair))
                .count();
            let suffix = previous[prefix..]
                .iter()
                .rev()
                .zip(stories[prefix..].iter().rev())
                .take_while(|&pair| same(pair))
                .count();
            (
                prefix,
                previous.len() - prefix - suffix,
                stories.len() - prefix - suffix,
                prefix == previous.len(),
            )
        };
        if removed != 0 || added != 0 {
            let scroll = list.scroll().and_then(|s| s.cast::<ScrollViewer>());
            let offset = scroll.as_ref().map(|s| s.offset()).unwrap_or_default();
            let focused = list.is_keyboard_focus_within();
            self.rendered_stories.remove_range(prefix, removed);
            self.rendered_stories.insert_range(
                prefix,
                stories[prefix..prefix + added]
                    .iter()
                    .map(|item| Some(Rc::clone(item) as BoxedValue)),
            );
            if !tail_append {
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
                                let list = view.get_control::<ListBox>("Stories");
                                if focused && !list.is_keyboard_focus_within() {
                                    list.focus();
                                }
                            }
                        },
                        DispatcherPriority::LOADED,
                    );
                }
            }
        }
    }

    fn update_welcome(&self, candidate: Option<&HnItem>, state: &ReaderState) {
        let feed = self.feed.borrow();
        self.button("WelcomeSaved")
            .set_is_visible(*feed != FeedRoute::Saved);
        self.get_control::<Border>("WelcomeStory")
            .set_is_visible(candidate.is_some());
        let open = self.button("WelcomeOpen");
        open.set_tag(candidate.map(|story| Rc::new(story.id) as BoxedValue));
        let Some(item) = candidate else {
            return;
        };
        let caption = if *feed == FeedRoute::History {
            "LAST OPENED"
        } else if state.is_read(item.id) {
            "FROM THIS COLLECTION"
        } else {
            "NEXT UNREAD"
        };
        let summary = StorySummary::new(item, services::now(), state.preferences.show_dead);
        self.label("WelcomeStoryLabel").set_text(Some(caption));
        self.label("WelcomeHeadline")
            .set_text(Some(&summary.headline));
        let source = [summary.source, summary.byline]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        self.label("WelcomeStorySource").set_text(Some(&source));
        AutomationProperties::set_help_text(&open, Some(&summary.headline));
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
        self.attach_story_menu(&card, item);
        card
    }
    fn refresh_story_cards(&self, only: Option<u64>, update_summary: bool) {
        let list = self.get_control::<ListBox>("Stories");
        let state = self.state.borrow();
        for container in list.get_realized_containers() {
            let Some(value) = list.item_from_container(&container) else {
                continue;
            };
            let Some(item) = value.downcast_ref::<HnItem>() else {
                continue;
            };
            if only.is_some_and(|id| id != item.id) {
                continue;
            }
            if let Some(card) = container
                .get_visual_descendants()
                .find_map(|v| v.cast::<StoryCard>())
            {
                if update_summary {
                    card.set_item(item, services::now(), state.preferences.show_dead);
                }
                self.update_card_state(&card, item.id, &state);
            }
        }
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
        card.classes().set(
            "hidden",
            state.items.get(&id).is_some_and(|entry| entry.hidden),
        );
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
    fn feed_article_url(&self, id: u64) -> Option<String> {
        let stories = self.stories.borrow();
        stories[stories.position(id)?]
            .visible_article_url(self.state.borrow().preferences.show_dead)
    }
    fn attach_story_menu(&self, card: &StoryCard, item: &HnItem) {
        let menu = ContextMenu::new();
        let owner = self.to_ref().downgrade();
        let weak_menu = menu.downgrade();
        let id = item.id;
        // Resolve state on opening. Scrolling a feed never constructs a menu's command tree.
        menu.opening(move |_| {
            let (Some(view), Some(menu)) = (owner.upgrade(), weak_menu.upgrade()) else {
                return;
            };
            menu.items().clear();
            let add = |label: &str, icon: &str, command: Box<dyn Fn(&ReaderView)>| {
                let entry = MenuItem::new();
                entry.set_header(Some(Rc::new(label.to_owned())));
                entry.set_icon(Some(Control::boxed(crate::icons::icon(icon))));
                let owner = view.downgrade();
                entry.click(move |_, _| {
                    if let Some(view) = owner.upgrade() {
                        command(&view);
                    }
                });
                menu.items().add(Some(Control::boxed(entry)));
            };
            add(
                "Open discussion",
                "chat",
                Box::new(move |v| v.open_item(id)),
            );
            if view.feed_article_url(id).is_some() {
                add(
                    "Open article",
                    "open",
                    Box::new(move |v| {
                        v.open_url(
                            &v.feed_article_url(id)
                                .unwrap_or_else(|| ember_core::item_url(id)),
                        )
                    }),
                );
            }
            let state = view.state.borrow();
            let hidden = state.items.get(&id).is_some_and(|item| item.hidden);
            for (label, icon, flag) in [
                (
                    if state.is_saved(id) {
                        "Remove from saved"
                    } else {
                        "Save for later"
                    },
                    "bookmark",
                    StoryFlag::Saved,
                ),
                (
                    if state.is_read(id) {
                        "Mark unread"
                    } else {
                        "Mark read"
                    },
                    "check",
                    StoryFlag::Read,
                ),
                (
                    if hidden {
                        "Restore to feeds"
                    } else {
                        "Hide story"
                    },
                    "eye_off",
                    StoryFlag::Hidden,
                ),
            ] {
                add(
                    label,
                    icon,
                    Box::new(move |v| v.toggle_story_flag(id, flag)),
                );
            }
            if state.downloaded_threads.contains_key(&id) {
                add(
                    "Stop keeping offline",
                    "download",
                    Box::new(move |v| v.release_download(id)),
                );
            } else if view.cache_work.get() == CacheWork::Idle {
                add(
                    "Download for offline reading",
                    "download",
                    Box::new(move |v| v.start_download(id)),
                );
            }
            add(
                "Copy HN link",
                "copy",
                Box::new(move |v| {
                    v.copy_text(&ember_core::item_url(id), "Hacker News link copied.")
                }),
            );
        });
        card.set_context_menu(menu);
    }
    fn wire_card_swipe(&self, card: &StoryCard, id: u64) {
        let swipe = SwipeGestureRecognizer::new();
        swipe.set_can_horizontally_swipe(true);
        swipe.set_can_vertically_swipe(false);
        card.gesture_recognizers()
            .add(swipe.upcast::<ferroui_base::input::gesture_recognizers::GestureRecognizer>());
        let swiped = card.to_ref().downgrade();
        card.add_handler(InputElement::swipe_gesture_event(), move |_, e| {
            if let Some(card) = swiped.upgrade() {
                // FerroUI reports content displacement, opposite to the finger.
                card.track_swipe(-e.delta().x);
            }
        });
        let weak = self.to_ref().downgrade();
        let swiped = card.to_ref().downgrade();
        card.add_handler(InputElement::swipe_gesture_ended_event(), move |_, _| {
            if let (Some(v), Some(card)) = (weak.upgrade(), swiped.upgrade()) {
                let delta = card.finish_swipe();
                if delta < -StoryCard::SWIPE_THRESHOLD {
                    v.toggle_story_flag(id, StoryFlag::Saved);
                } else if delta > StoryCard::SWIPE_THRESHOLD {
                    v.toggle_story_flag(id, StoryFlag::Hidden);
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
            .filter(|v| Some(v.snapshot.root.id) == self.selected.get())
            .map(|v| v.snapshot.root.clone())
    }
    fn open_item(&self, row: u64) {
        // A split-view row can be invoked again while its discussion is open.
        // Keep its loaded pages, reading position and any active Find session.
        if self.selected.get() == Some(row)
            && self.selected_row.get() == Some(row)
            && matches!(*self.detail.borrow(), DetailRoute::Thread(_))
            && self.thread.borrow().is_some()
        {
            if self.get_control::<Border>("ThreadTools").is_visible() {
                self.textbox("ThreadFind").focus();
            } else {
                self.button("BackButton").focus();
            }
            return;
        }
        // A scoped hit can be far beyond the locally loaded part of its thread.
        if matches!(&*self.feed.borrow(), FeedRoute::Search(query) if query.story_id.is_some()) {
            self.open_item_internal(row, true);
            return;
        }
        let id = self
            .search_story_ids
            .borrow()
            .get(&row)
            .copied()
            .unwrap_or(row);
        self.open_discussion(row, id, true);
    }
    fn open_item_internal(&self, id: u64, publish: bool) {
        self.open_discussion(id, id, publish);
    }
    fn open_discussion(&self, row: u64, id: u64, publish: bool) {
        self.present_discussion(ThreadVisit::new(id, row, None), publish);
    }
    fn present_discussion(&self, visit: ThreadVisit, publish: bool) {
        let id = visit.root;
        // Capture the outgoing visit before replacing its cursor or load window.
        self.navigate(DetailRoute::Thread(id));
        self.browser_entry.set(visit.browser_entry);
        self.selected_row.set(Some(visit.source_row));
        self.pending_comment.set(visit.pending_comment);
        self.selected.set(Some(id));
        self.thread_budget.set(visit.budget);
        self.label("DetailEyebrow").set_text(Some("DISCUSSION"));
        self.thread.borrow_mut().take();
        self.thread_control.borrow_mut().take();
        let loading = widgets::stack(16.0);
        loading.set_margin(Thickness::uniform(28.0));
        loading
            .children()
            .add(widgets::heading("Opening the conversation"));
        loading.children().add(widgets::text(
            if self.thread_fetch_mode(id) == FetchMode::Offline {
                "Opening the copy on this device…"
            } else {
                "Checking for a saved copy…"
            },
            15.0,
        ));
        self.show_content(loading.upcast());
        self.update_thread_commands();
        self.label("DetailTitle")
            .set_text(Some("Opening the conversation…"));
        let field = self.textbox("ThreadFind");
        if let Some(presentation) = visit.presentation {
            self.visit_seen.replace(presentation.seen.clone());
            self.resume_viewport.set(presentation.position);
            self.thread_sort.set(presentation.sort);
            self.new_replies_origin.set(presentation.new_replies_origin);
            if self.focus_reading.replace(presentation.focus_reading) != presentation.focus_reading
            {
                self.apply_layout();
            }
            if let Some(find) = &presentation.find {
                self.set_find_bar_visible(true);
                field.set_text(Some(&find.text));
                self.applied_find_query.replace(self.find_query());
                self.find_match_id.set(find.match_id);
                self.find_origin.set(find.origin);
                field.focus();
            } else {
                field.set_text(Some(""));
            }
        } else {
            field.set_text(Some(""));
            self.resume_viewport.set(None);
            self.visit_seen.replace(Rc::new(
                self.state
                    .borrow()
                    .items
                    .get(&id)
                    .map(|local| local.seen_comments.clone())
                    .unwrap_or_default(),
            ));
        }
        self.fetch_thread(id, self.thread_fetch_mode(id));
        self.refresh_story_cards(None, false);
        if publish {
            self.publish_location(Some(id));
        }
    }
    fn thread_fetch_mode(&self, id: u64) -> FetchMode {
        if self.downloaded_shelf() && self.state.borrow().downloaded_threads.contains_key(&id) {
            FetchMode::Offline
        } else {
            FetchMode::CacheFirst
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
                    let preview = repo
                        .load_thread(id, budget, services::now(), FetchMode::Offline)
                        .await;
                    let Some(view) = weak.upgrade().filter(|v| v.detail_generation.get() == generation)
                    else {
                        return;
                    };
                    if let Ok(mut cached) = preview {
                        // The cache-first traversal would read these same fresh records again.
                        // A budget-limited window may have pending children beyond its boundary.
                        let fresh_window = mode == FetchMode::CacheFirst
                            && !cached.stale
                            && cached.missing_ids.is_empty()
                            && cached.warning.is_none();
                        cached.warning = None;
                        view.accept_thread(cached);
                        if fresh_window || view.detail_generation.get() != generation {
                            return;
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
                            v.status(&format!("Discussion unavailable: {error}"));
                            if v.thread.borrow().is_none() {
                                v.label("DetailTitle")
                                    .set_text(Some("Discussion unavailable"));
                                let panel = widgets::stack(8.0);
                                panel.set_margin(Thickness::uniform(18.0));
                                panel
                                    .children()
                                    .add(widgets::heading("Couldn’t open this discussion"));
                                let offline = mode == FetchMode::Offline;
                                panel.children().add(widgets::muted(if offline {
                                    "The downloaded copy is missing from this device."
                                } else {
                                    "Check your connection and try loading the discussion again."
                                }));
                                let weak = v.downgrade();
                                let retry =
                                    widgets::action("Retry discussion", "refresh", move || {
                                        if let Some(view) = weak.upgrade() {
                                            view.fetch_thread(
                                                id,
                                                if offline {
                                                    FetchMode::Offline
                                                } else {
                                                    FetchMode::Refresh
                                                },
                                            );
                                        }
                                    });
                                retry.set_horizontal_alignment(
                                    ferroui_base::layout::HorizontalAlignment::Left,
                                );
                                panel.children().add(retry);
                                v.show_content(widgets::scroll(panel.upcast()));
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
        if let Some(warning) = thread.warning.as_deref() {
            self.status(warning);
        }
        // A fresh cache-first load can return the exact offline preview. Keep its
        // realized controls, Find index, scroll position and keyboard focus intact.
        if self.pending_comment.get().is_none()
            && self
                .thread
                .borrow()
                .as_ref()
                .is_some_and(|old| old.snapshot == thread)
        {
            return;
        }
        let summary = crate::thread_view::discussion_summary(&thread);
        let headline = crate::thread_view::discussion_title(
            &thread.root,
            self.state.borrow().preferences.show_dead,
        );
        self.label("DetailTitle").set_text(Some(&headline));
        self.label("DetailEyebrow").set_text(Some(&summary));
        // The cache preview, refresh and final network result are one visit.
        let reached_comment = self
            .pending_comment
            .get()
            .filter(|comment| thread.items.contains_key(comment));
        let first_load = self.thread.borrow().is_none();
        if first_load || reached_comment.is_some() {
            if matches!(self.undo_story.get(), Some(UndoStoryEdit::Read { id: edited, .. }) if (first_load && edited == id) || Some(edited) == reached_comment)
            {
                self.undo_story.set(None);
                self.button("UndoStory").set_is_visible(false);
                self.get_control::<Border>("StatusBar")
                    .set_is_visible(false);
            }
            let now = services::now();
            let mut state = self.state.borrow_mut();
            if first_load {
                state.mark_read(id, now);
            }
            if let Some(comment) = reached_comment {
                // The search result was read in its parent discussion. Keep History
                // anchored to the discussion and the unread filter anchored to the result.
                state.items.entry(comment).or_default().read_at = Some(now);
                let folds = &mut state.items.entry(id).or_default().collapsed;
                if !folds.is_empty() {
                    let mut cursor = Some(comment);
                    let mut seen = BTreeSet::new();
                    while let Some(item) = cursor.and_then(|i| thread.items.get(&i)) {
                        if item.id == id || !seen.insert(item.id) {
                            break;
                        }
                        folds.remove(&item.id);
                        cursor = item.parent;
                    }
                }
            }
            drop(state);
            self.changed();
        }
        self.thread.replace(Some(Rc::new(Discussion::new(thread))));
        self.render_thread();
        if first_load || reached_comment.is_some() {
            self.render_stories();
        }
    }
    fn render_thread(&self) {
        self.render_thread_at(None);
    }
    fn render_thread_at(&self, focus_comment: Option<u64>) {
        if !matches!(*self.detail.borrow(), DetailRoute::Thread(_)) {
            return;
        }
        let snapshot = self.thread.borrow();
        let Some(discussion) = snapshot.as_ref() else {
            return;
        };
        let thread = &discussion.snapshot;
        if Some(thread.root.id) != self.selected.get() {
            return;
        }
        let state = self.state.borrow();
        let query = self.find_query();
        let target = self
            .pending_comment
            .get()
            .filter(|id| thread.items.contains_key(id));
        if target.is_some() {
            self.pending_comment.set(None);
        }
        let resumed = self.resume_viewport.take();
        let mut anchor = focus_comment
            .or(target)
            .map(ReadingPosition::start)
            .or(resumed)
            .or_else(|| match self.thread_control.borrow().as_ref() {
                Some(control) => crate::thread_view::viewport_position(control),
                None => state.items.get(&thread.root.id).and_then(|local| {
                    Some(ReadingPosition {
                        item: local.scroll_anchor?,
                        inset: local.scroll_inset,
                    })
                }),
            });
        if self.applied_find_query.borrow().as_str() != query {
            self.find_match_id.set(None);
        }
        if query.is_empty() {
            if let Some(original) = self.find_origin.take() {
                anchor = Some(original);
            }
        } else if self.find_origin.get().is_none() {
            self.find_origin.set(Some(
                anchor.unwrap_or(ReadingPosition::start(thread.root.id)),
            ));
        }
        self.applied_find_query.replace(query.clone());
        let weak = self.to_ref().downgrade();
        let control = crate::thread_view::build_discussion(
            discussion.clone(),
            &state,
            self.visit_seen.borrow().clone(),
            &query,
            self.thread_sort.get(),
            self.new_replies_origin.get().is_some(),
            Rc::new(move |action| {
                if let Some(v) = weak.upgrade() {
                    v.thread_action(action);
                }
            }),
        );
        drop(state);
        drop(snapshot);
        self.show_content(control.clone());
        if !query.is_empty() {
            let result =
                crate::thread_view::select_find_match(&control, self.find_match_id.get(), 0);
            self.show_find_result(result, true);
            if let Some(position) = resumed {
                crate::thread_view::restore_position(&control, position, false);
            }
        } else {
            self.show_find_result(None, false);
            if let Some(position) = anchor {
                crate::thread_view::restore_position(
                    &control,
                    position,
                    focus_comment == Some(position.item),
                );
            }
        }
        *self.thread_control.borrow_mut() = Some(control);
        self.update_thread_commands();
    }
    /// Keep the baseline stable while reading so newly fetched replies retain their NEW badge.
    fn remember_thread(&self) -> Option<ThreadVisit> {
        let DetailRoute::Thread(id) = *self.detail.borrow() else {
            return None;
        };
        let visit = self.capture_thread_visit(id);
        self.browser_visits.borrow_mut().remember(&visit);
        let anchor = visit.presentation.as_ref().and_then(|p| p.position);
        self.save_reading_position(id, anchor);
        Some(visit)
    }

    fn save_reading_position(&self, id: u64, anchor: Option<ReadingPosition>) {
        let snapshot = self.thread.borrow();
        let Some(thread) = snapshot.as_ref().map(|discussion| &discussion.snapshot) else {
            return;
        };
        if thread.root.id != id {
            return;
        }
        let mut state = self.state.borrow_mut();
        let local = state.items.entry(id).or_default();
        let seen_before = local.seen_comments.len();
        local.seen_comments.extend(thread.items.keys().copied());
        let position = self
            .new_replies_origin
            .get()
            .or(self.find_origin.get())
            .or(anchor)
            .filter(|&position| position != ReadingPosition::start(id));
        let anchor = position.map(|p| p.item);
        let inset = position.map_or(0, |p| p.inset);
        if local.seen_comments.len() == seen_before
            && local.scroll_anchor == anchor
            && local.scroll_inset == inset
        {
            return;
        }
        local.scroll_anchor = anchor;
        local.scroll_inset = inset;
        drop(state);
        self.changed();
    }
    fn thread_action(&self, action: ThreadAction) {
        match action {
            ThreadAction::OpenUrl(url) => self.open_url(&url),
            ThreadAction::OpenArticle(id) => {
                let show_dead = self.state.borrow().preferences.show_dead;
                let url = self
                    .thread
                    .borrow()
                    .as_ref()
                    .map(|discussion| &discussion.snapshot.root)
                    .filter(|root| root.id == id)
                    .and_then(|root| root.visible_article_url(show_dead))
                    .unwrap_or_else(|| ember_core::item_url(id));
                self.open_url(&url);
            }
            ThreadAction::Profile(user) => self.show_profile(user),
            ThreadAction::SearchDiscussion(id) => self.open_search_form(Some(id)),
            ThreadAction::ShowNewReplies(enabled) => {
                if enabled {
                    let origin = self
                        .thread_control
                        .borrow()
                        .as_ref()
                        .and_then(|control| crate::thread_view::viewport_position(control))
                        .or_else(|| self.selected.get().map(ReadingPosition::start));
                    self.new_replies_origin.set(origin);
                }
                let restore = if enabled {
                    None
                } else {
                    self.new_replies_origin.take()
                };
                self.render_thread();
                if let Some(control) = self.thread_control.borrow().as_ref() {
                    if let Some(origin) = restore {
                        crate::thread_view::restore_position(control, origin, false);
                    }
                    if restore.is_none()
                        || restore == self.selected.get().map(ReadingPosition::start)
                    {
                        crate::thread_view::focus_thread_tool(control, None, "NewReplyFilter");
                    }
                }
            }
            ThreadAction::ReadInContext(id) => self.read_in_context(id),
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
                    self.render_thread_focusing(Some(id), "FoldReply");
                }
            }
            ThreadAction::FocusReading => self.set_focus_reading(true),
            ThreadAction::CopyCode(source) => self.copy_text(&source, "Code copied."),
            ThreadAction::OpenContentLink(url) => self.open_content_link(&url),
            ThreadAction::CopyCommentText(id) => {
                let show_dead = self.state.borrow().preferences.show_dead;
                let text = self
                    .thread
                    .borrow()
                    .as_ref()
                    .and_then(|discussion| discussion.comment_text(id, show_dead));
                if let Some(text) = text {
                    self.copy_text(&text, "Comment text copied.");
                } else {
                    self.status("No comment text to copy.");
                }
            }
            ThreadAction::CopyCommentLink(id) => {
                self.copy_text(&ember_core::item_url(id), "Comment link copied.");
            }
            ThreadAction::Share => {
                if let Some(id) = self.selected.get() {
                    self.copy_text(&ember_core::item_url(id), "Discussion link copied.");
                }
            }
            ThreadAction::Hide => {
                if let Some(id) = self.selected.get() {
                    self.toggle_story_flag(id, StoryFlag::Hidden);
                }
            }
            ThreadAction::MuteDomain => {
                let show_dead = self.state.borrow().preferences.show_dead;
                if let Some(item) = self
                    .current_item()
                    .filter(|item| item.visible_article_url(show_dead).is_some())
                {
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
                    self.fetch_thread(id, self.thread_fetch_mode(id));
                }
            }
            ThreadAction::Sort(sort) => {
                self.thread_sort.set(sort);
                self.render_thread_focusing(None, "ThreadSort");
            }
            ThreadAction::GoToItem(id) | ThreadAction::OpenComment(id) => {
                self.open_item_internal(id, true)
            }
        }
    }
    fn read_in_context(&self, id: u64) {
        let Some(root) = self.selected.get() else {
            return;
        };
        let Some(path) = self
            .thread_control
            .borrow()
            .as_ref()
            .and_then(|control| crate::thread_view::displayed_comment_path(control, id))
        else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let folds = &mut state.items.entry(root).or_default().collapsed;
        let previous_count = folds.len();
        for id in path {
            folds.remove(&id);
        }
        let expanded = folds.len() != previous_count;
        drop(state);
        if expanded {
            self.changed();
        }
        self.new_replies_origin.set(None);
        self.find_origin.set(None);
        self.set_find_bar_visible(false);
        self.textbox("ThreadFind").set_text(Some(""));
        self.render_thread_at(Some(id));
    }
    fn render_thread_focusing(&self, item: Option<u64>, name: &'static str) {
        self.render_thread();
        if let Some(control) = self.thread_control.borrow().as_ref() {
            crate::thread_view::focus_thread_tool(control, item, name);
        }
    }
    fn toggle_collapse_all(&self) {
        let snapshot = self.thread.borrow();
        let Some(thread) = snapshot.as_ref().map(|discussion| &discussion.snapshot) else {
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
        self.render_thread_focusing(None, "FoldAll");
    }
    fn toggle_story_flag(&self, id: u64, flag: StoryFlag) {
        if !self.state_ready.get() {
            self.status("Reading data is unavailable. Open Settings to restore a backup.");
            return;
        }
        let mut state = self.state.borrow_mut();
        let (before, message) = match flag {
            StoryFlag::Saved => {
                let saved_at = state.items.get(&id).and_then(|local| local.saved_at);
                let saved = state.toggle_saved(id, services::now());
                (
                    UndoStoryEdit::Saved { id, saved_at },
                    if saved {
                        "Added to Saved stories. Download the thread to keep all its comments offline."
                    } else {
                        "Removed from Saved stories."
                    },
                )
            }
            StoryFlag::Hidden => {
                let hidden = state.toggle_hidden(id);
                (
                    UndoStoryEdit::Hidden {
                        id,
                        hidden: !hidden,
                    },
                    if hidden {
                        "Story hidden."
                    } else {
                        "Story restored to your feeds."
                    },
                )
            }
            StoryFlag::Read => {
                let local = state.items.entry(id).or_default();
                let previous = local.read_at;
                local.read_at = previous.is_none().then(services::now);
                (
                    UndoStoryEdit::Read {
                        id,
                        read_at: previous,
                    },
                    if previous.is_some() {
                        "Marked unread."
                    } else {
                        "Marked read."
                    },
                )
            }
        };
        drop(state);
        self.update_after_story_edit(id, flag);
        self.status(message);
        self.undo_story.set(Some(before));
        self.button("UndoStory").set_is_visible(true);
    }
    fn undo_story_edit(&self) {
        let Some(edit) = self.undo_story.take() else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let (id, flag) = match edit {
            UndoStoryEdit::Saved { id, saved_at } => {
                state.items.entry(id).or_default().saved_at = saved_at;
                (id, StoryFlag::Saved)
            }
            UndoStoryEdit::Hidden { id, hidden } => {
                state.items.entry(id).or_default().hidden = hidden;
                (id, StoryFlag::Hidden)
            }
            UndoStoryEdit::Read { id, read_at } => {
                state.items.entry(id).or_default().read_at = read_at;
                (id, StoryFlag::Read)
            }
        };
        drop(state);
        self.update_after_story_edit(id, flag);
        self.status("Change undone.");
    }
    fn update_after_story_edit(&self, id: u64, flag: StoryFlag) {
        if matches!(flag, StoryFlag::Saved) && self.cache_work.get() == CacheWork::Downloading(id) {
            if let Some(transfer) = self.transfer.borrow_mut().as_mut() {
                transfer.keep_bookmark_choice = true;
            }
        }
        self.changed();
        if matches!(flag, StoryFlag::Saved) && *self.feed.borrow() != FeedRoute::Saved {
            self.refresh_story_cards(Some(id), false);
        } else {
            self.render_stories();
        }
        if matches!(
            (&*self.feed.borrow(), flag),
            (FeedRoute::Saved, StoryFlag::Saved) | (FeedRoute::Hidden, StoryFlag::Hidden)
        ) {
            self.load_feed(0, FetchMode::CacheFirst);
        }
        if self.selected.get() == Some(id) {
            if matches!(flag, StoryFlag::Hidden) {
                self.render_thread();
            }
            self.update_thread_commands();
        }
    }
    fn wire_phone_toolbar(&self) {
        let menu = ferroui_controls::MenuFlyout::new();
        let action = |label: &str, icon: &str, run: fn(&ReaderView)| {
            let item = MenuItem::new();
            item.set_header(Some(Rc::new(label.to_owned())));
            item.set_icon(Some(Control::boxed(crate::icons::icon(icon))));
            let weak = self.to_ref().downgrade();
            item.click(move |_, _| {
                if let Some(view) = weak.upgrade() {
                    run(&view);
                }
            });
            menu.items().add(Some(Control::boxed(&item)));
            item
        };
        action("Focus reading", "focus_reading", |v| {
            v.set_focus_reading(true)
        });
        let menu_download = action("Download thread", "download", Self::download);
        let menu_release = action("Stop keeping offline", "download", |v| {
            if let Some(id) = v.selected.get() {
                v.release_download(id);
            }
        });
        action("Refresh discussion", "refresh", |v| {
            if let Some(id) = v.selected.get() {
                v.fetch_thread(id, FetchMode::Refresh);
            }
        });
        action("Copy discussion link", "copy", |v| {
            v.thread_action(ThreadAction::Share)
        });
        let weak = self.to_ref().downgrade();
        menu.opening(move |_| {
            if let Some(view) = weak.upgrade() {
                let command = view.button("DownloadButton");
                menu_download.set_is_visible(!command.is_visible());
                menu_download.set_is_enabled(command.is_enabled());
                let verified = command.flyout().is_some();
                let label = if verified {
                    "Check offline copy"
                } else {
                    "Download thread"
                };
                menu_download.set_header(Some(Rc::new(label.to_owned())));
                menu_release.set_is_visible(verified && !command.is_visible());
                menu_release.set_is_enabled(command.is_enabled());
            }
        });
        self.button("ToolbarMore").set_flyout(menu);
    }
    fn update_thread_commands(&self) {
        self.update_download_command();
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
    fn set_cache_work(&self, work: CacheWork) {
        self.cache_work.set(work);
        self.update_download_command();
    }
    fn update_download_command(&self) {
        let button = self.button("DownloadButton");
        let selected = self.selected.get();
        let downloaded =
            selected.is_some_and(|id| self.state.borrow().downloaded_threads.contains_key(&id));
        if button.classes().contains("active") != downloaded {
            button.classes().set("active", downloaded);
            button.set_content(Some(Control::boxed(crate::icons::icon(if downloaded {
                "check"
            } else {
                "download"
            }))));
        }
        let menu_id = button
            .tag()
            .and_then(|tag| tag.downcast_ref::<u64>().copied());
        if let Some(id) = selected.filter(|_| downloaded) {
            if menu_id != Some(id) {
                button.set_flyout(self.offline_actions(id));
                button.set_tag(Some(Rc::new(id)));
            }
        } else if menu_id.is_some() {
            button.clear_value(Button::flyout_property());
            button.set_tag(None);
        }
        let work = self.cache_work.get();
        button.set_is_enabled(work == CacheWork::Idle);
        let name = match work {
            CacheWork::Pruning => "Managing offline storage",
            CacheWork::Clearing => "Clearing offline storage",
            CacheWork::Cleaning => "Cleaning temporary cache",
            CacheWork::Downloading(id) if Some(id) == selected => "Downloading this discussion",
            CacheWork::Downloading(_) => "Downloading another discussion",
            CacheWork::Idle if downloaded => "Manage downloaded discussion",
            CacheWork::Idle => "Download thread",
        };
        AutomationProperties::set_name(&button, Some(name));
        ToolTip::set_tip(&button, Some(Rc::new(name.to_owned()) as BoxedValue));
    }
    fn download(&self) {
        if let Some(id) = self.selected.get() {
            self.start_download(id);
        }
    }
    fn clear_cache(&self) {
        if !self.state_ready.get() {
            self.status("Restore your reading data before clearing offline storage.");
            return;
        }
        if self.cache_work.get() != CacheWork::Idle {
            self.status("Offline storage is busy. Try clearing it again when the current operation finishes.");
            return;
        }
        let Some(repo) = self.repo() else {
            return;
        };
        self.set_cache_work(CacheWork::Clearing);
        let weak = self.to_ref().downgrade();
        drop(Dispatcher::ui_thread().invoke_async_task_local(move || async move {
            let result = repo.clear_cache().await;
            let Some(view) = weak.upgrade() else { return; };
            // Even an interrupted clear can remove part of a previously verified thread.
            view.forget_feed_windows();
            view.state.borrow_mut().downloaded_threads.clear();
            view.set_cache_work(CacheWork::Idle);
            view.changed();
            view.render_stories();
            if view.downloaded_shelf() { view.load_feed(0, FetchMode::Offline); }
            view.refresh_storage_report();
            match result {
                Ok(()) => view.status("Cached content cleared. Bookmarks and preferences are retained."),
                Err(error) => view.status(&format!("Cache clearing did not finish: {error}. Download discussions again to verify offline availability.")),
            }
        }));
    }
    fn navigate(&self, route: DetailRoute) {
        let visit = if *self.detail.borrow() != route {
            self.remember_thread()
        } else {
            None
        };
        let old = self.detail.replace(route.clone());
        if old != route && old != DetailRoute::Welcome {
            let entry = match (visit, self.search_form.borrow().as_ref()) {
                (Some(visit), _) => HistoryEntry::Thread(visit),
                (None, Some(form)) => HistoryEntry::Search(Box::new((form.capture)())),
                (None, None) => HistoryEntry::Route(old),
            };
            self.history.borrow_mut().push(entry);
        }
        self.detail_generation
            .set(self.detail_generation.get().wrapping_add(1));
        self.configure_detail();
        self.apply_layout();
        self.update_navigation();
        self.animate_detail();
    }
    fn capture_thread_visit(&self, id: u64) -> ThreadVisit {
        let find = self
            .get_control::<Border>("ThreadTools")
            .is_visible()
            .then(|| FindVisit {
                text: self.textbox("ThreadFind").text().unwrap_or_default(),
                // A draft can change immediately before navigating, during debounce.
                match_id: self
                    .find_match_id
                    .get()
                    .filter(|_| self.applied_find_query.borrow().as_str() == self.find_query()),
                origin: self.find_origin.get(),
            });
        ThreadVisit {
            root: id,
            source_row: self.selected_row.get().unwrap_or(id),
            pending_comment: self.pending_comment.get(),
            budget: self.thread_budget.get(),
            browser_entry: self.browser_entry.get(),
            presentation: Some(Rc::new(ThreadPresentation {
                seen: self.visit_seen.borrow().clone(),
                position: self.resume_viewport.get().or_else(|| {
                    self.thread_control
                        .borrow()
                        .as_ref()
                        .and_then(|thread| crate::thread_view::viewport_position(thread))
                }),
                new_replies_origin: self.new_replies_origin.get(),
                sort: self.thread_sort.get(),
                focus_reading: self.focus_reading.get(),
                find,
            })),
        }
    }

    fn forget_thread_presentations(&self) {
        self.browser_visits.take();
        self.visit_seen.replace(Rc::default());
        self.resume_viewport.set(None);
        for entry in self.history.borrow_mut().iter_mut() {
            if let HistoryEntry::Thread(visit) = entry {
                visit.presentation = None;
            }
        }
    }
    fn configure_detail(&self) {
        self.search_form.borrow_mut().take();
        self.storage_report.borrow_mut().take();
        self.explore_counts.borrow_mut().clear();
        self.profile_mute.borrow_mut().take();
        self.settings_back.borrow_mut().take();
        self.pending_settings.borrow_mut().take();
        self.live_settings.borrow_mut().take();
        let welcome = *self.detail.borrow() == DetailRoute::Welcome;
        self.get_control::<ScrollViewer>("WelcomePane")
            .set_is_visible(welcome);
        self.get_control::<ContentControl>("DetailHost")
            .set_is_visible(!welcome);
        let thread = matches!(*self.detail.borrow(), DetailRoute::Thread(_));
        if !thread {
            self.focus_reading.set(false);
        }
        self.get_control::<Border>("ThreadTools")
            .set_is_visible(false);
        self.button("FindButton").classes().set("active", false);
        self.find_origin.set(None);
        self.new_replies_origin.set(None);
        self.applied_find_query.borrow_mut().clear();
        self.show_find_result(None, false);
        for name in ["SaveButton", "FindButton", "DownloadButton"] {
            self.button(name).set_is_visible(thread);
        }
        self.get_control::<Border>("DetailHeader")
            .set_is_visible(*self.detail.borrow() != DetailRoute::Welcome);
    }
    fn find_query(&self) -> String {
        self.textbox("ThreadFind")
            .text()
            .unwrap_or_default()
            .trim()
            .to_lowercase()
    }
    fn step_find(&self, direction: i32) {
        let query = self.find_query();
        if query.is_empty() {
            return;
        }
        if self.applied_find_query.borrow().as_str() != query {
            self.render_thread();
            if direction > 0 {
                return;
            }
        }
        if let Some(control) = self.thread_control.borrow().as_ref() {
            let result =
                crate::thread_view::select_find_match(control, self.find_match_id.get(), direction);
            self.show_find_result(result, true);
        }
    }
    fn show_find_result(&self, result: Option<crate::thread_view::FindMatch>, searching: bool) {
        self.find_match_id.set(result.map(|m| m.id));
        for name in ["FindPrevious", "FindNext"] {
            self.button(name)
                .set_is_enabled(result.is_some_and(|m| m.total > 1));
        }
        let label = self.label("FindCount");
        label.set_is_visible(searching);
        if searching {
            let message = match result {
                Some(m) if m.total == 1 => "1 matching comment".to_owned(),
                Some(m) => format!("{} of {} matching comments", m.ordinal, m.total),
                None => "No matches in loaded comments".to_owned(),
            };
            label.set_text(Some(&message));
        }
    }
    fn set_find_bar_visible(&self, open: bool) {
        self.get_control::<Border>("ThreadTools")
            .set_is_visible(open);
        self.button("FindButton").classes().set("active", open);
    }
    fn set_find_open(&self, open: bool) {
        self.set_find_bar_visible(open);
        if open {
            self.textbox("ThreadFind").focus();
        } else {
            self.textbox("ThreadFind").set_text(Some(""));
            if !self.applied_find_query.borrow().is_empty() {
                self.render_thread();
            }
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
        self.get_control::<ContentControl>("DetailHost")
            .set_content(None);
        self.thread_control.borrow_mut().take();
        self.thread.borrow_mut().take();
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
        if self
            .settings_back
            .borrow()
            .as_ref()
            .is_some_and(|settings| settings.dismiss())
        {
            return;
        }
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
        let route = match route {
            Some(HistoryEntry::Thread(visit)) => {
                self.present_discussion(visit, true);
                return;
            }
            Some(HistoryEntry::Search(draft)) => {
                self.present_search_form(None, None, Some(*draft));
                return;
            }
            Some(HistoryEntry::Route(route)) => Some(route),
            None => None,
        };
        match route {
            Some(DetailRoute::Thread(id)) => {
                self.open_item_internal(id, true);
            }
            Some(DetailRoute::Profile(user)) => self.show_profile(user),
            Some(DetailRoute::Settings) => self.show_settings(),
            Some(DetailRoute::Search) => self.show_search(),
            Some(DetailRoute::More) => self.show_more(),
            _ => self.return_to_feed(true),
        }
    }
    fn return_to_feed(&self, publish: bool) {
        let row = self.selected_row.get();
        self.back_to_feed(publish);
        let Some(row) = row else { return };
        let list = self.get_control::<ListBox>("Stories");
        // The list retains its viewport while the phone shows the discussion.
        // Never scroll to an unrelated row when an Unread filter has removed it.
        let origin = list
            .get_visual_descendants()
            .filter_map(|v| v.cast::<StoryCard>())
            .find(|card| card.tag().and_then(|id| id.downcast_ref::<u64>().copied()) == Some(row));
        if let Some(card) = origin {
            card.focus();
        } else if list.items().count() != 0 {
            list.focus();
        } else if self.button("RetryButton").is_effectively_visible() {
            self.button("RetryButton").focus();
        }
    }
    fn publish_location(&self, id: Option<u64>) {
        LOCATION_CHANGED.with(|v| {
            if let Some(callback) = v.borrow().clone() {
                callback(id);
            }
        });
    }
    fn wire_quick_search(&self) {
        let error = self.label("QuickSearchError");
        AutomationProperties::set_live_setting(&error, AutomationLiveSetting::Polite);
        let field = self.textbox("SearchBox");
        let input = field.downgrade();
        let button = self.button("SearchButton");
        field.property_changed(move |event| {
            if event.property() != TextBox::text_property().as_property() {
                return;
            }
            let Some(field) = input.upgrade() else { return };
            let query = field.text().unwrap_or_default();
            let (label, icon) = crate::search_panel::search_action(&query);
            if AutomationProperties::get_name(&button).as_deref() != Some(label) {
                button.set_content(Some(Control::boxed(crate::icons::icon(icon))));
                AutomationProperties::set_name(&button, Some(label));
                ToolTip::set_tip(&button, Some(Rc::new(label.to_owned()) as BoxedValue));
            }
            if error.is_visible()
                && (crate::search_panel::validate_terms(query.trim()).is_ok()
                    || ember_core::HnDestination::from_link(&query).is_some())
            {
                widgets::FormValidation::default().check(&field, Ok(()));
                error.set_is_visible(false);
            }
        });
        let weak = self.to_ref().downgrade();
        self.textbox("SearchBox").add_handler(
            InputElement::key_down_event(),
            move |_, e: &KeyEventArgs| {
                if e.key == Key::Enter && e.key_modifiers == KeyModifiers::NONE {
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
    }

    fn submit_quick_search(&self) {
        let field = self.textbox("SearchBox");
        let query = field.text().unwrap_or_default();
        if let Some(link) = ember_core::HnDestination::from_link(&query) {
            self.open_hn_destination(link);
        } else if query.trim().is_empty() {
            self.show_search();
        } else {
            let mut check = widgets::FormValidation::default();
            check.check(&field, crate::search_panel::validate_terms(query.trim()));
            if !check.finish(&self.label("QuickSearchError")) {
                return;
            }
            self.run_search(SearchQuery {
                query,
                ..Default::default()
            });
        }
    }
    fn open_content_link(&self, url: &str) {
        match ember_core::HnDestination::from_link(url) {
            Some(destination) => self.open_hn_destination(destination),
            None => self.open_url(url),
        }
    }
    fn open_hn_destination(&self, destination: ember_core::HnDestination) {
        match destination {
            ember_core::HnDestination::Discussion(id) => self.open_item_internal(id, true),
            ember_core::HnDestination::Profile(user) => self.show_profile(user),
        }
    }
    fn show_search(&self) {
        self.open_search_form(None);
    }
    fn show_profile_lookup(&self, name: &str) {
        // Correct the failed lookup in place. If Search opened it, restore that
        // draft without leaving the unavailable profile on the Back stack.
        let draft = self
            .history
            .borrow_mut()
            .pop_if(|entry| matches!(entry, HistoryEntry::Search(_)));
        let draft = match draft {
            Some(HistoryEntry::Search(draft)) => Some(*draft),
            _ => None,
        };
        self.detail.replace(DetailRoute::Welcome);
        self.present_search_form(Some(name), None, draft);
    }
    fn open_search_form(&self, discussion: Option<u64>) {
        if discussion.is_none() && *self.detail.borrow() == DetailRoute::Search {
            if let Some(form) = self.search_form.borrow().as_ref() {
                form.terms.focus();
                form.terms.bring_into_view();
                return;
            }
        }
        self.present_search_form(None, discussion, None);
    }
    fn present_search_form(
        &self,
        profile_lookup: Option<&str>,
        discussion: Option<u64>,
        draft: Option<crate::search_panel::Draft>,
    ) {
        self.navigate(DetailRoute::Search);
        self.label("DetailEyebrow").set_text(Some("DISCOVER"));
        self.label("DetailTitle").set_text(Some("Search"));
        let query = match (discussion, &*self.feed.borrow()) {
            (Some(id), _) => SearchQuery {
                story_id: Some(id),
                kind: SearchKind::Comments,
                ..Default::default()
            },
            (_, FeedRoute::Search(q)) => q.clone(),
            _ => SearchQuery {
                query: self.textbox("SearchBox").text().unwrap_or_default(),
                ..Default::default()
            },
        };
        let weak = self.to_ref().downgrade();
        let profile = self.to_ref().downgrade();
        let history = self.to_ref().downgrade();
        let form = crate::search_panel::build(
            &query,
            &self.state.borrow().recent_searches,
            profile_lookup,
            draft,
            Rc::new(move |term| {
                if let Some(v) = history.upgrade() {
                    v.state
                        .borrow_mut()
                        .recent_searches
                        .retain(|saved| saved != term);
                    v.changed();
                }
            }),
            Rc::new(move |query| {
                if let Some(v) = weak.upgrade() {
                    v.run_search(query);
                }
            }),
            Rc::new(move |link| {
                if let Some(v) = profile.upgrade() {
                    v.open_hn_destination(link);
                }
            }),
        );
        self.show_content(form.control.clone());
        self.search_form.replace(Some(form));
    }
    fn run_search(&self, mut query: SearchQuery) {
        query.query = query.query.trim().to_owned();
        query.page = 0;
        self.state.borrow_mut().remember_search(&query.query);
        self.changed();
        self.textbox("SearchBox").set_text(Some(&query.query));
        let route = FeedRoute::Search(query);
        if *self.feed.borrow() == route
            && matches!(self.feed_load.get(), FeedLoad::Ready | FeedLoad::Loading)
        {
            self.back_to_feed(true);
        } else {
            self.select_feed(route);
        }
    }
    fn show_settings(&self) {
        if *self.detail.borrow() != DetailRoute::Settings {
            self.present_settings();
        }
    }
    fn present_settings(&self) {
        self.navigate(DetailRoute::Settings);
        self.label("DetailEyebrow").set_text(Some("READER"));
        self.label("DetailTitle").set_text(Some("Settings"));
        let weak = self.to_ref().downgrade();
        let (content, restore, back) = crate::settings::build(Rc::new(move |action| {
            if let Some(v) = weak.upgrade() {
                v.setting_action(action);
            }
        }));
        self.settings_back.replace(Some(back));
        if self.state_ready.get() {
            self.live_settings
                .replace(Some(restore(&self.state.borrow())));
        } else {
            self.pending_settings.replace(Some(restore));
        }
        self.show_content(content);
    }
    fn setting_action(&self, action: SettingsAction) {
        match action {
            SettingsAction::Preferences(mut preferences) => {
                preferences.normalize();
                let previous = std::mem::replace(
                    &mut self.state.borrow_mut().preferences,
                    preferences.clone(),
                );
                crate::theme::apply(&preferences, Some(&previous));
                self.changed();
                if previous.compact != preferences.compact
                    || previous.hide_read != preferences.hide_read
                    || previous.show_dead != preferences.show_dead
                {
                    self.render_stories();
                }
                if previous.cache_max_items != preferences.cache_max_items {
                    self.prune_cache();
                }
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
            SettingsAction::Import(state) => self.import_backup(state),
            SettingsAction::ClearHistory => {
                self.forget_thread_presentations();
                {
                    let mut state = self.state.borrow_mut();
                    state.recent_searches.clear();
                    state.items.retain(|_, entry| {
                        entry.read_at = None;
                        entry.last_opened_at = None;
                        entry.seen_comments.clear();
                        entry.scroll_anchor = None;
                        entry.scroll_inset = 0;
                        *entry != ember_core::ItemState::default()
                    });
                }
                self.changed();
                if *self.feed.borrow() == FeedRoute::History {
                    self.load_feed(0, FetchMode::Offline);
                } else {
                    self.render_stories();
                }
                self.status("Reading history cleared. Saved stories are retained.");
            }
            SettingsAction::ClearCache => self.clear_cache(),
            SettingsAction::CleanTemporary => self.clean_temporary_cache(),
            SettingsAction::MeasureStorage(report) => self.measure_storage(report),
        }
    }
    fn import_backup(&self, mut state: ReaderState) {
        let generation = self.library_generation.get().wrapping_add(1);
        self.library_generation.set(generation);
        // A backup can come from a different device and never contains cached payloads.
        state.downloaded_threads.clear();
        if self.state_ready.get() {
            self.finish_import(state);
            return;
        }
        let Some(repo) = self.repo() else {
            self.status("Cannot import the backup until the reading database opens.");
            return;
        };
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                // Fetch the local manifest before overwriting persistent state, even if the
                // startup callback hasn't run. A corrupt state can still be recovered by import.
                if let Ok(local) = repo.load_state().await {
                    state.downloaded_threads = local.downloaded_threads;
                }
                if let Some(view) = weak.upgrade() {
                    if view.library_generation.get() == generation {
                        view.finish_import(state);
                    }
                }
            }),
        );
    }
    fn finish_import(&self, mut state: ReaderState) {
        self.forget_thread_presentations();
        if self.state_ready.get() {
            state.downloaded_threads =
                std::mem::take(&mut self.state.borrow_mut().downloaded_threads);
        }
        if let CacheWork::Downloading(id) = self.cache_work.get() {
            // The in-flight transfer may already have changed part of the local copy.
            state.downloaded_threads.remove(&id);
        }
        crate::theme::apply(&state.preferences, None);
        *self.state.borrow_mut() = state;
        self.state_ready.set(true);
        self.changed();
        self.render_stories();
        if matches!(
            *self.feed.borrow(),
            FeedRoute::Saved | FeedRoute::Hidden | FeedRoute::History
        ) {
            self.load_feed(0, FetchMode::CacheFirst);
        }
        self.present_settings();
        self.status("Reading library imported.");
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
    fn move_selection(&self, forward: bool, focused: Option<u64>) {
        let target = {
            // Keep the source row as the cursor even when opening it removed it from Unread.
            let stories = self.stories.borrow();
            let state = self.state.borrow();
            let feed = self.feed.borrow();
            let visible =
                |item: &&Rc<HnItem>| feed.includes(item, &state, self.downloaded_only.get());
            let current = focused
                .or(self.selected_row.get())
                .and_then(|id| stories.position(id));
            let next = match current {
                Some(i) if forward => stories[i + 1..].iter().find(visible),
                Some(i) => stories[..i].iter().rev().find(visible),
                None => stories.iter().find(visible),
            };
            next.map(|item| item.id)
        };
        if let Some(id) = target {
            if let Some(index) = self.rendered_stories.snapshot().iter().position(|value| {
                value
                    .as_ref()
                    .and_then(|v| v.downcast_ref::<HnItem>())
                    .is_some_and(|item| item.id == id)
            }) {
                self.get_control::<ListBox>("Stories")
                    .scroll_into_view(index as i32);
            }
            self.open_item(id);
            if focused.is_some() {
                self.button("BackButton").focus();
            }
        }
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
            .and_then(|c| crate::thread_view::viewport_position(c));
        self.apply_layout();
        if let (Some(anchor), Some(thread)) = (anchor, self.thread_control.borrow().as_ref()) {
            crate::thread_view::restore_position(thread, anchor, false);
        }
        let focus = self.button("FocusButton");
        if focus.is_effectively_visible() {
            focus.focus();
        } else {
            self.button("ToolbarMore").focus();
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
        } else if (600.0..960.0).contains(&width) && (0.0..=500.0).contains(&self.bounds().height) {
            WorkspaceLayout::ShortLandscape
        } else if width < 760.0 {
            WorkspaceLayout::Phone
        } else if width < 1120.0 {
            WorkspaceLayout::CompactRail
        } else {
            WorkspaceLayout::LabeledRail
        };
        let landscape = mode == WorkspaceLayout::ShortLandscape;
        let phone = mode == WorkspaceLayout::Phone || landscape;
        let reading = mode == WorkspaceLayout::Reading;
        let split = !phone && !reading;
        let navigation_focus = if self.layout_mode.get() != Some(mode) {
            self.navigation_focus_after_resize(mode)
        } else {
            None
        };
        let detail = *self.detail.borrow() != DetailRoute::Welcome;
        let thread = matches!(*self.detail.borrow(), DetailRoute::Thread(_));
        self.get_control::<Border>("NavigationRail")
            .set_is_visible(split);
        self.get_control::<Border>("BottomNavigation")
            .set_is_visible(phone);
        let feed_visible = split || (phone && !detail);
        self.get_control::<Border>("FeedPane")
            .set_is_visible(feed_visible);
        if feed_visible && self.initial_feed_pending.get() {
            let weak = self.to_ref().downgrade();
            Dispatcher::ui_thread().post_local(
                move || {
                    if let Some(view) = weak.upgrade() {
                        if view.initial_feed_pending.get()
                            && view.get_control::<Border>("FeedPane").is_visible()
                        {
                            view.load_feed(0, FetchMode::CacheFirst);
                        }
                    }
                },
                DispatcherPriority::BACKGROUND,
            );
        }
        self.get_control::<Border>("DetailPane")
            .set_is_visible(split || detail);
        self.get_control::<Border>("Masthead")
            .set_is_visible(split || (phone && !detail));
        self.button("ToolbarMore").set_is_visible(phone && thread);
        // On narrow phones the secondary offline command yields its slot to the headline.
        self.button("DownloadButton")
            .set_is_visible(thread && (!phone || width >= 380.0));
        self.button("FocusButton").set_is_visible(!phone && thread);
        self.get_control::<Border>("SearchHost")
            .set_is_visible(mode == WorkspaceLayout::LabeledRail);
        self.button("CompactSearch")
            .set_is_visible(mode == WorkspaceLayout::CompactRail);
        self.label("BrandSubtitle").set_is_visible(split);
        if self.layout_mode.replace(Some(mode)) == Some(mode) {
            return;
        }
        // Move the same five tab controls to the leading edge on short landscape screens.
        // A single control tree preserves focus, tooltips and accessibility across rotation.
        let shell = self.get_control::<Grid>("ReaderShell");
        shell.set_column_definitions(
            ColumnDefinitions::parse(if landscape { "64,*" } else { "0,*" })
                .expect("reader shell columns"),
        );
        let tabs = self.get_control::<Border>("BottomNavigation");
        Grid::set_column(&tabs, if landscape { 0 } else { 1 });
        Grid::set_row(&tabs, if landscape { 0 } else { 3 });
        Grid::set_row_span(&tabs, if landscape { 4 } else { 1 });
        tabs.classes().set("bottom-tabs", !landscape);
        tabs.set_border_thickness(if landscape {
            Thickness::new(0.0, 0.0, 1.0, 0.0)
        } else {
            Thickness::new(0.0, 1.0, 0.0, 0.0)
        });
        self.get_control::<ferroui_controls::primitives::UniformGrid>("PrimaryTabs")
            .set_columns(if landscape { 1 } else { 5 });
        let columns = match mode {
            WorkspaceLayout::Phone | WorkspaceLayout::ShortLandscape | WorkspaceLayout::Reading => {
                "0,*,0"
            }
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
        if focus.classes().contains("active") != reading {
            let (label, icon) = if reading {
                ("Exit focus reading", "exit_focus")
            } else {
                ("Focus reading", "focus_reading")
            };
            focus.set_content(Some(Control::boxed(crate::icons::icon(icon))));
            focus.classes().set("active", reading);
            AutomationProperties::set_name(&focus, Some(label));
            ToolTip::set_tip(&focus, Some(Rc::new(format!("{label} (F)")) as BoxedValue));
        }
        let compact = mode == WorkspaceLayout::CompactRail;
        self.get_control::<Border>("NavigationRail")
            .classes()
            .set("compact-rail", compact);
        for name in ["ExploreLabel", "LibraryLabel"] {
            self.label(name).set_is_visible(!compact);
        }
        self.update_navigation();
        if let Some(name) = navigation_focus {
            let target = self.get_control::<Control>(name);
            target.focus();
            target.bring_into_view();
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

#[cfg(test)]
mod browser_visit_tests {
    use super::*;

    #[test]
    fn recently_left_browser_slots_replace_their_own_state_and_evict_the_oldest() {
        let mut cache = BrowserVisits::default();
        for entry in 1..=MAX_BROWSER_VISITS as u64 {
            cache.remember(&ThreadVisit::new(17, entry, Some(entry)));
        }
        cache.remember(&ThreadVisit::new(17, 123, Some(1)));
        cache.remember(&ThreadVisit::new(17, 124, None));
        cache.remember(&ThreadVisit::new(18, 18, Some(100)));
        assert_eq!(cache.0.len(), MAX_BROWSER_VISITS);
        assert_eq!(cache.get(1, 17).unwrap().source_row, 123);
        assert!(cache.get(2, 17).is_none());
        assert!(cache.get(100, 17).is_none());
        assert_eq!(cache.get(100, 18).unwrap().root, 18);
    }
}
