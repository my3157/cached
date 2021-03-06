/*!
Implementation of various caches

*/

use std::cmp::Eq;
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

use super::Cached;

/// Default unbounded cache
///
/// This cache has no size limit or eviction policy.
///
/// Note: This cache is in-memory only
pub struct UnboundCache<K, V> {
    store: HashMap<K, V>,
    hits: u32,
    misses: u32,
    initial_capacity: Option<usize>,
}

impl<K: Hash + Eq, V> UnboundCache<K, V> {
    /// Creates an empty `UnboundCache`
    pub fn new() -> UnboundCache<K, V> {
        UnboundCache {
            store: Self::new_store(None),
            hits: 0,
            misses: 0,
            initial_capacity: None,
        }
    }

    /// Creates an empty `UnboundCache` with a given pre-allocated capacity
    pub fn with_capacity(size: usize) -> UnboundCache<K, V> {
        UnboundCache {
            store: Self::new_store(Some(size)),
            hits: 0,
            misses: 0,
            initial_capacity: Some(size),
        }
    }

    fn new_store(capacity: Option<usize>) -> HashMap<K, V> {
        capacity.map_or_else(
            || HashMap::new(),
            |size| HashMap::with_capacity(size),
        )
    }
}

impl<K: Hash + Eq, V> Cached<K, V> for UnboundCache<K, V> {
    fn cache_get(&mut self, key: &K) -> Option<&V> {
        match self.store.get(key) {
            Some(v) => {
                self.hits += 1;
                Some(v)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }
    fn cache_set(&mut self, key: K, val: V) {
        self.store.insert(key, val);
    }
    fn cache_remove(&mut self, k: &K) -> Option<V> {
        self.store.remove(k)
    }
    fn cache_clear(&mut self) {
        self.store.clear();
    }
    fn cache_reset(&mut self) {
        self.store = Self::new_store(self.initial_capacity);
    }
    fn cache_size(&self) -> usize {
        self.store.len()
    }
    fn cache_hits(&self) -> Option<u32> {
        Some(self.hits)
    }
    fn cache_misses(&self) -> Option<u32> {
        Some(self.misses)
    }
}

/// Limited functionality doubly linked list using Vec as storage.
struct LRUList<T> {
    values: Vec<ListEntry<T>>,
}

struct ListEntry<T> {
    value: Option<T>,
    next: usize,
    prev: usize,
}

/// Free and occupied cells are each linked into a cyclic list with one auxiliary cell.
/// Cell #0 is on the list of free cells, element #1 is on the list of occupied cells.
///
impl<T> LRUList<T> {
    const FREE: usize = 0;
    const OCCUPIED: usize = 1;

    fn with_capacity(capacity: usize) -> LRUList<T> {
        let mut values = Vec::with_capacity(capacity + 2);
        values.push(ListEntry::<T> {
            value: None,
            next: 0,
            prev: 0,
        });
        values.push(ListEntry::<T> {
            value: None,
            next: 1,
            prev: 1,
        });
        LRUList { values }
    }

    fn unlink(&mut self, index: usize) {
        let prev = self.values[index].prev;
        let next = self.values[index].next;
        self.values[prev].next = next;
        self.values[next].prev = prev;
    }

    fn link_after(&mut self, index: usize, prev: usize) {
        let next = self.values[prev].next;
        self.values[index].prev = prev;
        self.values[index].next = next;
        self.values[prev].next = index;
        self.values[next].prev = index;
    }

    fn move_to_front(&mut self, index: usize) {
        self.unlink(index);
        self.link_after(index, Self::OCCUPIED);
    }

    fn push_front(&mut self, value: Option<T>) -> usize {
        if self.values[Self::FREE].next == Self::FREE {
            self.values.push(ListEntry::<T> {
                value: None,
                next: Self::FREE,
                prev: Self::FREE,
            });
            self.values[Self::FREE].next = self.values.len() - 1;
        }
        let index = self.values[Self::FREE].next;
        self.values[index].value = value;
        self.unlink(index);
        self.link_after(index, Self::OCCUPIED);
        index
    }

    fn remove(&mut self, index: usize) -> T {
        self.unlink(index);
        self.link_after(index, Self::FREE);
        self.values[index].value.take().expect("invalid index")
    }

    fn back(&self) -> usize {
        self.values[Self::OCCUPIED].prev
    }

    fn pop_back(&mut self) -> T {
        let index = self.back();
        self.remove(index)
    }

    fn get(&self, index: usize) -> &T {
        self.values[index].value.as_ref().expect("invalid index")
    }

    fn set(&mut self, index: usize, value: T) {
        self.values[index].value = Some(value);
    }

    fn clear(&mut self) {
        self.values.clear();
        self.values.push(ListEntry::<T> {
            value: None,
            next: 0,
            prev: 0,
        });
        self.values.push(ListEntry::<T> {
            value: None,
            next: 1,
            prev: 1,
        });
    }

    fn iter(&self) -> LRUListIterator<T> {
        LRUListIterator::<T> {
            list: self,
            index: Self::OCCUPIED,
        }
    }
}

struct LRUListIterator<'a, T> {
    list: &'a LRUList<T>,
    index: usize,
}

impl<'a, T> Iterator for LRUListIterator<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.list.values[self.index].next;
        if next == LRUList::<T>::OCCUPIED {
            None
        } else {
            let value = self.list.values[next].value.as_ref();
            self.index = next;
            value
        }
    }
}

