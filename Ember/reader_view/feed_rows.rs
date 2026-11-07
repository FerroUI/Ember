//! The loaded window owns each payload once. Its visible ListBox holds shared references.
use super::*;
use std::ops::Deref;

#[derive(Default)]
pub(super) struct FeedRows {
    ordered: Vec<Rc<HnItem>>,
    positions: BTreeMap<u64, usize>,
    recovery: Option<PageRecovery>,
    pub(super) receipt: (usize, DataSource),
}

/// Allocate the provider order only when a page actually has recoverable gaps.
/// Ordinary paging keeps its existing zero-copy append path.
#[derive(Default)]
struct PageRecovery {
    order: Vec<u64>,
    missing: BTreeSet<u64>,
}

/// Incremental totals and the reading-desk recommendation for the active window.
#[derive(Default)]
pub(super) struct FeedSummaryState {
    pub loaded: usize,
    pub revision: u64,
    pub unread: usize,
    pub suggested: Option<Rc<HnItem>>,
}

impl Deref for FeedRows {
    type Target = [Rc<HnItem>];

    fn deref(&self) -> &Self::Target {
        &self.ordered
    }
}

impl FeedRows {
    pub(super) fn position(&self, id: u64) -> Option<usize> {
        self.positions.get(&id).copied()
    }

    pub(super) fn clear(&mut self) {
        self.ordered.clear();
        self.positions.clear();
        self.recovery = None;
        self.receipt = (0, DataSource::Cache);
    }

    pub(super) fn missing_count(&self) -> usize {
        self.recovery.as_ref().map_or(0, |r| r.missing.len())
    }

    pub(super) fn missing_ids(&self) -> Vec<u64> {
        let Some(recovery) = &self.recovery else {
            return Vec::new();
        };
        let mut missing = recovery.missing.clone();
        recovery
            .order
            .iter()
            .copied()
            .filter(|id| missing.remove(id))
            .collect()
    }

    pub(super) fn accept_page(
        &mut self,
        items: Vec<HnItem>,
        failed: &[(usize, u64)],
        replace: bool,
    ) {
        let mut recovery = if replace { None } else { self.recovery.take() };
        if !failed.is_empty() && recovery.is_none() {
            recovery = Some(PageRecovery {
                order: if replace {
                    Vec::new()
                } else {
                    self.iter().map(|item| item.id).collect()
                },
                missing: BTreeSet::new(),
            });
        }
        if let Some(recovery) = &mut recovery {
            let mut slots = failed.iter().peekable();
            for position in 0..=items.len() {
                while let Some(&&(_, id)) = slots.peek().filter(|slot| slot.0 == position) {
                    if (replace || !self.positions.contains_key(&id)) && recovery.missing.insert(id)
                    {
                        recovery.order.push(id);
                    }
                    slots.next();
                }
                if let Some(item) = items.get(position) {
                    recovery.order.push(item.id);
                    recovery.missing.remove(&item.id);
                }
            }
        }
        self.accept(items, replace);
        self.recovery = recovery.filter(|r| !r.missing.is_empty());
    }

    /// Refresh payloads by ID against the rank we already accepted. A transport
    /// failure leaves a readable row intact; a confirmed null retires the row.
    pub(super) fn reconcile(
        &mut self,
        items: Vec<HnItem>,
        missing: Vec<u64>,
        failed: &[(usize, u64)],
    ) -> usize {
        let mut recovery = self.recovery.take().unwrap_or_else(|| PageRecovery {
            order: self.iter().map(|item| item.id).collect(),
            missing: BTreeSet::new(),
        });
        let mut available: BTreeMap<_, _> =
            self.ordered.drain(..).map(|item| (item.id, item)).collect();
        let mut added = 0;
        for item in items {
            let was_missing = recovery.missing.remove(&item.id);
            if let Some(row) = available.get_mut(&item.id) {
                if **row != item {
                    *row = Rc::new(item);
                }
            } else if was_missing {
                available.insert(item.id, Rc::new(item));
                added += 1;
            }
        }
        let failed: BTreeSet<_> = failed.iter().map(|&(_, id)| id).collect();
        for id in missing {
            if !failed.contains(&id) {
                available.remove(&id);
                recovery.missing.remove(&id);
            }
        }
        self.positions.clear();
        for id in &recovery.order {
            if let Some(item) = available.remove(id) {
                self.positions.insert(*id, self.ordered.len());
                self.ordered.push(item);
            }
        }
        self.recovery = (!recovery.missing.is_empty()).then_some(recovery);
        added
    }

    pub(super) fn accept(&mut self, items: Vec<HnItem>, replace: bool) {
        if replace {
            let old = std::mem::take(self);
            self.ordered.reserve(items.len());
            for item in items {
                self.positions.entry(item.id).or_insert(self.ordered.len());
                let existing = old.position(item.id).map(|i| &old[i]);
                self.ordered.push(match existing {
                    Some(row) if **row == item => row.clone(),
                    _ => Rc::new(item),
                });
            }
        } else {
            for item in items {
                if let std::collections::btree_map::Entry::Vacant(slot) =
                    self.positions.entry(item.id)
                {
                    slot.insert(self.ordered.len());
                    self.ordered.push(Rc::new(item));
                }
            }
        }
    }
}

