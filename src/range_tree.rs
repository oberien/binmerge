use std::cmp::Ordering;
use std::fmt::Debug;
use std::ops::Range;

use num_traits::{Bounded, Num};

/// A tree of non-overlapping ranges (with gaps)
pub struct RangeTree<T> {
    ranges: Vec<Range<T>>,
}

impl<T: Num + Bounded + Copy + Ord + Debug> RangeTree<T> {
    pub fn new() -> RangeTree<T> {
        RangeTree { ranges: Vec::new() }
    }

    pub fn from_vec(mut ranges: Vec<Range<T>>) -> RangeTree<T> {
        ranges.sort_by_key(|r| r.start);
        for slice in ranges.windows(2) {
            let [a, b] = slice else { unreachable!() };
            assert!(a.start <= a.end);
            assert!(b.start <= b.end);
            assert!(a.end <= b.start);
        }
        RangeTree { ranges }
    }

    /// Append a range which must be larger than all other ranges added so far.
    ///
    /// O(1)
    pub fn append(&mut self, range: Range<T>) {
        let last_val = self.ranges.last()
            .map(|r| r.end)
            .unwrap_or_else(|| T::min_value());
        assert!(range.start <= range.end);
        assert!(range.start >= last_val);
        assert!(range.start <= range.end);
        self.ranges.push(range);
    }

    /// Insert a range into this tree. The range must not overlap any existing range.
    ///
    /// O(n)
    pub fn insert(&mut self, range: Range<T>) {
        let index = self.lookup_index(range.start);
        if index != 0 {
            assert!(self.ranges[index-1].end <= range.start);
        }
        assert!(range.end <= self.ranges.get(index).map(|r| r.start).unwrap_or(T::max_value()));
        self.ranges.insert(index, range);
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn get(&self, index: usize) -> Option<&Range<T>> {
        self.ranges.get(index)
    }

    /// Return the index of the smallest range containing the element, or where a range containing
    /// the element should be inserted.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use binmerge::range_tree::RangeTree;
    /// let range_tree = RangeTree::from_vec(vec![1..2, 3..4, 4..8, 9..10]);
    /// assert_eq!(range_tree.lookup_index(0), 0);
    /// assert_eq!(range_tree.lookup_index(1), 0);
    /// assert_eq!(range_tree.lookup_index(2), 1);
    /// assert_eq!(range_tree.lookup_index(3), 1);
    /// assert_eq!(range_tree.lookup_index(4), 2);
    /// assert_eq!(range_tree.lookup_index(7), 2);
    /// assert_eq!(range_tree.lookup_index(8), 3);
    /// assert_eq!(range_tree.lookup_index(9), 3);
    /// assert_eq!(range_tree.lookup_index(10), 4);
    /// assert_eq!(range_tree.lookup_index(100), 4);
    /// ```
    pub fn lookup_index(&self, element: T) -> usize {
        self.ranges.binary_search_by(|r| {
            if r.end <= element {
                Ordering::Less
            } else if r.contains(&element) {
                Ordering::Greater
            } else if element < r.start {
                Ordering::Greater
            } else {
                unreachable!("{:?} vs {:?}", element, r)
            }
        }).unwrap_err()
    }

    /// Return true if any range of this tree contains the element
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use binmerge::range_tree::RangeTree;
    /// let range_tree = RangeTree::from_vec(vec![0..2, 3..4, 4..8, 9..10]);
    /// assert_eq!(range_tree.contains(1), true);
    /// assert_eq!(range_tree.contains(2), false);
    /// assert_eq!(range_tree.contains(3), true);
    /// assert_eq!(range_tree.contains(4), true);
    /// ```
    ///
    pub fn contains(&self, element: T) -> bool {
        match self.ranges.get(self.lookup_index(element)) {
            Some(range) => range.contains(&element),
            None => false,
        }
    }

    pub fn contains_range_exact(&self, range: Range<T>) -> bool {
        assert!(range.start <= range.end);
        match self.ranges.get(self.lookup_index(range.start)) {
            Some(r) => r.start == range.start && r.end == range.end,
            None => false,
        }
    }

    /// Return an iterator over all ranges touching the given range.
    ///
    /// # Examples
    /// ```rust
    /// # use binmerge::range_tree::RangeTree;
    /// let range_tree = RangeTree::from_vec(vec![0..2, 3..4, 4..8, 9..10]);
    /// let mut ranges = range_tree.ranges_touching(4..9);
    /// assert_eq!(ranges.next(), Some(4..8));
    /// assert_eq!(ranges.next(), Some(9..10));
    /// assert_eq!(ranges.next(), None);
    /// ```
    /// ```rust
    /// # use binmerge::range_tree::RangeTree;
    /// let range_tree = RangeTree::from_vec(vec![0..2, 3..4, 4..8, 9..10]);
    /// let mut ranges = range_tree.ranges_touching(2..8);
    /// assert_eq!(ranges.next(), Some(3..4));
    /// assert_eq!(ranges.next(), Some(4..8));
    /// assert_eq!(ranges.next(), None);
    /// ```
    pub fn ranges_touching(&self, range: Range<T>) -> RangesTouching<T> {
        RangesTouching {
            range_tree: self,
            index: dbg!(self.lookup_index(range.start)),
            end: range.end,
        }
    }

    /// Remove the passed range from this RangeTree if the exact range was contained, returning
    /// if it was deleted.
    ///
    /// O(n)
    pub fn remove_range_exact(&mut self, range: Range<T>) -> bool {
        assert!(range.start <= range.end);
        let index = self.lookup_index(range.start);
        match self.ranges.get(index) {
            Some(r) if r.start == range.start && r.end == range.end => {
                self.ranges.remove(index);
                true
            }
            Some(_) | None => false,
        }
    }
}

pub struct RangesTouching<'a, T> {
    range_tree: &'a RangeTree<T>,
    index: usize,
    end: T,
}
impl<'a, T: Num + Copy + Ord + Debug> Iterator for RangesTouching<'a, T> {
    type Item = Range<T>;

    fn next(&mut self) -> Option<Self::Item> {
        let range = dbg!(self.range_tree.ranges.get(self.index))?;
        if range.start <= self.end {
            self.index += 1;
            Some(range.clone())
        } else {
            None
        }
    }
}