/// Least Recently Used / `Sized` Cache
///
/// Stores up to a specified size before beginning
/// to evict the least recently used keys
///
/// Note: This cache is in-memory only
pub struct SizedCache<K, V> {
    store: HashMap<K, usize>,
    order: LRUList<(K, V)>,
    capacity: usize,
    hits: u32,
    misses: u32,
}

impl<K: Hash + Eq, V> SizedCache<K, V> {
    #[deprecated(since = "0.5.1", note = "method renamed to `with_size`")]
    pub fn with_capacity(size: usize) -> SizedCache<K, V> {
        Self::with_size(size)
    }

    /// Creates a new `SizedCache` with a given size limit and pre-allocated backing data
    pub fn with_size(size: usize) -> SizedCache<K, V> {
        if size == 0 {
            panic!("`size` of `SizedCache` must be greater than zero.")
        }
        SizedCache {
            store: HashMap::with_capacity(size),
            order: LRUList::<(K, V)>::with_capacity(size),
            capacity: size,
            hits: 0,
            misses: 0,
        }
    }

    /// Return an iterator of keys in the current order from most
    /// to least recently used.
    pub fn key_order(&self) -> impl Iterator<Item = &K> {
        self.order.iter().map(|(k, _v)| k)
    }

    /// Return an iterator of values in the current order from most
    /// to least recently used.
    pub fn value_order(&self) -> impl Iterator<Item = &V> {
        self.order.iter().map(|(_k, v)| v)
    }
}