impl ReaderView {
    pub(super) fn retry_feed_items(&self) {
        if self.feed_load.get() == FeedLoad::Loading {
            return;
        }
        let Some(repo) = self.repo() else { return };
        let ids = self.stories.borrow().missing_ids();
        if ids.is_empty() {
            return;
        }
        let mode = if self.downloaded_shelf() {
            FetchMode::Offline
        } else {
            FetchMode::CacheFirst
        };
        let generation = self.feed_generation.get().wrapping_add(1);
        self.feed_generation.set(generation);
        self.feed_load.set(FeedLoad::Loading);
        self.button("LoadMoreButton").set_is_enabled(false);
        let retry = self.button("RetryFeedItems");
        retry.set_content(Some(Rc::new("Retrying unavailable stories…".to_owned())));
        AutomationProperties::set_name(&retry, Some("Retrying unavailable stories"));
        let owner = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let batch = repo.load_items(&ids, services::now(), mode).await;
                let Some(view) = owner
                    .upgrade()
                    .filter(|v| v.feed_generation.get() == generation)
                else {
                    return;
                };
                let focus = view.button("RetryFeedItems").is_focused();
                let (total, old_source) = view.stories.borrow().receipt;
                let source = if batch.items.is_empty() {
                    old_source
                } else {
                    batch.source
                };
                let warning = batch.warning.filter(|_| !batch.warning_from_failed_item);
                let first = if focus {
                    let state = view.state.borrow();
                    let route = view.feed.borrow();
                    batch
                        .items
                        .iter()
                        .find(|item| route.includes(item, &state, view.downloaded_only.get()))
                        .map(|item| item.id)
                } else {
                    None
                };
                let recovered = view.stories.borrow_mut().reconcile(
                    batch.items,
                    batch.missing_ids,
                    &batch.failed_slots,
                );
                let missing = view.stories.borrow().missing_count();
                view.feed_load
                    .set(if missing != 0 && view.stories.borrow().is_empty() {
                        FeedLoad::Unavailable
                    } else {
                        FeedLoad::Ready
                    });
                view.button("LoadMoreButton").set_is_enabled(true);
                view.set_feed_summary(total, source);
                view.render_stories();
                let message = match (recovered, missing) {
                    (0, 0) => "These stories are no longer available.".to_owned(),
                    (0, _) if mode == FetchMode::Offline => {
                        "Still unavailable on this device.".to_owned()
                    }
                    (0, _) => "Stories are still unavailable. Try again when connected.".to_owned(),
                    (1, _) => "Recovered 1 story.".to_owned(),
                    (n, _) => format!("Recovered {n} stories."),
                };
                view.status(warning.as_deref().unwrap_or(&message));
                if focus && missing == 0 {
                    // Row reconciliation restores the old viewport at Loaded priority.
                    // Move to the recovered keyboard target after that restoration.
                    let target = view.downgrade();
                    Dispatcher::ui_thread().post_local(
                        move || {
                            if let Some(view) = target.upgrade().filter(|v| {
                                v.feed_generation.get() == generation
                                    && v.get_control::<Border>("FeedPane").is_effectively_visible()
                            }) {
                                view.focus_recovered_story(first);
                            }
                        },
                        DispatcherPriority::LOADED,
                    );
                }
            }),
        );
    }

    fn focus_recovered_story(&self, first: Option<u64>) {
        let list = self.get_control::<ListBox>("Stories");
        let row = first.and_then(|id| {
            self.rendered_stories.snapshot().iter().position(|value| {
                value
                    .as_ref()
                    .and_then(|v| v.downcast_ref::<HnItem>())
                    .is_some_and(|item| item.id == id)
            })
        });
        if let Some(index) = row {
            list.scroll_into_view(index as i32);
            if let Some(card) = list.container_from_index(index as i32).and_then(|r| {
                r.get_visual_descendants()
                    .find_map(|v| v.cast::<StoryCard>())
            }) {
                card.focus();
                return;
            }
        }
        if list.items().count() != 0 {
            list.focus();
        } else if self.button("LoadMoreButton").is_effectively_visible() {
            self.button("LoadMoreButton").focus();
        } else if self.button("RetryButton").is_effectively_visible() {
            self.button("RetryButton").focus();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshing_reuses_unchanged_payloads_even_after_reordering() {
        let story = |id, score| HnItem {
            id,
            score: Some(score),
            ..HnItem::default()
        };
        let mut rows = FeedRows::default();
        rows.accept(vec![story(1, 5), story(2, 7)], true);
        let one = rows[0].clone();
        let two = rows[1].clone();
        rows.accept(vec![story(2, 7), story(1, 6)], true);
        assert!(Rc::ptr_eq(&two, &rows[0]));
        assert!(!Rc::ptr_eq(&one, &rows[1]));
        assert_eq!(rows[1].score, Some(6));
        rows.accept(vec![story(2, 8), story(3, 9), story(3, 9)], false);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.position(1), Some(1));
        assert_eq!(rows.position(3), Some(2));
    }
}
