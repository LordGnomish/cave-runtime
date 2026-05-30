// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Key-indexed scheduling heap.
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/internal/heap/heap.go
//!
//! Upstream's scheduler queues do not use a bare `container/heap`; they use
//! a `Heap` that wraps a `data` struct holding both the heap `queue` of keys
//! and an `items map[string]*heapItem` recording each object's position. The
//! key map is what lets the active queue:
//!
//!   * `Update` a pod that is already enqueued (re-`heap.Fix` its position),
//!   * `Delete` a pod by key while it sits in the interior of the heap
//!     (`heap.Remove(index)`), and
//!   * `AddIfNotPresent` without producing duplicates.
//!
//! Go's `container/heap` and Rust's [`std::collections::BinaryHeap`] both lack
//! arbitrary-key removal/update — `BinaryHeap` can only pop the root — so this
//! is a genuine structure, not a stdlib analog. The sift-up / sift-down /
//! `fix` / `remove` routines below mirror `container/heap`'s `up`, `down`,
//! `Fix`, and `Remove` exactly, keeping the `index` map consistent on every
//! `Swap`.

use std::collections::HashMap;

/// Error returned when an object is not present in the heap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotFound;

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "object not found in heap")
    }
}

impl std::error::Error for NotFound {}

type KeyFn<T> = Box<dyn Fn(&T) -> String + Send + Sync>;
type LessFn<T> = Box<dyn Fn(&T, &T) -> bool + Send + Sync>;

/// A binary heap keyed by a string identity, mirroring upstream's
/// `pkg/scheduler/internal/heap.Heap`.
///
/// `less(a, b) == true` means `a` is ordered ahead of `b` (pops first),
/// matching the semantics of Go's `heap.Interface.Less`.
pub struct Heap<T> {
    /// key → object.
    items: HashMap<String, T>,
    /// key → current position in `queue` (mirrors `heapItem.index`).
    index: HashMap<String, usize>,
    /// The heap-ordered list of keys (mirrors `data.queue`).
    queue: Vec<String>,
    key_fn: KeyFn<T>,
    less_fn: LessFn<T>,
    /// Mirrors upstream's `metricRecorder` add counter: incremented only on a
    /// genuinely new insertion, never on update or pop.
    adds: u64,
}