impl<K: Hash + Eq + Clone, V> Cached<K, V> for SizedCache<K, V> {
    fn cache_get(&mut self, key: &K) -> Option<&V> {
        let val = self.store.get(key);
        match val {
            Some(&index) => {
                self.order.move_to_front(index);
                self.hits += 1;
                Some(&self.order.get(index).1)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }
    fn cache_set(&mut self, key: K, val: V) {
        if self.store.len() >= self.capacity {
            // store has reached capacity, evict the oldest item.
            // store capacity cannot be zero, so there must be content in `self.order`.
            let (key, _value) = self.order.pop_back();
            self.store
                .remove(&key)
                .expect("SizedCache::cache_set failed evicting cache key");
        }
        let Self { store, order, .. } = self;
        let index = *store
            .entry(key.clone())
            .or_insert_with(|| order.push_front(None));
        order.set(index, (key, val));
    }

    fn cache_remove(&mut self, k: &K) -> Option<V> {
        // try and remove item from mapping, and then from order list if it was in mapping
        if let Some(index) = self.store.remove(k) {
            // need to remove the key in the order list
            let (_key, value) = self.order.remove(index);
            Some(value)
        } else {
            None
        }
    }
    fn cache_clear(&mut self) {
        // clear both the store and the order list
        self.store.clear();
        self.order.clear();
    }
    fn cache_reset(&mut self) {
        // SizedCache uses cache_clear because capacity is fixed.
        self.cache_clear();
    }
    fn cache_size(&self) -> usize {
        self.store.len()
    }
    fn cache_hits(&self) -> Option<u32> {
        Some(self.hits)
    }
    fn cache_misses(&self) -> Option<u32> {
        Some(self.misses)
    }
    fn cache_capacity(&self) -> Option<usize> {
        Some(self.capacity)
    }
}

/// Enum used for defining the status of time-cached values
enum Status {
    NotFound,
    Found,
    Expired,
}

/// Cache store bound by time
///
/// Values are timestamped when inserted and are
/// evicted if expired at time of retrieval.
///
/// Note: This cache is in-memory only
pub struct TimedCache<K, V> {
    store: HashMap<K, (Instant, V)>,
    seconds: u64,
    hits: u32,
    misses: u32,
    initial_capacity: Option<usize>,
}

impl<K: Hash + Eq, V> TimedCache<K, V> {
    /// Creates a new `TimedCache` with a specified lifespan
    pub fn with_lifespan(seconds: u64) -> TimedCache<K, V> {
        TimedCache {
            store: Self::new_store(None),
            seconds: seconds,
            hits: 0,
            misses: 0,
            initial_capacity: None,
        }
    }

    /// Creates a new `TimedCache` with a specified lifespan and
    /// cache-store with the specified pre-allocated capacity
    pub fn with_lifespan_and_capacity(seconds: u64, size: usize) -> TimedCache<K, V> {
        TimedCache {
            store: Self::new_store(Some(size)),
            seconds: seconds,
            hits: 0,
            misses: 0,
            initial_capacity: Some(size),
        }
    }

    fn new_store(capacity: Option<usize>) -> HashMap<K, (Instant, V)> {
        capacity.map_or_else(
            || HashMap::new(),
            |size| HashMap::with_capacity(size),
        )
    }
}

impl<K: Hash + Eq, V> Cached<K, V> for TimedCache<K, V> {
    fn cache_get(&mut self, key: &K) -> Option<&V> {
        let status = {
            let val = self.store.get(key);
            if let Some(&(instant, _)) = val {
                if instant.elapsed().as_secs() < self.seconds {
                    Status::Found
                } else {
                    Status::Expired
                }
            } else {
                Status::NotFound
            }
        };
        match status {
            Status::NotFound => {
                self.misses += 1;
                None
            }
            Status::Found => {
                self.hits += 1;
                self.store.get(key).map(|stamped| &stamped.1)
            }
            Status::Expired => {
                self.misses += 1;
                self.store.remove(key).unwrap();
                None
            }
        }
    }
    fn cache_set(&mut self, key: K, val: V) {
        let stamped = (Instant::now(), val);
        self.store.insert(key, stamped);
    }
    fn cache_remove(&mut self, k: &K) -> Option<V> {
        self.store.remove(k).map(|(_, v)| v)
    }
    fn cache_clear(&mut self) {
        self.store.clear();
    }
    fn cache_reset(&mut self) {
        self.store = Self::new_store(self.initial_capacity);
    }
    fn cache_size(&self) -> usize {
        self.store.len()
    }
    fn cache_hits(&self) -> Option<u32> {
        Some(self.hits)
    }
    fn cache_misses(&self) -> Option<u32> {
        Some(self.misses)
    }
    fn cache_lifespan(&self) -> Option<u64> {
        Some(self.seconds)
    }
}

#[cfg(test)]
/// Cache store tests
mod tests {
    use std::thread::sleep;
    use std::time::Duration;

    use super::Cached;

    use super::SizedCache;
    use super::TimedCache;
    use super::UnboundCache;

    #[test]
    fn basic_cache() {
        let mut c = UnboundCache::new();
        assert!(c.cache_get(&1).is_none());
        let misses = c.cache_misses().unwrap();
        assert_eq!(1, misses);

        c.cache_set(1, 100);
        assert!(c.cache_get(&1).is_some());
        let hits = c.cache_hits().unwrap();
        let misses = c.cache_misses().unwrap();
        assert_eq!(1, hits);
        assert_eq!(1, misses);
    }

    #[test]
    fn sized_cache() {
        let mut c = SizedCache::with_size(5);
        assert!(c.cache_get(&1).is_none());
        let misses = c.cache_misses().unwrap();
        assert_eq!(1, misses);

        c.cache_set(1, 100);
        assert!(c.cache_get(&1).is_some());
        let hits = c.cache_hits().unwrap();
        let misses = c.cache_misses().unwrap();
        assert_eq!(1, hits);
        assert_eq!(1, misses);

        c.cache_set(2, 100);
        c.cache_set(3, 100);
        c.cache_set(4, 100);
        c.cache_set(5, 100);

        assert_eq!(c.key_order().cloned().collect::<Vec<_>>(), [5, 4, 3, 2, 1]);

        c.cache_set(6, 100);
        c.cache_set(7, 100);

        assert_eq!(c.key_order().cloned().collect::<Vec<_>>(), [7, 6, 5, 4, 3]);

        assert!(c.cache_get(&2).is_none());
        assert!(c.cache_get(&3).is_some());

        assert_eq!(c.key_order().cloned().collect::<Vec<_>>(), [3, 7, 6, 5, 4]);

        assert_eq!(2, c.cache_misses().unwrap());
        let size = c.cache_size();
        assert_eq!(5, size);
    }

    #[test]
    /// This is a regression test to confirm that racing cache sets on a SizedCache
    /// do not cause duplicates to exist in the internal `order`. See issue #7
    fn size_cache_racing_keys_eviction_regression() {
        let mut c = SizedCache::with_size(2);
        c.cache_set(1, 100);
        c.cache_set(1, 100);
        // size would be 1, but internal ordered would be [1, 1]
        c.cache_set(2, 100);
        c.cache_set(3, 100);
        // this next set would fail because a duplicate key would be evicted
        c.cache_set(4, 100);
    }

    #[test]
    fn timed_cache() {
        let mut c = TimedCache::with_lifespan(2);
        assert!(c.cache_get(&1).is_none());
        let misses = c.cache_misses().unwrap();
        assert_eq!(1, misses);

        c.cache_set(1, 100);
        assert!(c.cache_get(&1).is_some());
        let hits = c.cache_hits().unwrap();
        let misses = c.cache_misses().unwrap();
        assert_eq!(1, hits);
        assert_eq!(1, misses);

        sleep(Duration::new(2, 0));
        assert!(c.cache_get(&1).is_none());
        let misses = c.cache_misses().unwrap();
        assert_eq!(2, misses);
    }

    #[test]
    fn clear() {
        let mut c = UnboundCache::new();

        c.cache_set(1, 100);
        c.cache_set(2, 200);
        c.cache_set(3, 300);

        // register some hits and misses
        c.cache_get(&1);
        c.cache_get(&2);
        c.cache_get(&3);
        c.cache_get(&10);
        c.cache_get(&20);
        c.cache_get(&30);

        assert_eq!(3, c.cache_size());
        assert_eq!(3, c.cache_hits().unwrap());
        assert_eq!(3, c.cache_misses().unwrap());
        assert_eq!(3, c.store.capacity());

        // clear the cache, should have no more elements
        // hits and misses will still be kept
        c.cache_clear();

        assert_eq!(0, c.cache_size());
        assert_eq!(3, c.cache_hits().unwrap());
        assert_eq!(3, c.cache_misses().unwrap());
        assert_eq!(3, c.store.capacity()); // Keeps the allocated memory for reuse.

        let capacity = 1;
        let mut c = UnboundCache::with_capacity(capacity);
        assert_eq!(capacity, c.store.capacity());

        c.cache_set(1, 100);
        c.cache_set(2, 200);
        c.cache_set(3, 300);

        assert_eq!(3, c.store.capacity());

        c.cache_clear();

        assert_eq!(3, c.store.capacity()); // Keeps the allocated memory for reuse.

        let mut c = SizedCache::with_size(3);

        c.cache_set(1, 100);
        c.cache_set(2, 200);
        c.cache_set(3, 300);
        c.cache_clear();

        assert_eq!(0, c.cache_size());

        let mut c = TimedCache::with_lifespan(3600);

        c.cache_set(1, 100);
        c.cache_set(2, 200);
        c.cache_set(3, 300);
        c.cache_clear();

        assert_eq!(0, c.cache_size());
    }

    #[test]
    fn reset() {
        let mut c = UnboundCache::new();
        c.cache_set(1, 100);
        c.cache_set(2, 200);
        c.cache_set(3, 300);
        assert_eq!(3, c.store.capacity());

        c.cache_reset();

        assert_eq!(0, c.store.capacity());

        let init_capacity = 1;
        let mut c = UnboundCache::with_capacity(init_capacity);
        c.cache_set(1, 100);
        c.cache_set(2, 200);
        c.cache_set(3, 300);
        assert_eq!(3, c.store.capacity());

        c.cache_reset();

        assert_eq!(init_capacity, c.store.capacity());

        let mut c = SizedCache::with_size(init_capacity);
        c.cache_set(1, 100);
        c.cache_set(2, 200);
        c.cache_set(3, 300);
        assert_eq!(init_capacity, c.store.capacity());

        c.cache_reset();

        assert_eq!(init_capacity, c.store.capacity());

        let mut c = TimedCache::with_lifespan(100);
        c.cache_set(1, 100);
        c.cache_set(2, 200);
        c.cache_set(3, 300);
        assert_eq!(3, c.store.capacity());

        c.cache_reset();

        assert_eq!(0, c.store.capacity());

        let mut c = TimedCache::with_lifespan_and_capacity(100, init_capacity);
        c.cache_set(1, 100);
        c.cache_set(2, 200);
        c.cache_set(3, 300);
        assert_eq!(3, c.store.capacity());

        c.cache_reset();

        assert_eq!(init_capacity, c.store.capacity());
}

    #[test]
    fn remove() {
        let mut c = UnboundCache::new();

        c.cache_set(1, 100);
        c.cache_set(2, 200);
        c.cache_set(3, 300);

        // register some hits and misses
        c.cache_get(&1);
        c.cache_get(&2);
        c.cache_get(&3);
        c.cache_get(&10);
        c.cache_get(&20);
        c.cache_get(&30);

        assert_eq!(3, c.cache_size());
        assert_eq!(3, c.cache_hits().unwrap());
        assert_eq!(3, c.cache_misses().unwrap());

        // remove some items from cache
        // hits and misses will still be kept
        assert_eq!(Some(100), c.cache_remove(&1));

        assert_eq!(2, c.cache_size());
        assert_eq!(3, c.cache_hits().unwrap());
        assert_eq!(3, c.cache_misses().unwrap());

        assert_eq!(Some(200), c.cache_remove(&2));

        assert_eq!(1, c.cache_size());

        // removing extra is ok
        assert_eq!(None, c.cache_remove(&2));

        assert_eq!(1, c.cache_size());

        let mut c = SizedCache::with_size(3);

        c.cache_set(1, 100);
        c.cache_set(2, 200);
        c.cache_set(3, 300);

        assert_eq!(Some(100), c.cache_remove(&1));
        assert_eq!(2, c.cache_size());

        assert_eq!(Some(200), c.cache_remove(&2));
        assert_eq!(1, c.cache_size());

        assert_eq!(None, c.cache_remove(&2));
        assert_eq!(1, c.cache_size());

        assert_eq!(Some(300), c.cache_remove(&3));
        assert_eq!(0, c.cache_size());

        let mut c = TimedCache::with_lifespan(3600);

        c.cache_set(1, 100);
        c.cache_set(2, 200);
        c.cache_set(3, 300);

        assert_eq!(Some(100), c.cache_remove(&1));
        assert_eq!(2, c.cache_size());
    }
}
