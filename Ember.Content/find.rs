//! Match byte ranges in the original, decoded text while comparing lowercase strings.
//! Native runs can split a match at any formatting boundary.

use std::collections::VecDeque;
use std::ops::Range;

#[derive(Default)]
pub(crate) struct FindRanges {
    ranges: VecDeque<Range<usize>>,
    offset: usize,
}

impl FindRanges {
    pub(crate) fn new(text: &str, needle: &str, budget: &mut usize) -> Self {
        if needle.is_empty() || *budget == 0 {
            return Self::default();
        }
        let folded = text.to_lowercase();
        let mut source = text.char_indices();
        let mut folded_end = 0;
        let mut scalar = 0..0;
        // A lowercase expansion (e.g. İ → i + combining dot) can change the UTF-8
        // length, or even contain both boundaries of a hit. Walk source scalars
        // once, in hit order. Contextual lowercase sigma has the same byte width.
        let mut source_scalar = |byte| {
            while byte >= folded_end {
                let (start, ch) = source.next().expect("byte lies inside folded text");
                scalar = start..start + ch.len_utf8();
                folded_end += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
            }
            scalar.clone()
        };
        let ascii = text.is_ascii();
        let mut ranges: VecDeque<Range<usize>> = VecDeque::new();
        for (start, found) in folded.match_indices(needle).take(*budget) {
            let end = start + found.len();
            let range = if ascii {
                start..end
            } else {
                source_scalar(start).start..source_scalar(end - 1).end
            };
            *budget -= 1;
            match ranges.back_mut() {
                Some(last) if last.end >= range.start => last.end = range.end,
                _ => ranges.push_back(range),
            }
        }
        Self { ranges, offset: 0 }
    }

    pub(crate) fn has_hits(&self) -> bool {
        !self.ranges.is_empty()
    }

    /// Emit exactly the original input, partitioned into marked and unmarked runs.
    pub(crate) fn segments<'a>(&mut self, text: &'a str, mut emit: impl FnMut(&'a str, bool)) {
        let base = self.offset;
        let end = base + text.len();
        let mut pos = base;
        while let Some(hit) = self.ranges.front() {
            if hit.start >= end {
                break;
            }
            let start = hit.start.max(pos);
            if start > pos {
                emit(&text[pos - base..start - base], false);
            }
            pos = hit.end.min(end);
            if pos > start {
                emit(&text[start - base..pos - base], true);
            }
            if hit.end > end {
                break;
            }
            self.ranges.pop_front();
        }
        if pos < end {
            emit(&text[pos - base..], false);
        }
        self.offset = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_unicode_lowercase_matches_back_to_original_run_boundaries() {
        let mut budget = 128;
        let mut hits = FindRanges::new("İstanbul & ΟΣ!", "i\u{307}stan", &mut budget);
        let mut segments = Vec::new();
        for part in ["İs", "tan", "bul & ", "ΟΣ!"] {
            hits.segments(part, |s, marked| segments.push((s.to_owned(), marked)));
        }
        assert_eq!(
            segments,
            [
                ("İs".into(), true),
                ("tan".into(), true),
                ("bul & ".into(), false),
                ("ΟΣ!".into(), false)
            ]
        );
        let mut hits = FindRanges::new("İstanbul & ΟΣ!", "ς", &mut budget);
        let mut marked = Vec::new();
        hits.segments("İstanbul & ΟΣ!", |s, hit| {
            if hit {
                marked.push(s)
            }
        });
        assert_eq!(marked, ["Σ"]);
        let mut hits = FindRanges::new("İ", "\u{307}", &mut budget);
        hits.segments("İ", |s, hit| {
            assert_eq!(s, "İ");
            assert!(hit);
        });
    }
}
