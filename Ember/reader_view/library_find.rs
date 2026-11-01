//! Find cached entries across a local collection, including pages not yet displayed.
use super::*;
use ferroui_base::Vector;

#[derive(Default)]
pub(super) struct LibraryFind {
    pub(super) query: String,
    revision: u64,
    missing: usize,
    browse_window: Option<BrowseWindow>,
}

struct BrowseWindow {
    stories: FeedRows,
    next: Option<usize>,
    summary: String,
    position: Vector,
    revision: u64,
    downloaded: bool,
    load: FeedLoad,
}

impl ReaderView {
    pub(super) fn wire_library_find(&self) {
        self.on("LibraryFindButton", |v| v.set_library_find_open(true));
        self.on("CloseLibraryFind", |v| v.set_library_find_open(false));
        let field = self.textbox("LibraryFind");
        let weak = self.to_ref().downgrade();
        field.property_changed(move |event| {
            if event.property() != TextBox::text_property().as_property() {
                return;
            }
            let Some(view) = weak.upgrade() else {
                return;
            };
            let revision = {
                let mut find = view.library_find.borrow_mut();
                find.revision = find.revision.wrapping_add(1);
                find.revision
            };
            let target = view.downgrade();
            DispatcherTimer::run_once(
                move || {
                    if let Some(view) = target
                        .upgrade()
                        .filter(|v| v.library_find.borrow().revision == revision)
                    {
                        view.apply_library_query();
                    }
                },
                Duration::from_millis(180),
                DispatcherPriority::BACKGROUND,
            );
        });
        let weak = self.to_ref().downgrade();
        field.add_handler(
            InputElement::key_down_event(),
            move |_, event: &KeyEventArgs| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                if event.key_modifiers != KeyModifiers::NONE
                    || view.textbox("LibraryFind").is_composing_text()
                {
                    return;
                }
                match event.key {
                    Key::Enter => view.apply_library_query(),
                    Key::Escape => view.set_library_find_open(false),
                    _ => return,
                }
                event.set_handled(true);
            },
        );
    }

    pub(super) fn set_library_find_open(&self, open: bool) {
        let panel = self.get_control::<Grid>("LibraryFindBar");
        panel.set_is_visible(open);
        self.button("LibraryFindButton")
            .classes()
            .set("active", open);
        if open {
            self.textbox("LibraryFind").focus();
        } else {
            self.textbox("LibraryFind").set_text(Some(""));
            self.apply_library_query();
            self.button("LibraryFindButton").focus();
        }
    }

    pub(super) fn reset_library_find(&self) {
        self.get_control::<Grid>("LibraryFindBar")
            .set_is_visible(false);
        self.button("LibraryFindButton")
            .classes()
            .set("active", false);
        self.textbox("LibraryFind").set_text(Some(""));
        *self.library_find.borrow_mut() = LibraryFind::default();
    }

    fn apply_library_query(&self) {
        if !self.feed.borrow().is_library() {
            return;
        }
        let field = self.textbox("LibraryFind");
        if field.is_composing_text() {
            return;
        }
        let query = field
            .text()
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let scroll = self.story_scroll();
        {
            let mut find = self.library_find.borrow_mut();
            if find.query == query {
                return;
            }
            if find.query.is_empty() {
                find.browse_window = Some(BrowseWindow {
                    stories: self.stories.take(),
                    next: self.next.get(),
                    summary: self.label("FeedSummary").text().unwrap_or_default(),
                    position: scroll.as_ref().map(|s| s.offset()).unwrap_or_default(),
                    revision: self.revision.get(),
                    downloaded: self.downloaded_only.get(),
                    load: self.feed_load.get(),
                });
            }
            find.query = query;
        }
        if self.library_find.borrow().query.is_empty() {
            let previous = self.library_find.borrow_mut().browse_window.take();
            if let Some(previous) = previous {
                // Invalidate any older scan before restoring the original paged list.
                self.feed_generation
                    .set(self.feed_generation.get().wrapping_add(1));
                let same_shelf = previous.downloaded == self.downloaded_only.get();
                let ready = previous.load == FeedLoad::Ready
                    && same_shelf
                    && previous.revision == self.revision.get();
                self.stories.replace(previous.stories);
                self.next.set(previous.next);
                self.feed_load.set(previous.load);
                self.label("FeedSummary").set_text(Some(&previous.summary));
                self.button("LoadMoreButton")
                    .set_is_visible(previous.next.is_some());
                self.button("LoadMoreButton").set_is_enabled(true);
                self.render_stories();
                if !ready {
                    self.load_feed(0, FetchMode::CacheFirst);
                }
                if same_shelf {
                    self.restore_feed_scroll(previous.position);
                }
                return;
            }
        }
        if let Some(scroll) = scroll {
            scroll.set_offset(Vector::default());
        }
        self.stories.borrow_mut().clear();
        self.next.set(None);
        self.button("LoadMoreButton").set_is_visible(false);
        self.load_feed(0, FetchMode::CacheFirst);
        self.render_stories();
    }

    pub(super) fn scan_library(
        &self,
        repo: Rc<Repository>,
        ids: Vec<u64>,
        offset: usize,
        limit: usize,
        generation: u64,
    ) {
        let query = self.library_find.borrow().query.clone();
        let weak = self.to_ref().downgrade();
        let show_dead = self.state.borrow().preferences.show_dead;
        if offset == 0 {
            self.library_find.borrow_mut().missing = 0;
        }
        self.label("FeedSummary")
            .set_text(Some("Searching this device…"));
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let terms: Vec<_> = query.split_whitespace().collect();
                let mut found = Vec::new();
                let mut missing = 0;
                let mut cursor = offset.min(ids.len());
                while cursor < ids.len() && found.len() < limit {
                    // Bound concurrent storage work and stop reading as soon as the search changes.
                    let end = (cursor + 32).min(ids.len());
                    let batch = repo
                        .load_items(&ids[cursor..end], services::now(), FetchMode::Offline)
                        .await;
                    if !weak
                        .upgrade()
                        .is_some_and(|v| v.feed_generation.get() == generation)
                    {
                        return;
                    }
                    let mut loaded = batch.items.into_iter().peekable();
                    while cursor < end && found.len() < limit {
                        let id = ids[cursor];
                        cursor += 1;
                        if let Some(item) = loaded.next_if(|item| item.id == id) {
                            if StorySummary::matches(&item, &terms, show_dead) {
                                found.push(item);
                            }
                        } else {
                            missing += 1;
                        }
                    }
                }
                let Some(view) = weak
                    .upgrade()
                    .filter(|v| v.feed_generation.get() == generation)
                else {
                    return;
                };
                view.library_find.borrow_mut().missing += missing;
                view.feed_load.set(FeedLoad::Ready);
                view.button("LoadMoreButton").set_is_enabled(true);
                let next = (cursor < ids.len()).then_some(cursor);
                view.accept_items(found, offset, next, ids.len(), DataSource::Cache, None);
                let count = view.stories.borrow().len();
                let count_label = match (count, next.is_some()) {
                    (_, true) => format!("{count}+ matches"),
                    (1, false) => "1 match".to_owned(),
                    _ => format!("{count} matches"),
                };
                let missing = view.library_find.borrow().missing;
                let detail = if missing == 0 {
                    "On this device".to_owned()
                } else {
                    format!("{missing} unavailable on this device")
                };
                view.label("FeedSummary")
                    .set_text(Some(&format!("{count_label} · {detail}")));
            }),
        );
    }
}
