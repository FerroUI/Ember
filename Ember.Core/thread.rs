use crate::{DataSource, HnItem, ItemId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ThreadSnapshot {
    pub root: HnItem,
    pub items: BTreeMap<ItemId, HnItem>,
    pub missing_ids: Vec<ItemId>,
    pub pending_ids: Vec<ItemId>,
    pub complete: bool,
    pub source: DataSource,
    pub stale: bool,
    pub warning: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThreadSort {
    #[default]
    Ranked,
    Newest,
    Oldest,
}

#[derive(Clone, Debug)]
pub struct ThreadOptions {
    pub collapsed: BTreeSet<ItemId>,
    pub query: String,
    pub sort: ThreadSort,
    pub show_dead: bool,
    pub max_depth: usize,
}
impl Default for ThreadOptions {
    fn default() -> Self {
        Self {
            collapsed: BTreeSet::new(),
            query: String::new(),
            sort: ThreadSort::Ranked,
            show_dead: false,
            max_depth: 64,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ThreadRow {
    pub id: ItemId,
    pub parent_id: ItemId,
    pub depth: usize,
    pub item: Option<HnItem>,
    pub collapsed: bool,
    pub child_count: usize,
    pub matches_query: bool,
}

/// Iterative, cycle-safe traversal. Search includes ancestor context and searches
/// collapsed branches without altering their persisted collapse state. Deleted
/// and dead parents remain structural placeholders so descendants stay reachable.
pub fn flatten_thread(snapshot: &ThreadSnapshot, options: &ThreadOptions) -> Vec<ThreadRow> {
    flatten_thread_with_match(snapshot, options, |item, query| {
        item.text
            .as_deref()
            .unwrap_or_default()
            .to_lowercase()
            .contains(query)
            || item.author().to_lowercase().contains(query)
    })
}

/// The same traversal with a host-provided text matcher. The query passed to the
/// callback is trimmed and lowercased. A rich-content host can match decoded,
/// visible text without requiring an HTML parser in the reader data layer.
/// Deleted/hidden-dead items never reach the matcher, and returned items retain
/// their original content for rendering.
pub fn flatten_thread_with_match(
    snapshot: &ThreadSnapshot,
    options: &ThreadOptions,
    mut matches: impl FnMut(&HnItem, &str) -> bool,
) -> Vec<ThreadRow> {
    let query = options.query.trim().to_lowercase();
    let mut rows = Vec::new();
    let mut seen = BTreeSet::from([snapshot.root.id]);
    let mut stack = Vec::new();
    let children = ordered_children(&snapshot.root.kids, snapshot, options.sort);
    for id in children.into_iter().rev() {
        stack.push((id, snapshot.root.id, 0));
    }
    while let Some((id, parent_id, depth)) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let item = snapshot.items.get(&id).cloned();
        let collapsed = options.collapsed.contains(&id);
        let matches_query = !query.is_empty()
            && item.as_ref().is_some_and(|item| {
                (!item.dead || options.show_dead) && !item.deleted && matches(item, &query)
            });
        let children = item
            .as_ref()
            .map(|item| ordered_children(&item.kids, snapshot, options.sort))
            .unwrap_or_default();
        rows.push(ThreadRow {
            id,
            parent_id,
            depth,
            item,
            collapsed,
            child_count: children.len(),
            matches_query,
        });
        if depth < options.max_depth.min(256) && (!collapsed || !query.is_empty()) {
            for child in children.into_iter().rev() {
                stack.push((child, id, depth + 1));
            }
        }
    }
    if !query.is_empty() {
        let parents: BTreeMap<_, _> = rows.iter().map(|row| (row.id, row.parent_id)).collect();
        let mut retained = BTreeSet::new();
        for row in rows.iter().filter(|row| row.matches_query) {
            let mut current = row.id;
            while retained.insert(current) {
                let Some(parent) = parents.get(&current) else {
                    break;
                };
                current = *parent;
            }
        }
        rows.retain(|row| retained.contains(&row.id));
    }
    rows
}

fn ordered_children(ids: &[ItemId], snapshot: &ThreadSnapshot, sort: ThreadSort) -> Vec<ItemId> {
    let mut ids = ids.to_vec();
    if sort != ThreadSort::Ranked {
        ids.sort_by_key(|id| {
            (
                snapshot
                    .items
                    .get(id)
                    .and_then(|item| item.time)
                    .unwrap_or(0),
                *id,
            )
        });
        if sort == ThreadSort::Newest {
            ids.reverse();
        }
    }
    ids
}
