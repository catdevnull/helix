//! Selections are the primary editing construct. Even a single cursor is
//! defined as a single empty or 1-wide selection range.
//!
//! All positioning is done via `char` offsets into the buffer.
use crate::{Assoc, ChangeSet, Rope, RopeSlice};
use smallvec::{smallvec, SmallVec};
use std::borrow::Cow;

#[inline]
fn abs_difference(x: usize, y: usize) -> usize {
    if x < y {
        y - x
    } else {
        x - y
    }
}

/// A single selection range.
///
/// The range consists of an "anchor" and "head" position in
/// the text.  The head is the part that the user moves when
/// directly extending the selection.  The head and anchor
/// can be in any order: either can precede or follow the
/// other in the text, and they can share the same position
/// for a zero-width range.
///
/// Below are some example `Range` configurations to better
/// illustrate.  The anchor and head indices are show as
/// "(anchor, head)", followed by example text with "[" and "]"
/// inserted to visually represent the anchor and head positions:
///
/// - (0, 3): [Som]e text.
/// - (3, 0): ]Som[e text.
/// - (2, 7): So[me te]xt.
/// - (1, 1): S[]ome text.
///
/// Ranges are considered to be inclusive on the left and
/// exclusive on the right, regardless of anchor-head ordering.
/// This means, for example, that non-zero-width ranges that
/// are directly adjecent, sharing an edge, do not overlap.
/// However, a zero-width range will overlap with the shared
/// left-edge of another range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    /// The anchor of the range: the side that doesn't move when extending.
    pub anchor: usize,
    /// The head of the range, moved when extending.
    pub head: usize,
    pub horiz: Option<u32>,
}

impl Range {
    pub fn new(anchor: usize, head: usize) -> Self {
        Self {
            anchor,
            head,
            horiz: None,
        }
    }

    pub fn point(head: usize) -> Self {
        Self::new(head, head)
    }

    /// Start of the range.
    #[inline]
    #[must_use]
    pub fn from(&self) -> usize {
        std::cmp::min(self.anchor, self.head)
    }

    /// End of the range.
    #[inline]
    #[must_use]
    pub fn to(&self) -> usize {
        std::cmp::max(self.anchor, self.head)
    }

    /// `true` when head and anchor are at the same position.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Check two ranges for overlap.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        // To my eye, it's non-obvious why this works, but I arrived
        // at it after transforming the slower version that explicitly
        // enumerated more cases.  The unit tests are thorough.
        self.from() == other.from() || (self.to() > other.from() && other.to() > self.from())
    }

    pub fn contains(&self, pos: usize) -> bool {
        self.from() <= pos && pos < self.to()
    }

    /// Map a range through a set of changes. Returns a new range representing the same position
    /// after the changes are applied.
    pub fn map(self, changes: &ChangeSet) -> Self {
        let anchor = changes.map_pos(self.anchor, Assoc::After);
        let head = changes.map_pos(self.head, Assoc::After);

        // We want to return a new `Range` with `horiz == None` every time,
        // even if the anchor and head haven't changed, because we don't
        // know if the *visual* position hasn't changed due to
        // character-width or grapheme changes earlier in the text.
        Self {
            anchor,
            head,
            horiz: None,
        }
    }

    /// Extend the range to cover at least `from` `to`.
    #[must_use]
    pub fn extend(&self, from: usize, to: usize) -> Self {
        debug_assert!(from <= to);

        if self.anchor <= self.head {
            Self {
                anchor: self.anchor.min(from),
                head: self.head.max(to),
                horiz: None,
            }
        } else {
            Self {
                anchor: self.anchor.max(to),
                head: self.head.min(from),
                horiz: None,
            }
        }
    }

    // groupAt

    #[inline]
    pub fn fragment<'a, 'b: 'a>(&'a self, text: RopeSlice<'b>) -> Cow<'b, str> {
        text.slice(self.from()..self.to()).into()
    }
}

/// A selection consists of one or more selection ranges.
/// invariant: A selection can never be empty (always contains at least primary range).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    ranges: SmallVec<[Range; 1]>,
    primary_index: usize,
}

#[allow(clippy::len_without_is_empty)] // a Selection is never empty
impl Selection {
    // eq

    #[must_use]
    pub fn primary(&self) -> Range {
        self.ranges[self.primary_index]
    }