impl<T> Heap<T> {
    /// Construct an empty heap with the given key and ordering functions.
    pub fn new(
        key_fn: impl Fn(&T) -> String + Send + Sync + 'static,
        less_fn: impl Fn(&T, &T) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            items: HashMap::new(),
            index: HashMap::new(),
            queue: Vec::new(),
            key_fn: Box::new(key_fn),
            less_fn: Box::new(less_fn),
            adds: 0,
        }
    }

    /// `true` if position `i` orders ahead of position `j`.
    fn less(&self, i: usize, j: usize) -> bool {
        let oi = &self.items[&self.queue[i]];
        let oj = &self.items[&self.queue[j]];
        (self.less_fn)(oi, oj)
    }

    /// Swap two heap positions, keeping the `index` map in sync — the exact
    /// invariant upstream's `data.Swap` maintains.
    fn swap(&mut self, i: usize, j: usize) {
        self.queue.swap(i, j);
        let ki = self.queue[i].clone();
        let kj = self.queue[j].clone();
        self.index.insert(ki, i);
        self.index.insert(kj, j);
    }

    /// `container/heap`'s `up`: sift element `j` toward the root.
    fn up(&mut self, mut j: usize) {
        loop {
            let parent = (j.wrapping_sub(1)) / 2;
            if j == 0 || !self.less(j, parent) {
                break;
            }
            self.swap(parent, j);
            j = parent;
        }
    }

    /// `container/heap`'s `down`: sift element at `i0` toward the leaves,
    /// over the sub-slice `[0, n)`. Returns whether it moved.
    fn down(&mut self, i0: usize, n: usize) -> bool {
        let mut i = i0;
        loop {
            let left = 2 * i + 1;
            if left >= n {
                break;
            }
            // Pick the smaller (higher-priority) of the two children.
            let mut j = left;
            let right = left + 1;
            if right < n && self.less(right, left) {
                j = right;
            }
            if !self.less(j, i) {
                break;
            }
            self.swap(i, j);
            i = j;
        }
        i > i0
    }

    /// `container/heap`'s `Fix`: re-establish ordering after the element at
    /// `i` changed value.
    fn fix(&mut self, i: usize) {
        let n = self.queue.len();
        if !self.down(i, n) {
            self.up(i);
        }
    }

    /// Push a brand-new key/obj onto the back and sift it up. Caller has
    /// already ensured the key is absent.
    fn push_new(&mut self, key: String, obj: T) {
        let n = self.queue.len();
        self.items.insert(key.clone(), obj);
        self.index.insert(key.clone(), n);
        self.queue.push(key);
        self.up(n);
        self.adds += 1;
    }

    /// Add or update `obj` (upstream `Heap.Add`). If the key already exists,
    /// the stored object is replaced and its position re-fixed; otherwise it
    /// is pushed as new and the add counter increments.
    pub fn add(&mut self, obj: T) {
        let key = (self.key_fn)(&obj);
        if let Some(&i) = self.index.get(&key) {
            self.items.insert(key, obj);
            self.fix(i);
        } else {
            self.push_new(key, obj);
        }
    }

    /// `Heap.Update` is an alias for `Add` upstream.
    pub fn update(&mut self, obj: T) {
        self.add(obj);
    }

    /// Add only if the key is not already present (upstream
    /// `Heap.AddIfNotPresent`). Existing entries are left untouched.
    pub fn add_if_not_present(&mut self, obj: T) {
        let key = (self.key_fn)(&obj);
        if !self.index.contains_key(&key) {
            self.push_new(key, obj);
        }
    }

    /// Remove the element at heap position `i` (`container/heap.Remove`),
    /// returning the evicted key.
    fn remove_at(&mut self, i: usize) -> String {
        let n = self.queue.len() - 1;
        if n != i {
            self.swap(i, n);
            // The element now at i may need to move either direction.
            if !self.down(i, n) {
                self.up(i);
            }
        }
        // Pop the (now-last) target element off the back.
        let key = self.queue.pop().unwrap();
        self.index.remove(&key);
        self.items.remove(&key);
        key
    }

    /// Delete `obj` by its key (upstream `Heap.Delete`). Errors if absent.
    pub fn delete(&mut self, obj: &T) -> Result<(), NotFound> {
        let key = (self.key_fn)(obj);
        if self.delete_by_key(&key) {
            Ok(())
        } else {
            Err(NotFound)
        }
    }

    /// Delete by raw key. Returns `true` if something was removed.
    pub fn delete_by_key(&mut self, key: &str) -> bool {
        if let Some(&i) = self.index.get(key) {
            self.remove_at(i);
            true
        } else {
            false
        }
    }

    /// Peek the root without removing it (upstream `Heap.Peek`).
    pub fn peek(&self) -> Option<&T> {
        self.queue.first().map(|k| &self.items[k])
    }

    /// Pop the root (upstream `Heap.Pop`). `container/heap.Pop` swaps root to
    /// the back, sifts the new root down, then truncates.
    pub fn pop(&mut self) -> Option<T> {
        if self.queue.is_empty() {
            return None;
        }
        let n = self.queue.len() - 1;
        self.swap(0, n);
        self.down(0, n);
        let key = self.queue.pop().unwrap();
        self.index.remove(&key);
        self.items.remove(&key)
    }

    /// Look up an object by computing its key (upstream `Heap.Get`).
    pub fn get(&self, obj: &T) -> Option<&T> {
        let key = (self.key_fn)(obj);
        self.items.get(&key)
    }

    /// Look up an object by raw key (upstream `Heap.GetByKey`).
    pub fn get_by_key(&self, key: &str) -> Option<&T> {
        self.items.get(key)
    }

    /// All objects in the heap, in unspecified order (upstream `Heap.List`).
    pub fn list(&self) -> Vec<&T> {
        self.items.values().collect()
    }

    /// Number of elements (upstream `Heap.Len`).
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// `true` if the heap holds no elements.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Count of genuinely new insertions — mirrors the `metricRecorder`
    /// pending-pods gauge increment path.
    pub fn adds(&self) -> u64 {
        self.adds
    }
}
