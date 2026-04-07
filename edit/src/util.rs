// Copyright © 2026 Joaquim Monteiro
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! Miscellaneous functions.

use std::cmp::Ordering;
use std::sync::LazyLock;

use icu_collator::options::{AlternateHandling, CaseLevel, CollatorOptions, Strength};
use icu_collator::preferences::{CollationCaseFirst, CollationNumericOrdering};
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};

use mmm_core::file_tree::{TreeNode, TreeNodeKind};

static COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    let mut prefs = CollatorPreferences::default();
    prefs.numeric_ordering = Some(CollationNumericOrdering::True);
    prefs.case_first = Some(CollationCaseFirst::False);

    let mut options = CollatorOptions::default();
    options.strength = Some(Strength::Tertiary);
    options.alternate_handling = Some(AlternateHandling::NonIgnorable);
    options.case_level = Some(CaseLevel::Off);

    Collator::try_new(prefs, options).unwrap()
});

/// A comparator for [`TreeNode`]s that sorts directories before files
/// and sorts names using the CLDR Collation Algorithm provided by ICU4X.
pub fn node_ord<F>(left: &TreeNode<F>, right: &TreeNode<F>) -> Ordering {
    match (&left.kind, &right.kind) {
        (TreeNodeKind::Dir, TreeNodeKind::File(_)) => Ordering::Less,
        (TreeNodeKind::File(_), TreeNodeKind::Dir) => Ordering::Greater,
        _ => COLLATOR.compare(&left.name, &right.name),
    }
}

/// Moves multiple items in a slice to the specified index.
///
/// When moving items to the right, the target index needs to be adjusted to compensate for the items shifted left,
/// so that the items move still end up in between the items before and at the initial target index.
/// The adjusted index is the value returned by this function.
///
/// # Implementation
///
/// The naïve way to implement this would be to use [`Vec::remove`] and [`Vec::insert`]:
///
/// ```ignore
/// let mut items = Vec::with_capacity(item_indices.len());
/// for idx in item_indices.iter().rev().copied() {
///     items.push(vec.remove(idx));
/// }
///
/// for item in items {
///     vec.insert(to, item);
/// }
/// ```
///
/// This has the downside of shifting a bunch of items in the vector unnecessarily, and it does it multiple times.
/// Instead, we [swap](slice::swap) only the items from the item we want to move to its destination.
///
/// For a group of `N` items, at indices `Xi`, with `i` being the index of each item within the group,
/// such that `i ∈ [0, N)`, that we want to move to index `Y`, the destination index of each item, `Yi`, is:
///
/// `Yi = Y + i, i ∈ [0, N)`
///
/// To move an item from `Xi` to `Yi`, it needs to be shifted right `Yi - Xi` times if `Yi > Xi`,
/// and shifted left `Xi - Yi` times if `Xi > Yi`.
///
/// We can split the `N` items into two groups, the ones that need to be shifted right
/// and the ones that need to be shifted left, by checking if `Xi > Yi`.
/// The first item for which `Xi > Yi` is true marks the beginning of the latter group, as, for every item before it,
/// `Xi < Yi`, and, for every item after it, `Xi > Yi`.
///
/// For the group of items that need to be shifted right, we start by the rightmost item, to avoid shifting left
/// any item we want to shift right. Likewise, we start by shifting the leftmost item from the group of items
/// that need to be shifted left.
///
/// # Example
///
/// With `from` set to `[1, 3, 8]`, and `to` set to `5`, we obtain:
///
/// ```text
///          to
///           │
/// ┌─────────V─────────┐
/// │0 1 2 3 4 5 6 7 8 9│
/// └──┼───┼─────────│──┘
///    │   └─┐       │
///    └───┐ │ ┌─────┘
/// ┌──────V─V─V────────┐
/// │0 2 4 1 3 8 5 6 7 9│
/// └───────────────────┘
/// ```
pub fn move_multiple<T>(slice: &mut [T], from: impl Iterator<Item = usize>, to: usize) -> usize {
    let item_indices = {
        let mut vec: Vec<_> = from.collect();
        vec.sort_unstable();
        vec
    };

    let offset = match item_indices.binary_search(&to) {
        Ok(n) | Err(n) => n,
    };
    let to = to.saturating_sub(offset);

    let split_point = item_indices.partition_point(|from| {
        let i = item_indices
            .element_offset(from)
            .expect("`from` is an element of `item_indices`");
        *from <= (to + i)
    });
    let (left, right) = item_indices.split_at(split_point);

    for (i, from) in left.iter().enumerate().rev() {
        for idx in *from..(to + i) {
            slice.swap(idx, idx + 1);
        }
    }

    for (i, from) in right.iter().enumerate() {
        for idx in ((to + left.len() + 1 + i)..=*from).rev() {
            slice.swap(idx, idx - 1);
        }
    }

    to
}