    #[must_use]
    pub fn cursor(&self) -> usize {
        self.primary().head
    }

    /// Ensure selection containing only the primary selection.
    pub fn into_single(self) -> Self {
        if self.ranges.len() == 1 {
            self
        } else {
            Self {
                ranges: smallvec![self.ranges[self.primary_index]],
                primary_index: 0,
            }
        }
    }

    pub fn push(mut self, range: Range) -> Self {
        let index = self.ranges.len();
        self.ranges.push(range);

        Self::normalize(self.ranges, index)
    }
    // replace_range

    /// Map selections over a set of changes. Useful for adjusting the selection position after
    /// applying changes to a document.
    pub fn map(self, changes: &ChangeSet) -> Self {
        if changes.is_empty() {
            return self;
        }

        Self::new(
            self.ranges
                .into_iter()
                .map(|range| range.map(changes))
                .collect(),
            self.primary_index,
        )
    }

    pub fn ranges(&self) -> &[Range] {
        &self.ranges
    }

    pub fn primary_index(&self) -> usize {
        self.primary_index
    }

    #[must_use]
    /// Constructs a selection holding a single range.
    pub fn single(anchor: usize, head: usize) -> Self {
        Self {
            ranges: smallvec![Range {
                anchor,
                head,
                horiz: None
            }],
            primary_index: 0,
        }
    }

    /// Constructs a selection holding a single cursor.
    pub fn point(pos: usize) -> Self {
        Self::single(pos, pos)
    }

    fn normalize(mut ranges: SmallVec<[Range; 1]>, mut primary_index: usize) -> Self {
        let primary = ranges[primary_index];
        ranges.sort_unstable_by_key(Range::from);
        primary_index = ranges.iter().position(|&range| range == primary).unwrap();

        let mut result = SmallVec::with_capacity(ranges.len()); // approx

        // TODO: we could do with one vec by removing elements as we mutate

        let mut i = 0;

        for range in ranges.into_iter() {
            // if previous value exists
            if let Some(prev) = result.last_mut() {
                // and we overlap it

                // TODO: we used to simply check range.from() <(=) prev.to()
                // avoiding two comparisons
                if range.overlaps(prev) {
                    let from = prev.from();
                    let to = std::cmp::max(range.to(), prev.to());

                    if i <= primary_index {
                        primary_index -= 1
                    }

                    // merge into previous
                    if range.anchor > range.head {
                        prev.anchor = to;
                        prev.head = from;
                    } else {
                        prev.anchor = from;
                        prev.head = to;
                    }
                    continue;
                }
            }

            result.push(range);
            i += 1
        }

        Self {
            ranges: result,
            primary_index,
        }
    }

    // TODO: consume an iterator or a vec to reduce allocations?
    #[must_use]
    pub fn new(ranges: SmallVec<[Range; 1]>, primary_index: usize) -> Self {
        assert!(!ranges.is_empty());

        // fast path for a single selection (cursor)
        if ranges.len() == 1 {
            return Self {
                ranges,
                primary_index: 0,
            };
        }

        // TODO: only normalize if needed (any ranges out of order)
        Self::normalize(ranges, primary_index)
    }

    /// Takes a closure and maps each selection over the closure.
    pub fn transform<F>(&self, f: F) -> Self
    where
        F: Fn(Range) -> Range,
    {
        Self::new(
            self.ranges.iter().copied().map(f).collect(),
            self.primary_index,
        )
    }

    pub fn fragments<'a>(&'a self, text: RopeSlice<'a>) -> impl Iterator<Item = Cow<str>> + 'a {
        self.ranges.iter().map(move |range| range.fragment(text))
    }

    #[inline(always)]
    pub fn iter(&self) -> std::slice::Iter<'_, Range> {
        self.ranges.iter()
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.ranges.len()
    }
}

impl<'a> IntoIterator for &'a Selection {
    type Item = &'a Range;
    type IntoIter = std::slice::Iter<'a, Range>;

    fn into_iter(self) -> std::slice::Iter<'a, Range> {
        self.ranges().iter()
    }
}

// TODO: checkSelection -> check if valid for doc length && sorted

