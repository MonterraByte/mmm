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
use std::fmt::{self, Write};
use std::ops::Deref;
use std::str::Utf8Error;
use std::string::FromUtf16Error;
use std::sync::Arc;
use std::sync::LazyLock;

use icu_collator::options::{AlternateHandling, CaseLevel, CollatorOptions, Strength};
use icu_collator::preferences::{CollationCaseFirst, CollationNumericOrdering};
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use thiserror::Error;

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

/// Converts UTF-8 or UTF-16 bytes into a `String`.
///
/// If interpreting the bytes as UTF-8 fails, this function will attempt to convert them from UTF-16,
/// using the BOM if present, or assuming little endian if not.
pub fn utf8_or_utf16_bytes_to_string(bytes: Vec<u8>) -> Result<String, FromUtf8OrUtf16Error> {
    let (utf8_err, bytes) = match String::from_utf8(bytes) {
        Ok(utf8) => return Ok(utf8),
        Err(err) => (err.utf8_error(), err.into_bytes()),
    };

    let endian = match bytes.get(..2) {
        Some([0xfe, 0xff]) => Endian::Big,
        Some([0xff, 0xfe]) => Endian::Little,
        _ => Endian::Unspecified,
    };

    let utf16_bytes_without_bom = if endian != Endian::Unspecified {
        &bytes[2..]
    } else {
        bytes.as_slice()
    };

    match endian {
        Endian::Little | Endian::Unspecified => String::from_utf16le(utf16_bytes_without_bom),
        Endian::Big => String::from_utf16be(utf16_bytes_without_bom),
    }
    .map_err(|utf16_err| FromUtf8OrUtf16Error { utf8_err, utf16_err, endian })
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Endian {
    Unspecified,
    Little,
    Big,
}

impl fmt::Display for Endian {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.write_str(match self {
            Endian::Unspecified => "unspecified",
            Endian::Little => "little",
            Endian::Big => "big",
        })
    }
}

/// Error type returned by [`utf8_or_utf16_bytes_to_string`].
#[derive(Debug, Error)]
#[error(
    "failed to convert to UTF-8: {utf8_err}, failed to convert to UTF-16: {utf16_err}, detected endianness: {endian}"
)]
pub struct FromUtf8OrUtf16Error {
    utf8_err: Utf8Error,
    utf16_err: FromUtf16Error,
    endian: Endian,
}

/// A cheaply clonable string type with inlining for small strings.
#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct SharedStr(flexstr::SharedStr);

pub static EMPTY_STR: SharedStr = SharedStr::empty();

impl SharedStr {
    /// Converts the string into a `SharedStr`, inlining it if possible, or moving it to an `Arc` otherwise.
    #[must_use]
    pub fn new<S: AsRef<str>>(s: S) -> Self {
        let s = s.as_ref();
        let shared_str = if let Ok(inline) = inline_flexstr::InlineFlexStr::try_from_type(s) {
            flexstr::FlexStr::Inlined(inline)
        } else {
            flexstr::FlexStr::RefCounted(Arc::from(s))
        };
        Self(shared_str)
    }

    #[must_use]
    pub const fn from_static_str(s: &'static str) -> Self {
        Self(flexstr::FlexStr::Borrowed(s))
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self(flexstr::FlexStr::Borrowed(""))
    }

    #[must_use]
    pub fn to_owned(&self) -> String {
        self.0.to_owned_type()
    }
}

impl fmt::Debug for SharedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_char('"')?;
        f.write_str(self.0.as_ref())?;
        f.write_char('"')
    }
}

impl fmt::Display for SharedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_ref())
    }
}

impl AsRef<str> for SharedStr {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

impl Deref for SharedStr {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl Default for SharedStr {
    fn default() -> Self {
        Self::empty()
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
