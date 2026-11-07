use crate::{DataSource, HnItem, ItemId};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
pub struct ThreadRow<T = Option<HnItem>> {
    pub id: ItemId,
    pub parent_id: ItemId,
    pub depth: usize,
    pub item: T,
    pub collapsed: bool,
    pub child_count: usize,
    pub matches_query: bool,
}

impl<T> ThreadRow<T> {
    /// Preserve display identity and flags while selecting an owned, borrowed or deferred payload.
    pub fn with_item<U>(&self, item: U) -> ThreadRow<U> {
        ThreadRow {
            id: self.id,
            parent_id: self.parent_id,
            depth: self.depth,
            item,
            collapsed: self.collapsed,
            child_count: self.child_count,
            matches_query: self.matches_query,
        }
    }
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
    matches: impl FnMut(&HnItem, &str) -> bool,
) -> Vec<ThreadRow> {
    project_thread_with_match(snapshot, options, matches)
        .into_iter()
        .map(|row| row.with_item(row.item.cloned()))
        .collect()
}

/// Borrow payloads directly from the immutable snapshot. Virtualized views can keep only
/// the display metadata and resolve the item by ID when a row is realized.
pub fn project_thread_with_match<'a>(
    snapshot: &'a ThreadSnapshot,
    options: &ThreadOptions,
    mut matches: impl FnMut(&HnItem, &str) -> bool,
) -> Vec<ThreadRow<Option<&'a HnItem>>> {
    let query = options.query.trim().to_lowercase();
    collect_rows(snapshot, options, !query.is_empty(), true, |item| {
        matches(item, &query)
    })
}

/// Return new replies and their ancestry, across collapsed branches, in the current sort order.
/// The baseline contains the items loaded on previous visits. Unavailable records and hidden
/// text cannot qualify as new replies, but their children remain reachable.
pub fn flatten_thread_since(
    snapshot: &ThreadSnapshot,
    options: &ThreadOptions,
    previous: &BTreeSet<ItemId>,
) -> Vec<ThreadRow> {
    project_thread_since(snapshot, options, previous)
        .into_iter()
        .map(|row| row.with_item(row.item.cloned()))
        .collect()
}

/// Borrowed projection of the previous-visit reply filter.
pub fn project_thread_since<'a>(
    snapshot: &'a ThreadSnapshot,
    options: &ThreadOptions,
    previous: &BTreeSet<ItemId>,
) -> Vec<ThreadRow<Option<&'a HnItem>>> {
    collect_rows(snapshot, options, true, false, |item| {
        !previous.contains(&item.id)
    })
}

fn collect_rows<'a>(
    snapshot: &'a ThreadSnapshot,
    options: &ThreadOptions,
    filtered: bool,
    text_search: bool,
    mut matches: impl FnMut(&HnItem) -> bool,
) -> Vec<ThreadRow<Option<&'a HnItem>>> {
    let mut rows: Vec<ThreadRow<Option<&HnItem>>> = Vec::new();
    let mut retained: Vec<(Option<usize>, bool)> = Vec::new();
    let mut seen = BTreeSet::from([snapshot.root.id]);
    let mut stack: Vec<(_, Option<usize>, _)> =
        ordered_children(&snapshot.root.kids, snapshot, options.sort)
            .iter()
            .rev()
            .map(|&id| (id, None, 0))
            .collect();
    while let Some((id, parent, depth)) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let item = snapshot.items.get(&id);
        let collapsed = options.collapsed.contains(&id);
        let matched = filtered
            && item.is_some_and(|item| {
                (!item.dead || options.show_dead) && !item.deleted && matches(item)
            });
        if filtered {
            retained.push((parent, matched));
        }
        let index = rows.len();
        let parent_id = parent.map_or(snapshot.root.id, |i| rows[i].id);
        let children = item.map_or(&[][..], |item| item.kids.as_slice());
        rows.push(ThreadRow {
            id,
            parent_id,
            depth,
            item,
            collapsed,
            child_count: children.len(),
            matches_query: text_search && matched,
        });
        if depth < options.max_depth.min(256) && (!collapsed || filtered) && !children.is_empty() {
            let children = ordered_children(children, snapshot, options.sort);
            stack.extend(
                children
                    .iter()
                    .rev()
                    .map(|&child| (child, Some(index), depth + 1)),
            );
        }
    }
    if filtered {
        // DFS emits every parent before its children. Propagate membership once per
        // edge in reverse, using the actual display parent even for repeated/cyclic IDs.
        for index in (0..retained.len()).rev() {
            if let (Some(parent), true) = retained[index] {
                retained[parent].1 = true;
            }
        }
        let mut flags = retained.into_iter();
        rows.retain(|_| flags.next().expect("one flag per visited row").1);
    }
    rows
}

fn ordered_children<'a>(
    ids: &'a [ItemId],
    snapshot: &ThreadSnapshot,
    sort: ThreadSort,
) -> Cow<'a, [ItemId]> {
    if sort == ThreadSort::Ranked || ids.len() < 2 {
        return Cow::Borrowed(ids);
    }
    let mut sorted = ids.to_vec();
    sorted.sort_by_cached_key(|id| {
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
        sorted.reverse();
    }
    Cow::Owned(sorted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projected_rows_borrow_original_bodies_and_keep_the_first_display_parent() {
        let mut snapshot = ThreadSnapshot {
            root: HnItem {
                id: 1,
                kids: vec![2, 3],
                ..Default::default()
            },
            ..ThreadSnapshot::default()
        };
        for (id, kids) in [(2, vec![4]), (3, vec![4]), (4, vec![2, 1])] {
            snapshot.items.insert(
                id,
                HnItem {
                    id,
                    kids,
                    text: Some(format!("Reply {id}")),
                    ..Default::default()
                },
            );
        }
        let options = ThreadOptions {
            query: "4".into(),
            ..Default::default()
        };
        let rows = project_thread_with_match(&snapshot, &options, |item, _| item.id == 4);
        assert_eq!(
            rows.iter().map(|r| (r.id, r.parent_id)).collect::<Vec<_>>(),
            [(2, 1), (4, 2)]
        );
        assert!(std::ptr::eq(rows[1].item.unwrap(), &snapshot.items[&4]));
        assert_eq!(
            flatten_thread(&snapshot, &options)[1].item.as_ref(),
            rows[1].item
        );
    }
}