pub fn keep_matches(
    text: RopeSlice,
    selection: &Selection,
    regex: &crate::regex::Regex,
) -> Option<Selection> {
    let result: SmallVec<_> = selection
        .iter()
        .filter(|range| regex.is_match(&range.fragment(text)))
        .copied()
        .collect();

    // TODO: figure out a new primary index
    if !result.is_empty() {
        return Some(Selection::new(result, 0));
    }
    None
}

pub fn select_on_matches(
    text: RopeSlice,
    selection: &Selection,
    regex: &crate::regex::Regex,
) -> Option<Selection> {
    let mut result = SmallVec::with_capacity(selection.len());

    for sel in selection {
        // TODO: can't avoid occasional allocations since Regex can't operate on chunks yet
        let fragment = sel.fragment(text);

        let mut sel_start = sel.from();
        let sel_end = sel.to();

        let mut start_byte = text.char_to_byte(sel_start);

        for mat in regex.find_iter(&fragment) {
            // TODO: retain range direction

            let start = text.byte_to_char(start_byte + mat.start());
            let end = text.byte_to_char(start_byte + mat.end());
            result.push(Range::new(start, end));
        }
    }

    // TODO: figure out a new primary index
    if !result.is_empty() {
        return Some(Selection::new(result, 0));
    }

    None
}

