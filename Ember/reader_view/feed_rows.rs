//! The loaded window owns each payload once. Its visible ListBox holds shared references.
use super::*;
use std::ops::Deref;

#[derive(Default)]
pub(super) struct FeedRows {
    ordered: Vec<Rc<HnItem>>,
    positions: BTreeMap<u64, usize>,
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
