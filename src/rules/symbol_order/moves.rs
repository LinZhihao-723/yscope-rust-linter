//! Choosing the items of a scope that stay in place, and where each of the other items belongs.

/// Where a misplaced item belongs among the kept items, and the kept item it conflicts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// The index of the kept item that the misplaced item should come next to.
    pub reference: usize,

    pub direction: Direction,

    /// The index of a kept item whose key is out of order with the misplaced item's key.
    pub conflict: usize,
}

/// The side of the reference item that a misplaced item should come on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    After,
    Before,
}

impl Direction {
    /// # Returns
    ///
    /// The direction as diagnostics spell it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::After => "after",
            Self::Before => "before",
        }
    }
}

/// Chooses the items that stay in place: a longest non-decreasing subsequence of `keys`, and among
/// the longest ones, the one whose indices are lexicographically smallest. Moving only the other
/// items sorts the keys, and no smaller set of moves does.
///
/// # Type Parameters
///
/// * `Key` - The type of the sort keys.
///
/// # Returns
///
/// Whether each item is kept, by index.
pub fn kept<Key: Ord>(keys: &[Key]) -> Vec<bool> {
    let mut lengths = vec![1_usize; keys.len()];
    for index in (0..keys.len()).rev() {
        lengths[index] = 1
            + (index + 1..keys.len())
                .filter(|&later| keys[later] >= keys[index])
                .map(|later| lengths[later])
                .max()
                .unwrap_or(0);
    }
    let mut is_kept = vec![false; keys.len()];
    let Some(&longest) = lengths.iter().max() else {
        return is_kept;
    };
    let mut next = lengths.iter().position(|&length| length == longest);
    while let Some(index) = next {
        is_kept[index] = true;
        next = (index + 1..keys.len())
            .find(|&later| keys[later] >= keys[index] && lengths[later] + 1 == lengths[index]);
    }
    is_kept
}

/// Places the item at `index`, which [`kept`] didn't keep:
///
/// * The reference is the last kept item whose key is at most the item's key, which the item should
///   come after. If there is none, it is the first kept item, which the item should come before.
/// * The conflict is the first kept item before the item with a larger key, or else the last kept
///   item after it with a smaller key.
///
/// # Type Parameters
///
/// * `Key` - The type of the sort keys.
///
/// # Returns
///
/// The item's placement, or [`None`] if no kept item conflicts with it, which only happens if
/// `is_kept` doesn't mark a longest non-decreasing subsequence of `keys`.
pub fn place<Key: Ord>(keys: &[Key], is_kept: &[bool], index: usize) -> Option<Placement> {
    let kept_indices: Vec<usize> = (0..keys.len())
        .filter(|&kept_index| is_kept[kept_index])
        .collect();
    let (reference, direction) = kept_indices
        .iter()
        .rev()
        .find(|&&kept_index| keys[kept_index] <= keys[index])
        .map_or_else(
            || (kept_indices.first().copied(), Direction::Before),
            |&kept_index| (Some(kept_index), Direction::After),
        );
    let conflict = kept_indices
        .iter()
        .find(|&&kept_index| kept_index < index && keys[kept_index] > keys[index])
        .or_else(|| {
            kept_indices
                .iter()
                .rev()
                .find(|&&kept_index| kept_index > index && keys[kept_index] < keys[index])
        });
    Some(Placement {
        reference: reference?,
        direction,
        conflict: *conflict?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// # Returns
    ///
    /// The indices of the items of `keys` that [`kept`] doesn't keep.
    fn moved(keys: &[u8]) -> Vec<usize> {
        kept(keys)
            .into_iter()
            .enumerate()
            .filter_map(|(index, is_kept)| (!is_kept).then_some(index))
            .collect()
    }

    #[test]
    fn only_the_items_outside_a_longest_non_decreasing_subsequence_move() {
        assert_eq!(moved(&[]), Vec::<usize>::new());
        assert_eq!(moved(&[0, 0, 1, 1, 2]), Vec::<usize>::new());
        assert_eq!(moved(&[2, 0, 0, 0, 0]), [0]);
        assert_eq!(moved(&[0, 0, 0, 0, 2, 1]), [5]);
        assert_eq!(moved(&[0, 3, 1, 2, 3]), [1]);
        assert_eq!(moved(&[3, 2, 1, 0]), [1, 2, 3]);
    }

    #[test]
    fn ties_keep_the_earlier_items() {
        assert_eq!(moved(&[0, 1, 0]), [2]);
        assert_eq!(moved(&[1, 0]), [1]);
        assert_eq!(moved(&[1, 2, 0, 0]), [2, 3]);
    }

    #[test]
    fn misplaced_items_are_placed_after_the_last_smaller_kept_item() {
        let keys = [0, 2, 3, 1, 4];
        let is_kept = kept(&keys);
        assert_eq!(
            place(&keys, &is_kept, 3),
            Some(Placement {
                reference: 0,
                direction: Direction::After,
                conflict: 1,
            })
        );

        let keys = [2, 2, 0, 3];
        let is_kept = kept(&keys);
        assert_eq!(
            place(&keys, &is_kept, 2),
            Some(Placement {
                reference: 0,
                direction: Direction::Before,
                conflict: 0,
            })
        );
    }

    #[test]
    fn misplaced_items_without_an_earlier_larger_kept_item_conflict_with_a_later_smaller_one() {
        let keys = [3, 0, 1, 2];
        let is_kept = kept(&keys);
        assert_eq!(
            place(&keys, &is_kept, 0),
            Some(Placement {
                reference: 3,
                direction: Direction::After,
                conflict: 3,
            })
        );
    }
}