// TODO: support to split on capture #N instead of whole match
pub fn split_on_matches(
    text: RopeSlice,
    selection: &Selection,
    regex: &crate::regex::Regex,
) -> Selection {
    let mut result = SmallVec::with_capacity(selection.len());

    for sel in selection {
        // Special case: zero-width selection.
        if sel.from() == sel.to() {
            result.push(*sel);
            continue;
        }

        // TODO: can't avoid occasional allocations since Regex can't operate on chunks yet
        let fragment = sel.fragment(text);

        let mut sel_start = sel.from();
        let sel_end = sel.to();

        let mut start_byte = text.char_to_byte(sel_start);

        let mut start = sel_start;

        for mat in regex.find_iter(&fragment) {
            // TODO: retain range direction
            let end = text.byte_to_char(start_byte + mat.start());
            result.push(Range::new(start, end));
            start = text.byte_to_char(start_byte + mat.end());
        }

        if start < sel_end {
            result.push(Range::new(start, sel_end));
        }
    }

    // TODO: figure out a new primary index
    Selection::new(result, 0)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    #[should_panic]
    fn test_new_empty() {
        let sel = Selection::new(smallvec![], 0);
    }

    #[test]
    fn test_create_normalizes_and_merges() {
        let sel = Selection::new(
            smallvec![
                Range::new(10, 12),
                Range::new(6, 7),
                Range::new(4, 5),
                Range::new(3, 4),
                Range::new(0, 6),
                Range::new(7, 8),
                Range::new(9, 13),
                Range::new(13, 14),
            ],
            0,
        );

        let res = sel
            .ranges
            .into_iter()
            .map(|range| format!("{}/{}", range.anchor, range.head))
            .collect::<Vec<String>>()
            .join(",");

        assert_eq!(res, "0/6,6/7,7/8,9/13,13/14");

        // it correctly calculates a new primary index
        let sel = Selection::new(
            smallvec![Range::new(0, 2), Range::new(1, 5), Range::new(4, 7)],
            2,
        );

        let res = sel
            .ranges
            .into_iter()
            .map(|range| format!("{}/{}", range.anchor, range.head))
            .collect::<Vec<String>>()
            .join(",");

        assert_eq!(res, "0/7");
        assert_eq!(sel.primary_index, 0);
    }

    #[test]
    fn test_create_merges_adjacent_points() {
        let sel = Selection::new(
            smallvec![
                Range::new(10, 12),
                Range::new(12, 12),
                Range::new(12, 12),
                Range::new(10, 10),
                Range::new(8, 10),
            ],
            0,
        );

        let res = sel
            .ranges
            .into_iter()
            .map(|range| format!("{}/{}", range.anchor, range.head))
            .collect::<Vec<String>>()
            .join(",");

        assert_eq!(res, "8/10,10/12,12/12");
    }

    #[test]
    fn test_contains() {
        let range = Range::new(10, 12);

        assert_eq!(range.contains(9), false);
        assert_eq!(range.contains(10), true);
        assert_eq!(range.contains(11), true);
        assert_eq!(range.contains(12), false);
        assert_eq!(range.contains(13), false);

        let range = Range::new(9, 6);
        assert_eq!(range.contains(9), false);
        assert_eq!(range.contains(7), true);
        assert_eq!(range.contains(6), true);
    }

    #[test]
    fn test_overlaps() {
        // Two non-zero-width ranges, no overlap.
        assert!(!Range::new(0, 3).overlaps(&Range::new(3, 6)));
        assert!(!Range::new(0, 3).overlaps(&Range::new(6, 3)));
        assert!(!Range::new(3, 0).overlaps(&Range::new(3, 6)));
        assert!(!Range::new(3, 0).overlaps(&Range::new(6, 3)));
        assert!(!Range::new(3, 6).overlaps(&Range::new(0, 3)));
        assert!(!Range::new(3, 6).overlaps(&Range::new(3, 0)));
        assert!(!Range::new(6, 3).overlaps(&Range::new(0, 3)));
        assert!(!Range::new(6, 3).overlaps(&Range::new(3, 0)));

        // Two non-zero-width ranges, overlap.
        assert!(Range::new(0, 4).overlaps(&Range::new(3, 6)));
        assert!(Range::new(0, 4).overlaps(&Range::new(6, 3)));
        assert!(Range::new(4, 0).overlaps(&Range::new(3, 6)));
        assert!(Range::new(4, 0).overlaps(&Range::new(6, 3)));
        assert!(Range::new(3, 6).overlaps(&Range::new(0, 4)));
        assert!(Range::new(3, 6).overlaps(&Range::new(4, 0)));
        assert!(Range::new(6, 3).overlaps(&Range::new(0, 4)));
        assert!(Range::new(6, 3).overlaps(&Range::new(4, 0)));

        // Zero-width and non-zero-width range, no overlap.
        assert!(!Range::new(0, 3).overlaps(&Range::new(3, 3)));
        assert!(!Range::new(3, 0).overlaps(&Range::new(3, 3)));
        assert!(!Range::new(3, 3).overlaps(&Range::new(0, 3)));
        assert!(!Range::new(3, 3).overlaps(&Range::new(3, 0)));

        // Zero-width and non-zero-width range, overlap.
        assert!(Range::new(1, 4).overlaps(&Range::new(1, 1)));
        assert!(Range::new(4, 1).overlaps(&Range::new(1, 1)));
        assert!(Range::new(1, 1).overlaps(&Range::new(1, 4)));
        assert!(Range::new(1, 1).overlaps(&Range::new(4, 1)));

        assert!(Range::new(1, 4).overlaps(&Range::new(3, 3)));
        assert!(Range::new(4, 1).overlaps(&Range::new(3, 3)));
        assert!(Range::new(3, 3).overlaps(&Range::new(1, 4)));
        assert!(Range::new(3, 3).overlaps(&Range::new(4, 1)));

        // Two zero-width ranges, no overlap.
        assert!(!Range::new(0, 0).overlaps(&Range::new(1, 1)));
        assert!(!Range::new(1, 1).overlaps(&Range::new(0, 0)));

        // Two zero-width ranges, overlap.
        assert!(Range::new(1, 1).overlaps(&Range::new(1, 1)));
    }

    #[test]
    fn test_split_on_matches() {
        use crate::regex::Regex;

        let text = Rope::from(" abcd efg wrs   xyz 123 456");

        let selection = Selection::new(smallvec![Range::new(0, 9), Range::new(11, 20),], 0);

        let result = split_on_matches(text.slice(..), &selection, &Regex::new(r"\s+").unwrap());

        assert_eq!(
            result.ranges(),
            &[
                // We get a leading zero-width range when there's
                // a leading match because ranges are inclusive on
                // the left.  Imagine, for example, if the entire
                // selection range were matched: you'd still want
                // at least one range to remain after the split.
                Range::new(0, 0),
                Range::new(1, 5),
                Range::new(6, 9),
                Range::new(11, 13),
                Range::new(16, 19),
                // In contrast to the comment above, there is no
                // _trailing_ zero-width range despite the trailing
                // match, because ranges are exclusive on the right.
            ]
        );

        assert_eq!(
            result.fragments(text.slice(..)).collect::<Vec<_>>(),
            &["", "abcd", "efg", "rs", "xyz"]
        );
    }
}
