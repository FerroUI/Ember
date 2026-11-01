//! The virtualized native conversation surface. Only realized rows create rich-text controls.

use crate::widgets::{self, bind_resource, muted, text as label};
use ember_content::{FindHighlight, HtmlDocument, RenderOptions};
use ember_core::{
    flatten_thread, flatten_thread_since, flatten_thread_with_match, relative_time, HnItem,
    ItemKind, ReaderState, ThreadOptions, ThreadRow, ThreadSnapshot, ThreadSort,
};
use ferroui_base::controls::ResourceKey;
use ferroui_base::layout::{HorizontalAlignment, Orientation, VerticalAlignment};
use ferroui_base::media::{FontWeight, IBrush, TextTrimming};
use ferroui_base::metadata::from_markup_value;
use ferroui_base::threading::{Dispatcher, DispatcherPriority};
use ferroui_base::{BoxedValue, CornerRadius, Point, Ref, Thickness, Vector};
use ferroui_controls::automation::AutomationProperties;
use ferroui_controls::primitives::ScrollBarVisibility;
use ferroui_controls::templates::FuncDataTemplate;
use ferroui_controls::{
    Application, Border, Button, ColumnDefinitions, ComboBox, Control, Grid, ItemsSource, ListBox,
    MenuFlyout, MenuItem, ProgressBar, ScrollViewer, StackPanel, TextBlock, ToolTip, WrapPanel,
};
use std::cell::{Cell, OnceCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum ThreadAction {
    OpenUrl(String),
    Profile(String),
    SearchDiscussion(u64),
    ToggleCollapse(u64),
    ToggleAll,
    ShowNewReplies(bool),
    ReadInContext(u64),
    Share,
    CopyCommentLink(u64),
    CopyCode(String),
    OpenContentLink(String),
    FocusReading,
    Hide,
    MuteDomain,
    Reply(u64),
    Vote(u64),
    LoadMore,
    Sort(ThreadSort),
    GoToItem(u64),
}

#[derive(Clone)]
enum RowKind {
    Header,
    Comment(ThreadRow),
    Empty,
    Footer,
}

#[derive(Clone)]
struct ConversationRow {
    key: usize,
    kind: RowKind,
    document: OnceCell<HtmlDocument>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FindMatch {
    pub id: u64,
    pub ordinal: usize,
    pub total: usize,
}

#[derive(PartialEq)]
struct ThreadAnchors {
    root: u64,
    indices: BTreeMap<u64, i32>,
    parents: BTreeMap<u64, u64>,
    /// Only matches, in display order. Context ancestors remain in the ordinary row index.
    matches: Vec<(i32, u64)>,
    current_match: Rc<Cell<Option<FindMatch>>>,
}

/// Comment rows already own their item data. Keep only the header/footer payload here,
/// rather than retaining another copy of every comment body in a downloaded thread.
struct ThreadContext {
    root: HnItem,
    poll_options: BTreeMap<u64, HnItem>,
    loaded_comments: usize,
    pending_items: usize,
    missing_items: usize,
    complete: bool,
    stale: bool,
    warning: Option<String>,
}

impl From<&ThreadSnapshot> for ThreadContext {
    fn from(snapshot: &ThreadSnapshot) -> Self {
        Self {
            root: snapshot.root.clone(),
            poll_options: snapshot
                .root
                .parts
                .iter()
                .filter_map(|id| snapshot.items.get(id).map(|item| (*id, item.clone())))
                .collect(),
            loaded_comments: snapshot
                .items
                .keys()
                .filter(|&&id| id != snapshot.root.id && !snapshot.root.parts.contains(&id))
                .count(),
            pending_items: snapshot.pending_ids.len(),
            missing_items: snapshot.missing_ids.len(),
            complete: snapshot.complete,
            stale: snapshot.stale,
            warning: snapshot.warning.clone(),
        }
    }
}
/// Retain only the current thread's local state in the virtualized template.
struct ReadingContext {
    hidden: bool,
    seen: BTreeSet<u64>,
    new_count: Option<usize>,
    show_dead: bool,
    typography: RenderOptions,
    find_foreground: Option<Rc<dyn IBrush>>,
}

impl PartialEq for ConversationRow {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

/// The parent must arrange this control into a finite-height viewport; putting it inside an
/// outer ScrollViewer would disable ListBox's VirtualizingStackPanel.
pub fn build(
    snapshot: &ThreadSnapshot,
    state: &ReaderState,
    query: &str,
    sort: ThreadSort,
    new_only: bool,
    on_action: Rc<dyn Fn(ThreadAction)>,
) -> Ref<Control> {
    let query = query.trim();
    let local = state.items.get(&snapshot.root.id);
    let options = ThreadOptions {
        collapsed: local.map(|s| s.collapsed.clone()).unwrap_or_default(),
        query: query.to_owned(),
        sort,
        show_dead: state.preferences.show_dead,
        ..Default::default()
    };
    let mut matched_documents = BTreeMap::new();
    let seen = local.map(|s| s.seen_comments.clone()).unwrap_or_default();
    // Find always searches the whole loaded discussion and restores the selected scope on close.
    let new_only = new_only && !seen.is_empty() && query.is_empty();
    let comments = if new_only {
        flatten_thread_since(snapshot, &options, &seen)
    } else if query.is_empty() {
        flatten_thread(snapshot, &options)
    } else {
        flatten_thread_with_match(snapshot, &options, |item, query| {
            if item
                .by
                .as_ref()
                .is_some_and(|author| author.to_lowercase().contains(query))
            {
                return true;
            }
            let document = HtmlDocument::parse(item.text.as_deref().unwrap_or_default());
            let matches = document.plain_text().to_lowercase().contains(query);
            // Reuse search parsing for the first matching rows, without retaining thousands
            // of parsed documents when a broad query matches a downloaded conversation.
            if matches && matched_documents.len() < 128 {
                matched_documents.insert(item.id, document);
            }
            matches
        })
    };
    let new_count = new_only.then(|| {
        comments
            .iter()
            .filter(|row| is_new_reply(row, &seen, options.show_dead))
            .count()
    });
    let mut rows = vec![Rc::new(ConversationRow {
        key: 0,
        kind: RowKind::Header,
        document: OnceCell::new(),
    })];
    let mut branches = Vec::new();
    let mut matches = Vec::new();
    for comment in comments {
        if comment.depth == 0 {
            branches.push((rows.len(), comment.id));
        }
        if comment.matches_query {
            matches.push((rows.len() as i32, comment.id));
        }
        let document = matched_documents
            .remove(&comment.id)
            .map(OnceCell::from)
            .unwrap_or_default();
        rows.push(Rc::new(ConversationRow {
            key: rows.len(),
            kind: RowKind::Comment(comment),
            document,
        }));
    }
    if rows.len() == 1 && !matches!(snapshot.root.kind, Some(ItemKind::Job | ItemKind::Pollopt)) {
        rows.push(Rc::new(ConversationRow {
            key: rows.len(),
            kind: RowKind::Empty,
            document: OnceCell::new(),
        }));
    }
    if !snapshot.complete || snapshot.warning.is_some() {
        rows.push(Rc::new(ConversationRow {
            key: rows.len(),
            kind: RowKind::Footer,
            document: OnceCell::new(),
        }));
    }
    let list = ListBox::new();
    list.classes().add("reader-list");
    list.classes().add("thread-list");
    list.set_padding(Thickness::new(14.0, 0.0, 14.0, 12.0));
    list.set_auto_scroll_to_selected_item(false);
    ScrollViewer::set_horizontal_scroll_bar_visibility_on(&list, ScrollBarVisibility::Disabled);
    ScrollViewer::set_vertical_scroll_bar_visibility_on(&list, ScrollBarVisibility::Auto);
    let indices = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| match &row.kind {
            RowKind::Comment(comment) => Some((comment.id, index as i32)),
            _ => None,
        })
        .collect();
    let mut parents = BTreeMap::new();
    for item in snapshot
        .items
        .values()
        .chain(std::iter::once(&snapshot.root))
    {
        for &child in &item.kids {
            parents.entry(child).or_insert(item.id);
        }
    }
    let current_match: Rc<Cell<Option<FindMatch>>> = Rc::default();
    list.set_tag(Some(Rc::new(ThreadAnchors {
        root: snapshot.root.id,
        indices,
        parents,
        matches,
        current_match: current_match.clone(),
    }) as BoxedValue));
    list.set_items_source(Some(ItemsSource::from_values(rows)));
    AutomationProperties::set_name(&list, Some("Hacker News conversation"));
    let snapshot = Rc::new(ThreadContext::from(snapshot));
    let reading = ReadingContext {
        hidden: local.is_some_and(|s| s.hidden),
        seen,
        new_count,
        show_dead: state.preferences.show_dead,
        typography: render_options(state),
        find_foreground: if query.is_empty() {
            None
        } else {
            brush("EmberSurface")
        },
    };
    let bulk_collapse = (query.is_empty() && !new_only && !snapshot.root.kids.is_empty())
        .then_some(!options.collapsed.is_empty());
    let query = query.to_lowercase();
    let weak_list = list.downgrade();
    let comment_actions: Rc<dyn Fn(ThreadAction)> = {
        let forward = on_action.clone();
        Rc::new(move |action| {
            if let ThreadAction::GoToItem(id) = action {
                if let Some(list) = weak_list.upgrade() {
                    if let Some((index, _)) = anchor_index(&list, id) {
                        reveal_index(&list, index, 0, true);
                        return;
                    }
                }
            }
            forward(action);
        })
    };
    list.set_item_template(Some(FuncDataTemplate::for_type::<Rc<ConversationRow>>(
        move |row, _| {
            let control = match &row.kind {
                RowKind::Header => header(
                    &snapshot,
                    &reading,
                    sort,
                    bulk_collapse,
                    query.is_empty(),
                    &on_action,
                ),
                RowKind::Comment(comment) => {
                    let control = comment_row(
                        comment,
                        &row.document,
                        &snapshot.root,
                        &reading,
                        &query,
                        BranchNeighbors::at(row.key, &branches),
                        &comment_actions,
                    );
                    if comment.matches_query {
                        control.classes().add("find-result");
                        mark_find_result(
                            &control,
                            current_match.get().filter(|m| m.id == comment.id),
                        );
                    }
                    control
                }
                RowKind::Empty => empty(&snapshot, &query, new_only, &on_action),
                RowKind::Footer => footer(&snapshot, &on_action),
            };
            // Stretch into narrow panes, cap the line length on wide monitors.
            control.set_max_width((reading.typography.font_size * 44.0).clamp(560.0, 840.0));
            Some(control)
        },
        // Native containers are virtualized; document controls are rebuilt for a different
        // item so a recycled row can never retain another comment's action closures.
        false,
    )));
    list.upcast()
}

/// Select or step through matching comments without rebuilding the list. The caller
/// keeps the stable comment ID across refreshes and sort changes. A missing preferred
/// result starts at the first match, or the last match when stepping backward.
pub(crate) fn select_find_match(
    control: &Control,
    preferred: Option<u64>,
    direction: i32,
) -> Option<FindMatch> {
    let list = conversation_list(control)?;
    let tag = list.tag()?;
    let anchors = tag.downcast_ref::<ThreadAnchors>()?;
    let total = anchors.matches.len();
    if total == 0 {
        return None;
    }
    let previous = preferred.and_then(|id| {
        let index = anchors.indices.get(&id)?;
        anchors
            .matches
            .binary_search_by_key(index, |&(index, _)| index)
            .ok()
    });
    let index = match (previous, direction.signum()) {
        (Some(0) | None, -1) => total - 1,
        (Some(n), -1) => n - 1,
        (Some(n), 1) => (n + 1) % total,
        (Some(n), _) => n,
        (None, _) => 0,
    };
    let (row, id) = anchors.matches[index];
    let selected = FindMatch {
        id,
        ordinal: index + 1,
        total,
    };
    if let Some(old) = anchors
        .current_match
        .replace(Some(selected))
        .filter(|m| m.id != id)
    {
        update_realized_match(&list, anchors.matches[old.ordinal - 1].0, None);
    }
    update_realized_match(&list, row, Some(selected));
    // The template reads current_match when a later scroll first realizes this row.
    reveal_index(&list, row, 0, false);
    Some(selected)
}

fn update_realized_match(list: &ListBox, index: i32, result: Option<FindMatch>) {
    if let Some(container) = list.container_from_index(index) {
        if let Some(row) = container.get_visual_descendants().find_map(|node| {
            node.cast::<Control>()
                .filter(|c| c.classes().contains("find-result"))
        }) {
            mark_find_result(&row, result);
        }
    }
}

fn mark_find_result(row: &Control, result: Option<FindMatch>) {
    row.classes().set("current-find-result", result.is_some());
    let status = result.map(|m| format!("Search result {} of {}", m.ordinal, m.total));
    AutomationProperties::set_item_status(row, status.as_deref());
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadingPosition {
    pub item: u64,
    /// Distance from the top of the row in logical pixels.
    pub inset: u32,
}

impl ReadingPosition {
    pub fn start(item: u64) -> Self {
        Self { item, inset: 0 }
    }
}

/// Capture the first intersecting row and how far it has scrolled. A single
/// long comment or a text-only story header can span several screens.
pub(crate) fn viewport_position(control: &Control) -> Option<ReadingPosition> {
    let list = conversation_list(control)?;
    let tag = list.tag()?;
    let root = tag.downcast_ref::<ThreadAnchors>()?.root;
    let scroll = list.scroll()?.cast::<ScrollViewer>()?;
    let height = scroll.viewport().height;
    list.get_realized_containers()
        .into_iter()
        .filter_map(|container| {
            if !container.is_effectively_visible() {
                return None;
            }
            let origin = container.translate_point(Point::new(0.0, 0.0), &scroll)?;
            if origin.y >= height || origin.y + container.bounds().height <= 0.0 {
                return None;
            }
            let item = list.item_from_container(&container)?;
            let row = item.downcast_ref::<Rc<ConversationRow>>()?;
            match &row.kind {
                RowKind::Header => Some((origin.y, root)),
                RowKind::Comment(comment) => Some((origin.y, comment.id)),
                _ => None,
            }
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(top, item)| ReadingPosition {
            item,
            inset: (-top).max(0.0).round() as u32,
        })
}

/// Return the displayed path, including the result itself when its text was folded.
pub(crate) fn displayed_comment_path(control: &Control, mut id: u64) -> Option<Vec<u64>> {
    let list = conversation_list(control)?;
    let tag = list.tag()?;
    let anchors = tag.downcast_ref::<ThreadAnchors>()?;
    let mut path = Vec::new();
    while id != anchors.root {
        let index = usize::try_from(*anchors.indices.get(&id)?).ok()?;
        let item = list.items().get_at(index)?;
        let row = item.downcast_ref::<Rc<ConversationRow>>()?;
        let RowKind::Comment(comment) = &row.kind else {
            return None;
        };
        path.push(id);
        // Filtered rows include the entire ancestry of each hit. Follow the actual
        // display tree even if a malformed snapshot lists a child under two parents.
        id = comment.parent_id;
    }
    Some(path)
}

/// Restores reading context after rebuilding a thread. If an ancestor was collapsed,
/// the nearest visible ancestor is used. A filtered-out branch falls back to its root.
/// Calling this before attaching the newly built control is supported.
pub(crate) fn restore_position(control: &Control, position: ReadingPosition, focus_row: bool) {
    let Some(list) = conversation_list(control) else {
        return;
    };
    let Some((index, exact)) = anchor_index(&list, position.item) else {
        return;
    };
    // When the former row is folded away, show its ancestor from the start.
    let inset = if exact { position.inset } else { 0 };
    reveal_index(&list, index, inset, focus_row);
}

fn reveal_index(list: &Ref<ListBox>, index: i32, inset: u32, focus_row: bool) {
    let reveal = move |list: &ListBox| {
        scroll_anchor_into_view(list, index, inset);
        if focus_row {
            if let Some(container) = list.container_from_index(index) {
                container.focus();
            }
        }
    };
    if list.is_loaded() {
        let weak = list.downgrade();
        Dispatcher::ui_thread().post_local(
            move || {
                if let Some(list) = weak.upgrade().filter(|list| list.is_loaded()) {
                    reveal(&list);
                }
            },
            DispatcherPriority::LOADED,
        );
    } else {
        let weak = list.downgrade();
        let restored = Cell::new(false);
        list.loaded(move |_, _| {
            if !restored.replace(true) {
                if let Some(list) = weak.upgrade() {
                    reveal(&list);
                }
            }
        });
    }
}

/// Rebuilds replace both virtual containers and their controls. Look up a disclosure by
/// comment ID so a recycled row cannot receive focus on behalf of its former item.
pub(crate) fn focus_thread_tool(control: &Control, item: Option<u64>, name: &'static str) {
    let Some(list) = conversation_list(control) else {
        return;
    };
    let weak = list.downgrade();
    Dispatcher::ui_thread().post_local(
        move || {
            let Some(list) = weak.upgrade().filter(|list| list.is_loaded()) else {
                return;
            };
            let index = match item {
                None => 0,
                Some(id) => {
                    let Some(index) = list.tag().and_then(|tag| {
                        tag.downcast_ref::<ThreadAnchors>()?
                            .indices
                            .get(&id)
                            .copied()
                    }) else {
                        return;
                    };
                    index
                }
            };
            if list.container_from_index(index).is_none() {
                list.scroll_into_view(index);
            }
            if let Some(row) = list.container_from_index(index) {
                if let Some(tool) = row
                    .get_visual_descendants()
                    .filter_map(|v| v.cast::<Control>())
                    .find(|tool| tool.name().as_deref() == Some(name))
                {
                    tool.focus();
                }
            }
        },
        DispatcherPriority::LOADED,
    );
}

fn conversation_list(control: &Control) -> Option<Ref<ListBox>> {
    control
        .to_ref()
        .cast::<ListBox>()
        .filter(|list| list.classes().contains("thread-list"))
        .or_else(|| {
            control
                .get_visual_descendants()
                .filter_map(|node| node.cast::<ListBox>())
                .find(|list| list.classes().contains("thread-list"))
        })
}

fn anchor_index(list: &ListBox, target: u64) -> Option<(i32, bool)> {
    let mut id = target;
    let tag = list.tag()?;
    let anchors = tag.downcast_ref::<ThreadAnchors>()?;
    let mut seen = BTreeSet::new();
    while seen.insert(id) {
        if id == anchors.root {
            return Some((0, target == id));
        }
        if let Some(&index) = anchors.indices.get(&id) {
            return Some((index, target == id));
        }
        id = *anchors.parents.get(&id)?;
    }
    None
}

fn scroll_anchor_into_view(list: &ListBox, index: i32, inset: u32) {
    list.scroll_into_view(index);
    if let (Some(container), Some(scroll)) = (
        list.container_from_index(index),
        list.scroll()
            .and_then(|scroll| scroll.cast::<ScrollViewer>()),
    ) {
        // VirtualizingStackPanel uses physical scroll offsets. Align the restored row
        // at the top instead of merely making its final pixel visible at the bottom.
        if let Some(origin) = container.translate_point(Point::new(0.0, 0.0), &scroll) {
            let offset = scroll.offset();
            let inset = f64::from(inset).min((container.bounds().height - 1.0).max(0.0));
            scroll.set_offset(Vector::new(
                offset.x,
                (offset.y + origin.y + inset).max(0.0),
            ));
        }
    }
}

/// The pinned detail header stays meaningful for standalone comments and poll options too.
pub(crate) fn discussion_summary(snapshot: &ThreadSnapshot) -> String {
    let root = &snapshot.root;
    match root.kind {
        Some(ItemKind::Job) => return "Job listing".to_owned(),
        Some(ItemKind::Pollopt) => {
            return root
                .score
                .and_then(|n| u64::try_from(n).ok())
                .map(vote_count)
                .unwrap_or_else(|| "Poll result".to_owned());
        }
        _ => {}
    }
    let count = root.descendants.unwrap_or_else(|| {
        snapshot
            .items
            .values()
            .filter(|item| item.id != root.id && item.kind == Some(ItemKind::Comment))
            .count() as u64
    });
    let noun = match (root.kind == Some(ItemKind::Comment), count == 1) {
        (true, true) => "reply",
        (true, false) => "replies",
        (false, true) => "comment",
        (false, false) => "comments",
    };
    if root.descendants.is_none() && !snapshot.complete {
        format!("{count} {noun} loaded")
    } else {
        format!("{count} {noun}")
    }
}

pub(crate) fn discussion_title(story: &HnItem, show_dead: bool) -> String {
    if story.deleted {
        "[deleted]".to_owned()
    } else if story.dead && !show_dead {
        "[dead]".to_owned()
    } else if let Some(title) = story
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
    {
        HtmlDocument::parse(title).plain_text()
    } else {
        match story.kind {
            Some(ItemKind::Comment) => story
                .by
                .as_ref()
                .map(|author| format!("Comment by {author}"))
                .unwrap_or_else(|| "Comment".to_owned()),
            Some(ItemKind::Pollopt) => "Poll option".to_owned(),
            _ => story.title_text().to_owned(),
        }
    }
}

fn header(
    snapshot: &ThreadContext,
    reading: &ReadingContext,
    sort: ThreadSort,
    bulk_collapse: Option<bool>,
    show_scope: bool,
    actions: &Rc<dyn Fn(ThreadAction)>,
) -> Ref<Control> {
    let story = &snapshot.root;
    let options = &reading.typography;
    let body = widgets::stack(8.0);
    body.set_margin(Thickness::new(0.0, 12.0, 0.0, 4.0));
    let source = if story.article_url().is_some() {
        story.domain()
    } else {
        match story.kind {
            Some(ItemKind::Comment) => "Hacker News comment",
            Some(ItemKind::Poll | ItemKind::Pollopt) => "Hacker News poll",
            Some(ItemKind::Job) => "Hacker News job",
            _ => "Hacker News",
        }
        .to_owned()
    };
    let eyebrow = label(&source, 12.0);
    eyebrow.set_font_weight(FontWeight::SemiBold);
    bind_resource(
        &eyebrow,
        TextBlock::foreground_property().as_property(),
        "EmberAccentInk",
    );
    body.children().add(eyebrow);

    let headline = discussion_title(story, reading.show_dead);
    let title = label(&headline, (options.font_size * 1.55).clamp(23.0, 36.0));
    AutomationProperties::set_heading_level(&title, 1);
    title.set_font_weight(FontWeight::SemiBold);
    title.set_line_height(title.font_size() * 1.24);
    AutomationProperties::set_name(&title, Some(&headline));
    body.children().add(title);

    let meta = WrapPanel::new();
    if let Some(author) = story.by.as_ref().filter(|_| !story.deleted) {
        meta.children().add(author_button(author, actions));
    }
    let mut facts = Vec::new();
    if story.time.is_some() {
        facts.push(relative_time(story.time, crate::services::now()));
    }
    if let Some(score) = story.score {
        if story.kind == Some(ItemKind::Pollopt) {
            if let Ok(votes) = u64::try_from(score) {
                facts.push(vote_count(votes));
            }
        } else {
            facts.push(format!(
                "{score} {}",
                if score == 1 { "point" } else { "points" }
            ));
        }
    }
    if !facts.is_empty() {
        let metadata = muted(&facts.join(" · "));
        metadata.set_vertical_alignment(VerticalAlignment::Center);
        metadata.set_margin(Thickness::symmetric(4.0, 0.0));
        meta.children().add(metadata);
    }
    body.children().add(meta);

    let primary = WrapPanel::new();
    if let Some(url) = story.article_url() {
        primary.children().add(labelled_action(
            "Read article",
            "open",
            ThreadAction::OpenUrl(url),
            actions,
        ));
    }
    if let Some(parent) = story.parent {
        primary.children().add(labelled_action(
            "View parent",
            "up",
            ThreadAction::GoToItem(parent),
            actions,
        ));
    } else if let Some(poll) = story.poll {
        primary.children().add(labelled_action(
            "View poll",
            "up",
            ThreadAction::GoToItem(poll),
            actions,
        ));
    }
    primary.children().add(icon_action(
        "Copy discussion link",
        "copy",
        ThreadAction::Share,
        actions,
    ));
    let more = widgets::icon_button("More story actions", "more");
    let menu = MenuFlyout::new();
    menu_action(
        &menu,
        "Focus reading",
        "focus_reading",
        ThreadAction::FocusReading,
        actions,
    );
    if matches!(story.kind, Some(ItemKind::Story | ItemKind::Poll)) {
        menu_action(
            &menu,
            "Search this discussion",
            "search",
            ThreadAction::SearchDiscussion(story.id),
            actions,
        );
    }
    menu_action(
        &menu,
        "Open discussion on HN",
        "open",
        ThreadAction::OpenUrl(story.discussion_url()),
        actions,
    );
    if !matches!(story.kind, Some(ItemKind::Job | ItemKind::Pollopt))
        && !story.deleted
        && !story.dead
    {
        menu_action(
            &menu,
            "Vote on HN",
            "up",
            ThreadAction::Vote(story.id),
            actions,
        );
        menu_action(
            &menu,
            "Reply on HN",
            "reply",
            ThreadAction::Reply(story.id),
            actions,
        );
    }
    menu_action(
        &menu,
        if reading.hidden {
            "Restore to feed"
        } else {
            "Hide story"
        },
        "eye_off",
        ThreadAction::Hide,
        actions,
    );
    if story.article_url().is_some() && story.domain() != "news.ycombinator.com" {
        menu_action(
            &menu,
            &format!("Mute {}", story.domain()),
            "eye_off",
            ThreadAction::MuteDomain,
            actions,
        );
    }
    more.set_flyout(menu);
    primary.children().add(more);
    body.children().add(primary);

    if story.deleted {
        body.children()
            .add(muted("This item was deleted on Hacker News."));
    } else if story.dead && !reading.show_dead {
        body.children().add(muted("This item is marked dead on Hacker News. Enable dead items in Reader settings to show its content."));
    } else if let Some(html) = &story.text {
        if !html.trim().is_empty() {
            body.children().add(rich_document(
                &HtmlDocument::parse(html),
                options,
                None,
                actions,
            ));
        }
    }
    if !story.parts.is_empty() && !story.deleted && (!story.dead || reading.show_dead) {
        body.children()
            .add(poll_options(snapshot, reading, actions));
    }
    if matches!(story.kind, Some(ItemKind::Job | ItemKind::Pollopt)) && story.kids.is_empty() {
        return body.upcast();
    }
    let separator = Border::new();
    separator.set_height(1.0);
    bind_resource(
        &separator,
        Border::background_property().as_property(),
        "EmberBorder",
    );
    body.children().add(separator);

    let sort_row = WrapPanel::new();
    let caption = if let Some(count) = reading.new_count {
        format!(
            "{count} new {}",
            if count == 1 { "reply" } else { "replies" }
        )
    } else {
        story
            .descendants
            .map(|n| format!("{n} {}", if n == 1 { "comment" } else { "comments" }))
            .unwrap_or_else(|| {
                if story.kind == Some(ItemKind::Comment) {
                    "Replies"
                } else {
                    "Conversation"
                }
                .to_owned()
            })
    };
    let heading = label(&caption, 17.0);
    AutomationProperties::set_heading_level(&heading, 2);
    heading.set_font_weight(FontWeight::SemiBold);
    heading.set_vertical_alignment(VerticalAlignment::Center);
    heading.set_margin(Thickness::new(0.0, 4.0, 12.0, 4.0));
    sort_row.children().add(heading);
    if !story.kids.is_empty() {
        sort_row.children().add(sort_control(sort, actions));
    }
    if let Some(expand) = bulk_collapse {
        let (label, icon) = if expand {
            ("Expand all replies", "chevron_down")
        } else {
            ("Collapse all replies", "collapse")
        };
        let button = icon_action(label, icon, ThreadAction::ToggleAll, actions);
        button.set_name(Some("FoldAll".to_owned()));
        sort_row.children().add(button);
    }
    if show_scope
        && !reading.seen.is_empty()
        && (reading.new_count.is_some() || !story.kids.is_empty())
    {
        let filter = ferroui_controls::primitives::ToggleButton::new();
        filter.set_content(Some(Rc::new("New replies".to_owned())));
        filter.classes().add("feed-filter");
        filter.classes().set("active", reading.new_count.is_some());
        filter.set_name(Some("NewReplyFilter".to_owned()));
        filter.set_min_height(44.0);
        filter.set_is_checked(Some(reading.new_count.is_some()));
        AutomationProperties::set_name(&filter, Some("New replies since last visit"));
        ToolTip::set_tip(
            &filter,
            Some(Rc::new(
                "Show loaded replies absent from your previous copy".to_owned(),
            )),
        );
        let actions = actions.clone();
        let enabled = reading.new_count.is_none();
        filter.click(move |_, _| actions(ThreadAction::ShowNewReplies(enabled)));
        sort_row.children().add(filter);
    }
    body.children().add(sort_row);
    if snapshot.stale {
        body.children().add(muted(
            "Showing a saved copy. Refresh to check for new replies.",
        ));
    }
    body.upcast()
}

fn sort_control(sort: ThreadSort, actions: &Rc<dyn Fn(ThreadAction)>) -> Ref<ComboBox> {
    let sort_picker = ComboBox::new();
    sort_picker.set_name(Some("ThreadSort".to_owned()));
    sort_picker.set_items_source(Some(ItemsSource::from_values([
        "Ranked".to_owned(),
        "Newest first".to_owned(),
        "Oldest first".to_owned(),
    ])));
    sort_picker.set_selected_index(match sort {
        ThreadSort::Ranked => 0,
        ThreadSort::Newest => 1,
        ThreadSort::Oldest => 2,
    });
    sort_picker.set_min_height(44.0);
    sort_picker.set_min_width(128.0);
    AutomationProperties::set_name(&sort_picker, Some("Comment sort order"));
    let weak = sort_picker.downgrade();
    let sort_action = actions.clone();
    sort_picker.selection_changed(move |_, _| {
        if let Some(picker) = weak.upgrade() {
            let value = match picker.selected_index() {
                1 => ThreadSort::Newest,
                2 => ThreadSort::Oldest,
                _ => ThreadSort::Ranked,
            };
            sort_action(ThreadAction::Sort(value));
        }
    });
    sort_picker
}

#[derive(Clone, Copy, Default)]
struct BranchNeighbors {
    previous: Option<u64>,
    next: Option<u64>,
}

impl BranchNeighbors {
    /// Top-level rows follow the active sort and Find filter. Descendants share their
    /// branch’s neighbors, so jumping forward skips the remainder of a long exchange.
    fn at(row: usize, branches: &[(usize, u64)]) -> Self {
        let next = branches.partition_point(|&(index, _)| index <= row);
        Self {
            previous: next.checked_sub(2).map(|index| branches[index].1),
            next: branches.get(next).map(|&(_, id)| id),
        }
    }
}

fn is_new_reply(row: &ThreadRow, previous: &BTreeSet<u64>, show_dead: bool) -> bool {
    !previous.is_empty()
        && !previous.contains(&row.id)
        && row
            .item
            .as_ref()
            .is_some_and(|item| !item.deleted && (!item.dead || show_dead))
}

fn comment_row(
    row: &ThreadRow,
    document: &OnceCell<HtmlDocument>,
    root: &HnItem,
    reading: &ReadingContext,
    query: &str,
    neighbors: BranchNeighbors,
    actions: &Rc<dyn Fn(ThreadAction)>,
) -> Ref<Control> {
    let searching = !query.is_empty();
    let outer = Border::new();
    outer.set_border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0));
    bind_resource(
        &outer,
        Border::border_brush_property().as_property(),
        "EmberBorder",
    );
    let body = widgets::stack(3.0);
    body.set_margin(Thickness::new(0.0, 2.0, 0.0, 10.0));
    let line = Grid::new();
    line.set_column_definitions(ColumnDefinitions::parse("*,Auto").expect("constant columns"));
    let byline = WrapPanel::new();
    byline.set_vertical_alignment(VerticalAlignment::Center);
    if let Some(item) = &row.item {
        if let Some(author) = item.by.as_ref().filter(|_| !item.deleted) {
            let button = author_button(author, actions);
            if row.matches_query && author.to_lowercase().contains(query) {
                button.classes().add("active");
                AutomationProperties::set_item_status(&button, Some("Author matches search"));
            }
            byline.children().add(button);
            if root.by.as_ref() == Some(author) {
                byline.children().add(badge("OP"));
            }
        }
        // A timestamp doubles as the stable comment link, as on Hacker News.
        let caption = if item.time.is_some() {
            relative_time(item.time, crate::services::now())
        } else {
            "Link".to_owned()
        };
        let permalink = text_action(
            &caption,
            "Permalink",
            ThreadAction::OpenUrl(ember_core::item_url(row.id)),
            actions,
        );
        byline.children().add(permalink);
        if is_new_reply(row, &reading.seen, reading.show_dead) {
            byline.children().add(badge("NEW"));
        } else if reading.new_count.is_some() {
            let context = muted("Earlier reply");
            context.set_vertical_alignment(VerticalAlignment::Center);
            byline.children().add(context);
        }
        if row.matches_query {
            byline.children().add(badge("MATCH"));
        }
    } else {
        byline.children().add(muted(&format!("Comment {}", row.id)));
    }
    line.children().add(byline);
    let commands = widgets::stack(0.0);
    commands.set_orientation(Orientation::Horizontal);
    Grid::set_column(&commands, 1);
    if reading.new_count.is_none() && !searching && (row.child_count > 0 || row.collapsed) {
        let count = format!(
            "{} {}",
            row.child_count,
            if row.child_count == 1 {
                "reply"
            } else {
                "replies"
            }
        );
        let action = ThreadAction::ToggleCollapse(row.id);
        let disclosure = if row.collapsed {
            let caption = if row.child_count == 0 {
                "Show text"
            } else {
                &count
            };
            let expand = labelled_action(caption, "chevron_down", action, actions);
            let name = if row.child_count == 0 {
                "Expand comment".to_owned()
            } else {
                format!("Expand comment and {count}")
            };
            AutomationProperties::set_name(&expand, Some(&name));
            ToolTip::set_tip(&expand, Some(Rc::new(name)));
            expand
        } else {
            icon_action(&format!("Collapse {count}"), "chevron_up", action, actions)
        };
        disclosure.set_name(Some("FoldReply".to_owned()));
        commands.children().add(disclosure);
    }
    if row.matches_query
        || (reading.new_count.is_some() && is_new_reply(row, &reading.seen, reading.show_dead))
    {
        let context = icon_action(
            "Read in context",
            "chat",
            ThreadAction::ReadInContext(row.id),
            actions,
        );
        context.set_name(Some("ReadInContext".to_owned()));
        commands.children().add(context);
    }
    commands
        .children()
        .add(comment_menu(row, root.id, neighbors, actions));
    line.children().add(commands);
    body.children().add(line);

    if row.collapsed && reading.new_count.is_none() && !searching {
        let summary = match &row.item {
            Some(item) if item.deleted => "[deleted]".to_owned(),
            Some(item) if item.dead && !reading.show_dead => "[dead]".to_owned(),
            Some(item) => item
                .text
                .as_deref()
                .map(|text| {
                    document
                        .get_or_init(|| HtmlDocument::parse(text))
                        .plain_text()
                })
                .unwrap_or_else(|| "Collapsed conversation".to_owned()),
            None => "Collapsed conversation".to_owned(),
        };
        let mut chars = summary
            .split_whitespace()
            .flat_map(|word| word.chars().chain(std::iter::once(' ')));
        let mut snippet: String = chars.by_ref().take(120).collect();
        if chars.next().is_some() {
            snippet.push('…');
        }
        let preview = muted(snippet.trim_end());
        preview.set_max_lines(1);
        preview.set_text_trimming(<dyn TextTrimming>::character_ellipsis());
        body.children().add(preview);
        AutomationProperties::set_item_status(&outer, Some("Collapsed"));
    } else {
        match &row.item {
            None => body.children().add(muted(
                "This comment is not available in the local copy yet.",
            )),
            Some(item) if item.deleted => body
                .children()
                .add(muted("[deleted] — replies remain available below.")),
            Some(item) if item.dead && !reading.show_dead => body.children().add(muted(
                "[dead] — enable dead comments in Reader settings to show this text.",
            )),
            Some(item) => {
                if let Some(html) = item.text.as_deref().filter(|s| !s.trim().is_empty()) {
                    let highlight = if row.matches_query {
                        reading
                            .find_foreground
                            .as_ref()
                            .zip(reading.typography.link_brush.as_ref())
                            .map(|(foreground, background)| FindHighlight {
                                query,
                                foreground: foreground.clone(),
                                background: background.clone(),
                            })
                    } else {
                        None
                    };
                    body.children().add(rich_document(
                        document.get_or_init(|| HtmlDocument::parse(html)),
                        &reading.typography,
                        highlight,
                        actions,
                    ));
                } else {
                    body.children().add(muted("No comment text is available."));
                }
            }
        }
    }

    if row.item.is_none() {
        body.children().add(labelled_action(
            "Retry loading",
            "refresh",
            ThreadAction::LoadMore,
            actions,
        ));
    }
    AutomationProperties::set_name(
        &outer,
        Some(&format!(
            "Comment by {}, level {}",
            row.item
                .as_ref()
                .map(HnItem::author)
                .unwrap_or("unavailable"),
            row.depth + 1
        )),
    );
    let branch = Border::new();
    // Preserve reading width on phones, even when HN threads are dozens of levels deep.
    let inset = row.depth.min(4) as f64 * 8.0;
    branch.set_margin(Thickness::new(inset, 0.0, 0.0, 0.0));
    if row.depth > 0 || row.matches_query {
        branch.set_padding(Thickness::new(8.0, 0.0, 0.0, 0.0));
        branch.set_border_thickness(Thickness::new(2.0, 0.0, 0.0, 0.0));
        bind_resource(
            &branch,
            Border::border_brush_property().as_property(),
            if row.matches_query {
                "EmberAccentInk"
            } else {
                "EmberBorder"
            },
        );
    }
    branch.set_child(body);
    outer.set_child(branch);
    outer.upcast()
}

/// Populate on first open so scrolling a long thread need only realize the overflow trigger.
fn comment_menu(
    row: &ThreadRow,
    root: u64,
    neighbors: BranchNeighbors,
    actions: &Rc<dyn Fn(ThreadAction)>,
) -> Ref<Button> {
    let more = widgets::icon_button("More comment actions", "more");
    let menu = MenuFlyout::new();
    let weak = menu.downgrade();
    let actions = actions.clone();
    let id = row.id;
    let can_reply = row
        .item
        .as_ref()
        .is_some_and(|item| !item.deleted && !item.dead);
    let parent = (row.depth > 0).then_some(row.parent_id);
    menu.opening(move |_| {
        let Some(menu) = weak.upgrade() else {
            return;
        };
        if menu.items().count() != 0 {
            return;
        }
        menu_action(
            &menu,
            "Copy comment link",
            "copy",
            ThreadAction::CopyCommentLink(id),
            &actions,
        );
        if can_reply {
            menu_action(
                &menu,
                "Reply on HN",
                "reply",
                ThreadAction::Reply(id),
                &actions,
            );
        }
        if let Some(parent) = parent {
            menu_action(
                &menu,
                "Go to parent comment",
                "up",
                ThreadAction::GoToItem(parent),
                &actions,
            );
        }
        if let Some(previous) = neighbors.previous {
            menu_action(
                &menu,
                "Previous top-level comment",
                "chevron_up",
                ThreadAction::GoToItem(previous),
                &actions,
            );
        }
        if let Some(next) = neighbors.next {
            menu_action(
                &menu,
                "Next top-level comment",
                "chevron_down",
                ThreadAction::GoToItem(next),
                &actions,
            );
        }
        menu_action(
            &menu,
            "Back to story",
            "back",
            ThreadAction::GoToItem(root),
            &actions,
        );
    });
    more.set_flyout(menu);
    more
}

fn poll_options(
    snapshot: &ThreadContext,
    reading: &ReadingContext,
    actions: &Rc<dyn Fn(ThreadAction)>,
) -> Ref<Control> {
    let panel = widgets::stack(8.0);
    let title = label("Poll results", 17.0);
    title.set_font_weight(FontWeight::SemiBold);
    AutomationProperties::set_heading_level(&title, 2);
    panel.children().add(title);

    // A share is meaningful only after every option has a reported vote count.
    let total: Option<u64> = snapshot.root.parts.iter().try_fold(0u64, |total, id| {
        let votes = u64::try_from(snapshot.poll_options.get(id)?.score?).ok()?;
        total.checked_add(votes)
    });
    let caption = match total {
        Some(0) => "No votes yet".to_owned(),
        Some(votes) => format!("{} in total", vote_count(votes)),
        None => "Some poll results are unavailable.".to_owned(),
    };
    panel.children().add(muted(&caption));
    for (index, id) in snapshot.root.parts.iter().enumerate() {
        let Some(option) = snapshot.poll_options.get(id) else {
            panel
                .children()
                .add(muted(&format!("Option {} is unavailable.", index + 1)));
            continue;
        };
        let item = widgets::stack(3.0);
        let text = if option.deleted {
            "[deleted]".to_owned()
        } else if option.dead && !reading.show_dead {
            "[dead]".to_owned()
        } else {
            HtmlDocument::parse(
                option
                    .text
                    .as_deref()
                    .or(option.title.as_deref())
                    .unwrap_or_default(),
            )
            .plain_text()
        };
        let text = if text.trim().is_empty() {
            format!("Option {}", index + 1)
        } else {
            text
        };
        item.children()
            .add(label(&text, reading.typography.font_size));
        if let Some(votes) = option.score.and_then(|score| u64::try_from(score).ok()) {
            let count = vote_count(votes);
            if let Some(total) = total.filter(|&total| total > 0) {
                let share = 100.0 * votes as f64 / total as f64;
                let percentage = if votes > 0 && share < 1.0 {
                    "<1%".to_owned()
                } else if votes < total && share > 99.0 {
                    ">99%".to_owned()
                } else {
                    format!("{share:.0}%")
                };
                let info = format!("{count} · {percentage}");
                let progress = ProgressBar::new();
                progress.set_maximum(total as f64);
                progress.set_range_value(votes as f64);
                progress.set_height(4.0);
                AutomationProperties::set_name(&progress, Some(&format!("{text}, {info}")));
                item.children().add(progress);
                item.children().add(muted(&info));
            } else {
                item.children().add(muted(&count));
            }
        } else {
            item.children().add(muted("Vote count unavailable"));
        }
        panel.children().add(item);
    }
    panel.children().add(labelled_action(
        "Vote in poll on HN",
        "up",
        ThreadAction::OpenUrl(snapshot.root.discussion_url()),
        actions,
    ));
    panel.upcast()
}

fn vote_count(votes: u64) -> String {
    format!("{votes} {}", if votes == 1 { "vote" } else { "votes" })
}

fn footer(snapshot: &ThreadContext, actions: &Rc<dyn Fn(ThreadAction)>) -> Ref<Control> {
    let panel = widgets::stack(6.0);
    panel.set_margin(Thickness::new(0.0, 8.0, 0.0, 12.0));
    if !snapshot.complete {
        let mut facts = vec![format!(
            "{} {} loaded",
            snapshot.loaded_comments,
            if snapshot.loaded_comments == 1 {
                "comment"
            } else {
                "comments"
            }
        )];
        if snapshot.pending_items != 0 {
            facts.push(format!("{} pending", snapshot.pending_items));
        }
        if snapshot.missing_items != 0 {
            facts.push(format!("{} unavailable", snapshot.missing_items));
        }
        panel.children().add(muted(&facts.join(" · ")));
    }
    if let Some(warning) = &snapshot.warning {
        panel.children().add(muted(warning));
    }
    if !snapshot.complete {
        let (label, icon) = if snapshot.pending_items == 0 {
            ("Retry unavailable items", "refresh")
        } else if snapshot.root.parts.is_empty() {
            ("Load more comments", "chevron_down")
        } else {
            ("Load more items", "chevron_down")
        };
        panel.children().add(labelled_action(
            label,
            icon,
            ThreadAction::LoadMore,
            actions,
        ));
    }
    panel.upcast()
}

fn empty(
    snapshot: &ThreadContext,
    query: &str,
    new_only: bool,
    actions: &Rc<dyn Fn(ThreadAction)>,
) -> Ref<Control> {
    let panel = StackPanel::new();
    panel.set_spacing(12.0);
    panel.set_margin(Thickness::uniform(20.0));
    if !query.is_empty() {
        panel.children().add(label("No matching comments", 17.0));
        panel.children().add(muted(&format!(
            "No loaded comments contain “{query}”. Edit or clear the search above."
        )));
    } else if new_only {
        panel
            .children()
            .add(label("No new replies in this copy", 17.0));
        panel.children().add(labelled_action(
            "Show all replies",
            "chat",
            ThreadAction::ShowNewReplies(false),
            actions,
        ));
    } else if !snapshot.complete && !snapshot.root.kids.is_empty() {
        panel
            .children()
            .add(label("Comments haven’t loaded yet", 17.0));
        panel.children().add(muted(
            "Use the action below to continue loading this discussion.",
        ));
    } else {
        panel.children().add(label("No comments yet", 17.0));
        if !snapshot.root.deleted && !snapshot.root.dead {
            panel.children().add(labelled_action(
                "Reply on Hacker News",
                "reply",
                ThreadAction::Reply(snapshot.root.id),
                actions,
            ));
        }
    }
    panel.upcast()
}

fn rich_document(
    document: &HtmlDocument,
    options: &RenderOptions,
    highlight: Option<FindHighlight<'_>>,
    actions: &Rc<dyn Fn(ThreadAction)>,
) -> Ref<Control> {
    let open = actions.clone();
    let copy = actions.clone();
    crate::rich_content::render(
        document,
        options,
        highlight,
        Rc::new(move |url| open(ThreadAction::OpenContentLink(url.to_owned()))),
        Rc::new(move |source| copy(ThreadAction::CopyCode(source.to_owned()))),
    )
}

/// Shared typography, palette and touch metrics for comments, story bodies and biographies.
pub fn render_options(state: &ReaderState) -> RenderOptions {
    RenderOptions {
        font_size: state.preferences.font_size,
        line_height: state.preferences.font_size * state.preferences.line_height,
        foreground: brush("EmberText"),
        link_brush: brush("EmberAccentInk").or_else(|| brush("HyperlinkButtonForeground")),
        code_background: brush("EmberCode"),
        quote_brush: brush("EmberMuted"),
        link_min_height: 44.0,
        max_link_width: 260.0,
        ..Default::default()
    }
}

fn author_button(author: &str, actions: &Rc<dyn Fn(ThreadAction)>) -> Ref<Button> {
    let button = text_action(
        author,
        &format!("View {author}’s profile"),
        ThreadAction::Profile(author.to_owned()),
        actions,
    );
    button.set_font_weight(FontWeight::SemiBold);
    button
}

fn text_action(
    caption: &str,
    name: &str,
    action: ThreadAction,
    actions: &Rc<dyn Fn(ThreadAction)>,
) -> Ref<Button> {
    let button = Button::new();
    button.classes().add("quiet");
    button.set_content(Some(Rc::new(caption.to_owned())));
    button.set_font_size(12.0);
    button.set_padding(Thickness::symmetric(4.0, 0.0));
    button.set_horizontal_alignment(HorizontalAlignment::Left);
    AutomationProperties::set_name(&button, Some(name));
    ToolTip::set_tip(&button, Some(Rc::new(name.to_owned())));
    let invoke = actions.clone();
    button.click(move |_, _| invoke(action.clone()));
    button
}

fn labelled_action(
    name: &str,
    icon: &str,
    action: ThreadAction,
    actions: &Rc<dyn Fn(ThreadAction)>,
) -> Ref<Button> {
    let invoke = actions.clone();
    widgets::action(name, icon, move || invoke(action.clone()))
}

fn icon_action(
    name: &str,
    icon: &str,
    action: ThreadAction,
    actions: &Rc<dyn Fn(ThreadAction)>,
) -> Ref<Button> {
    let button = widgets::icon_button(name, icon);
    let invoke = actions.clone();
    button.click(move |_, _| invoke(action.clone()));
    button
}

fn menu_action(
    menu: &MenuFlyout,
    name: &str,
    icon: &str,
    action: ThreadAction,
    actions: &Rc<dyn Fn(ThreadAction)>,
) {
    let item = MenuItem::new();
    item.set_header(Some(Rc::new(name.to_owned())));
    item.set_icon(Some(Control::boxed(crate::icons::icon(icon))));
    let invoke = actions.clone();
    item.click(move |_, _| invoke(action.clone()));
    menu.items().add(Some(Control::boxed(item)));
}

fn badge(text: &str) -> Ref<Border> {
    let border = Border::new();
    border.set_corner_radius(CornerRadius::uniform(5.0));
    border.set_padding(Thickness::symmetric(6.0, 3.0));
    border.set_margin(Thickness::symmetric(3.0, 0.0));
    border.set_vertical_alignment(VerticalAlignment::Center);
    bind_resource(
        &border,
        Border::background_property().as_property(),
        "EmberTint",
    );
    let label = label(text, 9.0);
    label.set_font_weight(FontWeight::SemiBold);
    bind_resource(
        &label,
        TextBlock::foreground_property().as_property(),
        "EmberAccentInk",
    );
    border.set_child(label);
    border
}

fn brush(key: &str) -> Option<Rc<dyn IBrush>> {
    let app = Application::current()?;
    let value = app
        .try_get_resource(&ResourceKey::from(key), app.actual_theme_variant().as_ref())
        .flatten();
    from_markup_value(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroui_base::interactivity::RoutedEventArgs;
    use ferroui_controls::testing::UnitTestApplication;
    use ferroui_controls::{HyperlinkButton, SelectableTextBlock, Window};
    use sample_testing::{Shell, TestGlobalClock};
    use std::cell::RefCell;

    #[test]
    fn overflow_keeps_story_actions_accessible_and_uses_the_current_item() {
        let clock = Rc::new(TestGlobalClock::default());
        let _application = UnitTestApplication::start(sample_testing::services(clock));
        let snapshot = ThreadSnapshot {
            root: HnItem {
                id: 42,
                kind: Some(ItemKind::Story),
                url: Some("https://example.com/article".into()),
                ..Default::default()
            },
            complete: true,
            ..Default::default()
        };
        let reading = ReadingContext {
            hidden: true,
            show_dead: false,
            seen: BTreeSet::new(),
            new_count: None,
            typography: RenderOptions::default(),
            find_foreground: None,
        };
        let calls = Rc::new(RefCell::new(Vec::new()));
        let observed = calls.clone();
        let actions: Rc<dyn Fn(ThreadAction)> =
            Rc::new(move |action| observed.borrow_mut().push(action));
        let header = header(
            &ThreadContext::from(&snapshot),
            &reading,
            ThreadSort::Ranked,
            None,
            true,
            &actions,
        );
        let more = header
            .get_logical_descendants()
            .filter_map(|node| node.cast::<Button>())
            .find(|b| AutomationProperties::get_name(b).as_deref() == Some("More story actions"))
            .expect("named overflow button");
        let menu = more.flyout().unwrap().cast::<MenuFlyout>().unwrap();
        let items = menu.items();
        let entries: Vec<_> = (0..items.count())
            .map(|i| {
                Control::from_boxed(&items.get_at(i).unwrap())
                    .unwrap()
                    .cast::<MenuItem>()
                    .unwrap()
            })
            .collect();
        for (name, index) in [
            ("Reply on HN", 0),
            ("Restore to feed", 1),
            ("Mute example.com", 2),
            ("Search this discussion", 3),
        ] {
            let entry = entries
                .iter()
                .find(|item| {
                    item.header()
                        .as_ref()
                        .and_then(|h| h.downcast_ref::<String>())
                        .is_some_and(|s| s == name)
                })
                .unwrap();
            entry.raise_event(&RoutedEventArgs::with_event(MenuItem::click_event()));
            assert_eq!(calls.borrow().len(), index + 1);
        }
        assert!(matches!(
            calls.borrow().as_slice(),
            [
                ThreadAction::Reply(42),
                ThreadAction::Hide,
                ThreadAction::MuteDomain,
                ThreadAction::SearchDiscussion(42)
            ]
        ));
    }

    #[test]
    fn direct_comment_and_poll_option_links_identify_their_item_and_open_its_context() {
        let clock = Rc::new(TestGlobalClock::default());
        let _application = UnitTestApplication::start(sample_testing::services(clock));
        let target = Rc::new(Cell::new(0));
        let observe = target.clone();
        let actions: Rc<dyn Fn(ThreadAction)> = Rc::new(move |action| {
            if let ThreadAction::GoToItem(id) = action {
                observe.set(id);
            }
        });
        let reading = ReadingContext {
            hidden: false,
            seen: BTreeSet::new(),
            new_count: None,
            show_dead: false,
            typography: RenderOptions::default(),
            find_foreground: None,
        };
        for (root, title, name, expected) in [
            (
                HnItem {
                    kind: Some(ItemKind::Comment),
                    by: Some("maker".to_owned()),
                    parent: Some(4),
                    ..Default::default()
                },
                "Comment by maker",
                "View parent",
                4,
            ),
            (
                HnItem {
                    kind: Some(ItemKind::Pollopt),
                    poll: Some(9),
                    score: Some(1),
                    ..Default::default()
                },
                "Poll option",
                "View poll",
                9,
            ),
        ] {
            assert_eq!(discussion_title(&root, false), title);
            let snapshot = ThreadSnapshot {
                root,
                complete: true,
                ..Default::default()
            };
            assert_eq!(
                discussion_summary(&snapshot),
                if snapshot.root.kind == Some(ItemKind::Pollopt) {
                    "1 vote"
                } else {
                    "0 replies"
                }
            );
            let content = header(
                &ThreadContext::from(&snapshot),
                &reading,
                ThreadSort::Ranked,
                None,
                true,
                &actions,
            );
            let button = content
                .get_logical_descendants()
                .filter_map(|v| v.cast::<Button>())
                .find(|button| AutomationProperties::get_name(button).as_deref() == Some(name))
                .unwrap();
            button.raise_event(&RoutedEventArgs::with_event(Button::click_event()));
            assert_eq!(target.get(), expected);
            if snapshot.root.kind == Some(ItemKind::Pollopt) {
                assert!(content
                    .get_logical_descendants()
                    .filter_map(|v| v.cast::<TextBlock>())
                    .any(|text| text.text().as_deref() == Some("1 vote")));
                let list = build(
                    &snapshot,
                    &ReaderState::default(),
                    "",
                    ThreadSort::Ranked,
                    false,
                    actions.clone(),
                )
                .cast::<ListBox>()
                .unwrap();
                assert_eq!(
                    list.items().count(),
                    1,
                    "a poll option has no reply invitation"
                );
            }
        }
    }

    #[test]
    fn poll_results_withhold_shares_until_all_counts_are_known() {
        let clock = Rc::new(TestGlobalClock::default());
        let _application = UnitTestApplication::start(sample_testing::services(clock));
        let root = HnItem {
            id: 1,
            kind: Some(ItemKind::Poll),
            parts: vec![2, 3],
            ..Default::default()
        };
        let option = |id, score, text: &str| HnItem {
            id,
            score,
            text: Some(text.to_owned()),
            ..Default::default()
        };
        let reading = ReadingContext {
            hidden: false,
            seen: BTreeSet::new(),
            new_count: None,
            show_dead: false,
            typography: RenderOptions::default(),
            find_foreground: None,
        };
        let actions: Rc<dyn Fn(ThreadAction)> = Rc::new(|_| {});
        let mut snapshot = ThreadSnapshot {
            root,
            items: [(2, option(2, Some(1), "First"))].into(),
            missing_ids: vec![3],
            ..Default::default()
        };
        let inspect = |snapshot: &ThreadSnapshot| {
            let control = poll_options(&ThreadContext::from(snapshot), &reading, &actions);
            let lines = control
                .get_logical_descendants()
                .filter_map(|v| v.cast::<TextBlock>())
                .filter_map(|t| t.text())
                .collect::<Vec<_>>();
            let bars = control
                .get_logical_descendants()
                .filter(|v| v.cast::<ProgressBar>().is_some())
                .count();
            (lines, bars)
        };
        let (lines, bars) = inspect(&snapshot);
        assert_eq!(bars, 0);
        assert!(lines.iter().any(|s| s == "1 vote"));
        assert!(lines.iter().any(|s| s == "Option 2 is unavailable."));
        let context = ThreadContext::from(&snapshot);
        let footer = footer(&context, &actions);
        assert!(footer
            .get_logical_descendants()
            .filter_map(|v| v.cast::<Button>())
            .any(|b| AutomationProperties::get_name(&b).as_deref()
                == Some("Retry unavailable items")));
        let no_comments = empty(&context, "", false, &actions);
        assert!(no_comments
            .get_logical_descendants()
            .filter_map(|v| v.cast::<TextBlock>())
            .any(|t| t.text().as_deref() == Some("No comments yet")));
        snapshot.items.insert(3, option(3, None, "Second"));
        let (lines, bars) = inspect(&snapshot);
        assert_eq!(bars, 0);
        assert!(lines.iter().any(|s| s == "Vote count unavailable"));
        snapshot.items.get_mut(&3).unwrap().score = Some(3);
        let (lines, bars) = inspect(&snapshot);
        assert_eq!(bars, 2);
        assert!(lines.iter().any(|s| s == "4 votes in total"));
        assert!(lines.iter().any(|s| s == "1 vote · 25%"));
        assert!(lines.iter().any(|s| s == "3 votes · 75%"));
        snapshot.items.get_mut(&2).unwrap().score = Some(0);
        snapshot.items.get_mut(&3).unwrap().score = Some(0);
        let (lines, bars) = inspect(&snapshot);
        assert_eq!(bars, 0);
        assert!(lines.iter().any(|s| s == "No votes yet"));
    }

    #[test]
    fn conversation_find_matches_visible_decoded_text_with_ancestor_context() {
        let clock = Rc::new(TestGlobalClock::default());
        let _application = UnitTestApplication::start(sample_testing::services(clock));
        let root = HnItem {
            id: 1000,
            kids: vec![2001, 2003],
            ..Default::default()
        };
        let parent = HnItem {
            id: 2001,
            parent: Some(1000),
            kids: vec![2002],
            text: Some("<p>fish &amp; chips</p>".into()),
            ..Default::default()
        };
        let child = HnItem {
            id: 2002,
            parent: Some(2001),
            text: Some("<p><b>native</b> <i>text</i> <a href='https://attributeonly.example'>a link</a></p>".into()),
            ..Default::default()
        };
        let inert = HnItem {
            id: 2003,
            parent: Some(1000),
            text: Some("<script>hiddenneedle</script><p>safe content</p>".into()),
            ..Default::default()
        };
        let snapshot = ThreadSnapshot {
            root,
            items: [parent, child, inert]
                .into_iter()
                .map(|item| (item.id, item))
                .collect(),
            complete: true,
            ..Default::default()
        };
        for (query, expected) in [
            ("fish & chips", vec![2001]),
            ("  NATIVE TEXT  ", vec![2001, 2002]),
            ("attributeonly", vec![]),
            ("hiddenneedle", vec![]),
            ("<script>", vec![]),
        ] {
            let control = build(
                &snapshot,
                &ReaderState::default(),
                query,
                ThreadSort::Ranked,
                false,
                Rc::new(|_| {}),
            );
            let list = control.cast::<ListBox>().unwrap();
            let ids: Vec<_> = (0..list.items().count())
                .filter_map(|index| {
                    let item = list.items().get_at(index)?;
                    let row = item.downcast_ref::<Rc<ConversationRow>>()?;
                    match &row.kind {
                        RowKind::Comment(comment) => Some(comment.id),
                        _ => None,
                    }
                })
                .collect();
            assert_eq!(ids, expected, "query: {query}");
        }
    }

    #[test]
    fn find_cycles_actual_hits_and_keeps_ids_stable_across_sort_changes() {
        let clock = Rc::new(TestGlobalClock::default());
        let _application = UnitTestApplication::start(sample_testing::services(clock));
        let root = HnItem {
            id: 1,
            kids: vec![2, 4, 5],
            ..Default::default()
        };
        let comment = |id, text: &str| HnItem {
            id,
            text: Some(text.to_owned()),
            time: Some(id),
            ..Default::default()
        };
        let parent = HnItem {
            kids: vec![3],
            ..comment(2, "Context only")
        };
        let child = comment(3, "<b>Found</b> &amp; decoded");
        let author = HnItem {
            by: Some("founder".into()),
            ..comment(4, "Author-only hit")
        };
        let hidden = HnItem {
            dead: true,
            ..comment(5, "Found in hidden text")
        };
        let snapshot = ThreadSnapshot {
            root,
            items: [parent, child, author, hidden]
                .into_iter()
                .map(|i| (i.id, i))
                .collect(),
            complete: true,
            ..Default::default()
        };
        let mut state = ReaderState::default();
        state.items.entry(1).or_default().collapsed.insert(2);
        let view = |q: &str, sort| build(&snapshot, &state, q, sort, false, Rc::new(|_| {}));
        let results = view("found", ThreadSort::Ranked);
        let first = FindMatch {
            id: 3,
            ordinal: 1,
            total: 2,
        };
        let last = FindMatch {
            id: 4,
            ordinal: 2,
            total: 2,
        };
        assert_eq!(select_find_match(&results, None, 0), Some(first));
        assert_eq!(select_find_match(&results, Some(3), -1), Some(last));
        assert_eq!(select_find_match(&results, Some(4), 1), Some(first));
        assert_eq!(
            select_find_match(&results, Some(2), 0),
            Some(first),
            "retained ancestors are not hits"
        );
        let reordered = view("found", ThreadSort::Newest);
        assert_eq!(
            select_find_match(&reordered, Some(3), 0),
            Some(FindMatch {
                ordinal: 2,
                ..first
            })
        );
        assert_eq!(
            select_find_match(&view("absent", ThreadSort::Ranked), None, 0),
            None
        );
        assert_eq!(
            select_find_match(&view("deleted", ThreadSort::Ranked), None, 0),
            None,
            "an absent author has no visible text to match"
        );
        assert_eq!(
            select_find_match(&view("", ThreadSort::Ranked), None, 0),
            None
        );
        assert!(
            state.items[&1].collapsed.contains(&2),
            "finding a hidden descendant must not change the saved collapse state"
        );
    }

    #[test]
    fn collapsed_deleted_and_dead_comments_do_not_expose_retained_html() {
        let clock = Rc::new(TestGlobalClock::default());
        let _application = UnitTestApplication::start(sample_testing::services(clock));
        let root = HnItem {
            id: 1000,
            ..Default::default()
        };
        let state = ReaderState::default();
        let actions: Rc<dyn Fn(ThreadAction)> = Rc::new(|_| {});
        for (deleted, dead, placeholder) in [(true, false, "[deleted]"), (false, true, "[dead]")] {
            let row = ThreadRow {
                id: 2001,
                parent_id: root.id,
                depth: 0,
                item: Some(HnItem {
                    id: 2001,
                    text: Some("<p>retained source must stay hidden</p>".into()),
                    deleted,
                    dead,
                    ..Default::default()
                }),
                collapsed: true,
                child_count: 1,
                matches_query: false,
            };
            let document = OnceCell::new();
            let control = comment_row(
                &row,
                &document,
                &root,
                &ReadingContext {
                    hidden: false,
                    seen: BTreeSet::new(),
                    new_count: None,
                    show_dead: false,
                    typography: render_options(&state),
                    find_foreground: None,
                },
                "",
                BranchNeighbors::default(),
                &actions,
            );
            let text = control
                .get_logical_descendants()
                .filter_map(|node| node.cast::<TextBlock>())
                .filter_map(|block| block.text())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains(placeholder));
            assert!(!text.contains("retained source"));
            assert!(document.get().is_none(), "hidden HTML must not be parsed");
        }
    }

    #[test]
    fn inline_touch_links_share_the_sentence_baseline_at_reader_font_sizes() {
        let shell = Shell::start(&crate::SAMPLE, 390.0, 844.0, || crate::App::new().upcast());
        for (font_size, caption) in [
            (17.0, "the design notes".to_owned()),
            (22.0, "the design notes".to_owned()),
            (17.0, "a wrapped link caption ".repeat(12)),
        ] {
            let document = HtmlDocument::parse(&format!(
                "<p>See <a href='https://example.com/'>{caption}</a>.</p><p>Next paragraph.</p>"
            ));
            let mut options = render_options(&ReaderState::default());
            options.font_size = font_size;
            options.line_height = font_size * 1.55;
            let content = ember_content::render(&document, &options, Rc::new(|_| {}));
            content.set_margin(Thickness::uniform(20.0));
            let window = Window::new();
            window.set_width(390.0);
            window.set_height(844.0);
            window.set_content(Some(Control::boxed(&content)));
            window.show();
            for _ in 0..4 {
                shell.settle();
            }
            let paragraph = content
                .get_visual_descendants()
                .find_map(|node| node.cast::<SelectableTextBlock>())
                .expect("native selectable paragraph");
            let link = paragraph
                .get_visual_descendants()
                .find_map(|node| node.cast::<HyperlinkButton>())
                .expect("native inline hyperlink");
            let caption = link
                .get_visual_descendants()
                .find_map(|node| node.cast::<TextBlock>())
                .expect("measured link caption");
            let sentence_baseline = Shell::bounds_of(&window, &paragraph).top()
                + paragraph.padding().top
                + paragraph.layout_rounding_offset()
                + paragraph.text_layout().baseline();
            let caption_baseline = Shell::bounds_of(&window, &caption).top()
                + caption.padding().top
                + caption.layout_rounding_offset()
                + caption.text_layout().baseline();
            assert!(
                (sentence_baseline - caption_baseline).abs() <= 1.0,
                "sentence baseline {sentence_baseline} must align with caption {caption_baseline} at {font_size}px"
            );
            assert!(link.bounds().height >= 44.0, "keep the full touch target");
            window.close();
        }
    }

    #[test]
    fn phone_rich_links_wrap_and_activate_once_while_long_code_stays_bounded() {
        let shell = Shell::start(&crate::SAMPLE, 390.0, 844.0, || crate::App::new().upcast());
        let url = format!("https://example.com/{}", "a".repeat(200));
        let code = "    let unbroken_line = 1234567890;".repeat(5) + "\n";
        let document = HtmlDocument::parse(&format!(
            "<p><a href='{url}'>{url}</a></p><pre>{}</pre>",
            code.repeat(80)
        ));
        let calls = Rc::new(RefCell::new(Vec::new()));
        let observed = calls.clone();
        let content = ember_content::render(
            &document,
            &render_options(&ReaderState::default()),
            Rc::new(move |url| observed.borrow_mut().push(url.to_owned())),
        );
        content.set_margin(Thickness::uniform(20.0));
        let window = Window::new();
        window.set_width(390.0);
        window.set_height(844.0);
        window.set_content(Some(Control::boxed(&content)));
        window.show();
        for _ in 0..4 {
            shell.settle();
        }
        let link = content
            .get_visual_descendants()
            .find_map(|node| node.cast::<HyperlinkButton>())
            .expect("native link is part of the rendered visual tree");
        let link_bounds = Shell::bounds_of(&window, &link);
        assert!(link_bounds.height >= 44.0);
        assert!(link_bounds.right() <= 370.0);
        shell.click(0, link_bounds.center());
        assert_eq!(*calls.borrow(), vec![url]);
        let code_scroll = content
            .get_visual_descendants()
            .filter_map(|node| node.cast::<ScrollViewer>())
            .find(|scroll| AutomationProperties::get_name(scroll).as_deref() == Some("Code block"))
            .expect("preformatted code has its own native viewport");
        assert!(code_scroll.bounds().height <= 420.0);
        assert!(code_scroll.extent().height > code_scroll.viewport().height);
        assert!(code_scroll.extent().width > code_scroll.viewport().width);
        assert!(
            Shell::bounds_of(&window, &code_scroll).top() >= link_bounds.bottom(),
            "wrapped touch links must reserve their full height before the next block"
        );
        assert!(code_scroll.is_scroll_chaining_enabled());
        assert!(content.bounds().width <= 350.0);
        window.close();
    }

    #[test]
    fn rebuilt_discussions_restore_interior_positions_in_long_stories_and_comments() {
        let shell = Shell::start(&crate::SAMPLE, 390.0, 620.0, || crate::App::new().upcast());
        let long_text = "<p>This paragraph is part of a long discussion.</p>".repeat(56);
        let root = HnItem {
            id: 1,
            kids: vec![2, 4],
            text: Some(long_text.clone()),
            ..Default::default()
        };
        let snapshot = ThreadSnapshot {
            root,
            items: [
                HnItem {
                    id: 2,
                    kids: vec![3],
                    text: Some(long_text.clone()),
                    ..Default::default()
                },
                HnItem {
                    id: 3,
                    text: Some(long_text.clone()),
                    ..Default::default()
                },
                HnItem {
                    id: 4,
                    text: Some(long_text),
                    ..Default::default()
                },
            ]
            .into_iter()
            .map(|item| (item.id, item))
            .collect(),
            complete: true,
            ..Default::default()
        };
        let make_view = |state: &ReaderState| {
            build(
                &snapshot,
                state,
                "",
                ThreadSort::Ranked,
                false,
                Rc::new(|_| {}),
            )
        };
        let state = ReaderState::default();
        let window = Window::new();
        window.set_width(390.0);
        window.set_height(620.0);
        window.show();
        for item in [1, 3] {
            let anchor = ReadingPosition { item, inset: 340 };
            let view = make_view(&state);
            restore_position(&view, anchor, false);
            window.set_content(Some(Control::boxed(&view)));
            for _ in 0..5 {
                shell.settle();
            }
            assert_eq!(viewport_position(&view), Some(anchor));
        }
        let mut folded = state;
        folded.items.entry(1).or_default().collapsed.insert(2);
        let view = make_view(&folded);
        restore_position(
            &view,
            ReadingPosition {
                item: 3,
                inset: 340,
            },
            false,
        );
        window.set_content(Some(Control::boxed(&view)));
        for _ in 0..5 {
            shell.settle();
        }
        assert_eq!(viewport_position(&view), Some(ReadingPosition::start(2)));
        window.close();
    }

    #[test]
    fn long_threads_virtualize_rows_and_keep_scrolled_actions_attached_to_their_item() {
        let shell = Shell::start(&crate::SAMPLE, 390.0, 620.0, || crate::App::new().upcast());
        let root = HnItem {
            id: 1000,
            title: Some("A long conversation".into()),
            kids: (2001..=2200).collect(),
            ..Default::default()
        };
        let snapshot = ThreadSnapshot {
            items: root
                .kids
                .iter()
                .map(|&id| {
                    (
                        id,
                        HnItem {
                            id,
                            parent: Some(root.id),
                            by: Some(format!("author{id}")),
                            text: Some(format!("<p>Comment {id} has <b>native</b> content.</p>")),
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            root,
            complete: true,
            ..Default::default()
        };
        let calls = Rc::new(RefCell::new(Vec::new()));
        let observed = calls.clone();
        let content = build(
            &snapshot,
            &ReaderState::default(),
            "",
            ThreadSort::Ranked,
            false,
            Rc::new(move |action| observed.borrow_mut().push(action)),
        );
        let list = content.cast::<ListBox>().unwrap();
        let window = Window::new();
        window.set_width(390.0);
        window.set_height(620.0);
        window.set_content(Some(Control::boxed(&content)));
        window.show();
        for _ in 0..4 {
            shell.settle();
        }
        assert_eq!(list.items().count(), 201);
        assert_eq!(
            viewport_position(&content),
            Some(ReadingPosition::start(1000)),
            "a visible story header anchors the top"
        );
        for index in [20, 170, 10] {
            restore_position(&content, ReadingPosition::start(2000 + index as u64), false);
            for _ in 0..4 {
                shell.settle();
            }
            assert_eq!(
                viewport_position(&content),
                Some(ReadingPosition::start(2000 + index as u64))
            );
            assert!(
                list.get_realized_containers().len() < 50,
                "a phone viewport must not realize the entire conversation"
            );
            let row = list
                .container_from_index(index)
                .expect("requested comment should be realized");
            let permalink = row
                .get_visual_descendants()
                .filter_map(|node| node.cast::<Button>())
                .find(|button| {
                    AutomationProperties::get_name(button).as_deref() == Some("Permalink")
                })
                .expect("realized comment has an accessible permalink action");
            permalink.raise_event(&RoutedEventArgs::with_event(Button::click_event()));
            let expected = ember_core::item_url(2000 + index as u64);
            assert!(
                matches!(calls.borrow().last(), Some(ThreadAction::OpenUrl(url)) if url == &expected)
            );
        }
        // The overflow follows the realized row after recycling. Exercise the actual
        // flyout opening event, then jump between comments without leaving this story.
        for (from, command, destination) in [
            (10, "Next top-level comment", 11),
            (11, "Previous top-level comment", 10),
            (10, "Back to story", 0),
        ] {
            let row = list.container_from_index(from).unwrap();
            let more = row
                .get_visual_descendants()
                .filter_map(|node| node.cast::<Button>())
                .find(|button| {
                    AutomationProperties::get_name(button).as_deref()
                        == Some("More comment actions")
                })
                .unwrap();
            let menu = more.flyout().unwrap().cast::<MenuFlyout>().unwrap();
            menu.show_at(&more);
            shell.settle();
            let items = menu.items();
            let target = (0..items.count())
                .filter_map(|i| Control::from_boxed(&items.get_at(i)?))
                .filter_map(|control| control.cast::<MenuItem>())
                .find(|item| {
                    item.header()
                        .and_then(|h| h.downcast_ref::<String>().cloned())
                        .as_deref()
                        == Some(command)
                })
                .unwrap();
            assert!(menu.is_open());
            target.raise_event(&RoutedEventArgs::with_event(MenuItem::click_event()));
            menu.hide();
            for _ in 0..4 {
                shell.settle();
            }
            let expected = if destination == 0 {
                1000
            } else {
                2000 + destination as u64
            };
            assert_eq!(
                viewport_position(&content),
                Some(ReadingPosition::start(expected))
            );
            assert!(list
                .container_from_index(destination)
                .unwrap()
                .is_keyboard_focus_within());
        }
        assert_eq!(calls.borrow().len(), 3);
        let row = list.container_from_index(1).unwrap();
        let more = row
            .get_visual_descendants()
            .filter_map(|node| node.cast::<Button>())
            .find(|b| AutomationProperties::get_name(b).as_deref() == Some("More comment actions"))
            .unwrap();
        let menu = more.flyout().unwrap().cast::<MenuFlyout>().unwrap();
        menu.show_at(&more);
        shell.settle();
        let entries = menu.items();
        let copy = (0..entries.count())
            .filter_map(|i| Control::from_boxed(&entries.get_at(i)?))
            .filter_map(|v| v.cast::<MenuItem>())
            .find(|item| {
                item.header()
                    .and_then(|v| v.downcast_ref::<String>().cloned())
                    .as_deref()
                    == Some("Copy comment link")
            })
            .unwrap();
        copy.raise_event(&RoutedEventArgs::with_event(MenuItem::click_event()));
        menu.hide();
        assert!(matches!(
            calls.borrow().last(),
            Some(ThreadAction::CopyCommentLink(2001))
        ));
        let replacement = build(
            &snapshot,
            &ReaderState::default(),
            "",
            ThreadSort::Ranked,
            false,
            Rc::new(|_| {}),
        );
        // A rebuilt view is not attached yet when the controller applies saved context.
        restore_position(&replacement, ReadingPosition::start(2010), false);
        window.set_content(Some(Control::boxed(&replacement)));
        for _ in 0..4 {
            shell.settle();
        }
        assert_eq!(
            viewport_position(&replacement),
            Some(ReadingPosition::start(2010))
        );
        let filtered = build(
            &snapshot,
            &ReaderState::default(),
            "native",
            ThreadSort::Ranked,
            false,
            Rc::new(|_| {}),
        );
        let list = filtered.clone().cast::<ListBox>().unwrap();
        // Set the cursor before attachment. Its row has never been realized.
        assert_eq!(
            select_find_match(&filtered, Some(2180), 0).unwrap().ordinal,
            180
        );
        window.set_content(Some(Control::boxed(&filtered)));
        for _ in 0..4 {
            shell.settle();
        }
        for (step, expected) in [(0, 2180), (1, 2181), (-1, 2180)] {
            let from = if step < 0 { 2181 } else { 2180 };
            let found = select_find_match(&filtered, Some(from), step).unwrap();
            for _ in 0..4 {
                shell.settle();
            }
            assert_eq!(found.id, expected);
            assert_eq!(
                viewport_position(&filtered),
                Some(ReadingPosition::start(expected))
            );
            let active: Vec<_> = list
                .get_visual_descendants()
                .filter_map(|node| node.cast::<Border>())
                .filter(|row| row.classes().contains("current-find-result"))
                .collect();
            assert_eq!(active.len(), 1);
            let marked: Vec<_> = active[0]
                .get_logical_descendants()
                .filter_map(|node| node.cast::<ferroui_controls::documents::Run>())
                .filter(|run| run.background().is_some())
                .filter_map(|run| run.text())
                .collect();
            assert_eq!(
                marked,
                ["native"],
                "recycled comments must paint the current search text"
            );
            assert_eq!(
                AutomationProperties::get_item_status(&active[0]),
                Some(format!("Search result {} of 200", found.ordinal))
            );
            assert!(list.get_realized_containers().len() < 50);
        }
        assert!(shell.take_binding_reports().is_empty());
        window.close();
    }
}
