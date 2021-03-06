use crate::{
    arena::Arena,
    hasher::{HashMap, RandomState},
    internable::Internable,
    key::{Key, Spur},
    reader::RodeoReader,
    resolver::RodeoResolver,
};

use core::{
    hash::{BuildHasher, Hash},
    iter, mem,
    sync::atomic::{AtomicUsize, Ordering},
};
use dashmap::DashMap;
use std::sync::Mutex;

/// A concurrent string interner that caches strings quickly with a minimal memory footprint,
/// returning a unique key to re-access it with `O(1)` internment and resolution.
///
/// This struct is only avaliable with the `multi-threaded` feature!  
/// By default ThreadedRodeo uses the [`Spur`] type for keys and [`RandomState`] as the hasher
///
/// [`Spur`]: crate::Spur
/// [`ahash::RandomState`]: https://docs.rs/ahash/0.3.2/ahash/struct.RandomState.html
/// [`RandomState`]: index.html#cargo-features
#[derive(Debug)]
pub struct ThreadedRodeo<V = str, K = Spur, S = RandomState>
where
    V: Internable + ?Sized,
    K: Key + Hash,
    S: BuildHasher + Clone,
{
    /// Map that allows str to key resolution
    map: DashMap<&'static V, K, S>,
    /// Map that allows key to str resolution
    strings: DashMap<K, &'static V, S>,
    /// The current key value
    key: AtomicUsize,
    /// The arena where all strings are stored
    arena: Mutex<Arena<V::Raw>>,
}

// TODO: More parity functions with std::HashMap

impl<V, K> ThreadedRodeo<V, K, RandomState>
where
    V: Internable + ?Sized,
    K: Key + Hash,
{
    /// Create a new ThreadedRodeo
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::{ThreadedRodeo, Spur};
    /// use std::{thread, sync::Arc};
    ///
    /// let lasso: Arc<ThreadedRodeo<str, Spur>> = Arc::new(ThreadedRodeo::new());
    /// let hello = lasso.get_or_intern("Hello, ");
    ///
    /// let l = Arc::clone(&lasso);
    /// let world = thread::spawn(move || {
    ///     l.get_or_intern("World!")
    /// })
    /// .join()
    /// .unwrap();
    ///
    /// assert_eq!("Hello, ", lasso.resolve(&hello));
    /// assert_eq!("World!", lasso.resolve(&world));
    /// ```
    ///
    #[inline]
    pub fn new() -> Self {
        Self {
            map: DashMap::with_hasher(RandomState::new()),
            strings: DashMap::with_hasher(RandomState::new()),
            key: AtomicUsize::new(0),
            arena: Mutex::new(Arena::new()),
        }
    }

    /// Create a new ThreadedRodeo with the specified capacity. The interner will be able to hold `capacity`
    /// strings without reallocating. If capacity is 0, the interner will not allocate.
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::{ThreadedRodeo, Spur};
    ///
    /// let rodeo: ThreadedRodeo<str, Spur> = ThreadedRodeo::with_capacity(10);
    /// ```
    ///
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            map: DashMap::with_capacity_and_hasher(capacity, RandomState::new()),
            strings: DashMap::with_capacity_and_hasher(capacity, RandomState::new()),
            key: AtomicUsize::new(0),
            arena: Mutex::new(Arena::new()),
        }
    }
}

impl<V, K, S> ThreadedRodeo<V, K, S>
where
    V: Internable + ?Sized,
    K: Key + Hash,
    S: BuildHasher + Clone,
{
    /// Creates an empty ThreadedRodeo which will use the given hasher for its internal hashmap
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::{Spur, ThreadedRodeo};
    /// use std::collections::hash_map::RandomState;
    ///
    /// let rodeo: ThreadedRodeo<str, Spur, RandomState> = ThreadedRodeo::with_hasher(RandomState::new());
    /// ```
    ///
    #[inline]
    pub fn with_hasher(hash_builder: S) -> Self {
        Self {
            map: DashMap::with_hasher(hash_builder.clone()),
            strings: DashMap::with_hasher(hash_builder),
            key: AtomicUsize::new(0),
            arena: Mutex::new(Arena::new()),
        }
    }

    /// Creates a new ThreadedRodeo with the specified capacity that will use the given hasher for its internal hashmap
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::{Spur, ThreadedRodeo};
    /// use std::collections::hash_map::RandomState;
    ///
    /// let rodeo: ThreadedRodeo<str, Spur, RandomState> = ThreadedRodeo::with_capacity_and_hasher(10, RandomState::new());
    /// ```
    ///
    #[inline]
    pub fn with_capacity_and_hasher(capacity: usize, hash_builder: S) -> Self {
        Self {
            map: DashMap::with_capacity_and_hasher(capacity, hash_builder.clone()),
            strings: DashMap::with_capacity_and_hasher(capacity, hash_builder),
            key: AtomicUsize::new(0),
            arena: Mutex::new(Arena::new()),
        }
    }

    /// Get the key for a string, interning it if it does not yet exist
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::ThreadedRodeo;
    ///
    /// let rodeo = ThreadedRodeo::default();
    ///
    /// // Interned the string
    /// let key = rodeo.get_or_intern("Strings of things with wings and dings");
    /// assert_eq!("Strings of things with wings and dings", rodeo.resolve(&key));
    ///
    /// // No string was interned, as it was already contained
    /// let key = rodeo.get_or_intern("Strings of things with wings and dings");
    /// assert_eq!("Strings of things with wings and dings", rodeo.resolve(&key));
    /// ```
    ///
    #[inline]
    pub fn get_or_intern<T>(&self, val: T) -> K
    where
        T: AsRef<V>,
    {
        if let Some(key) = self.map.get(val.as_ref()) {
            *key
        } else {
            let shard = self.map.determine_map(val.as_ref());
            // Safety: The indices provided by DashMap always refer to a shard in it's shards
            let shard = unsafe { self.map.shards().get_unchecked(shard) };

            if let Some(key) = shard.read().get(val.as_ref()) {
                return *key.get();
            }

            // Safety: The drop impl removes all references before the arena is dropped
            let string: &'static V = unsafe {
                V::from_raw(
                    self.arena
                        .lock()
                        .unwrap()
                        .store_slice(val.as_ref().to_raw()),
                )
            };
            let key = K::try_from_usize(self.key.fetch_add(1, Ordering::SeqCst))
                .expect("Failed to get or intern string");

            self.map.insert(string, key);
            self.strings.insert(key, string);

            key
        }
    }

    /// Get the key for a string, interning it if it does not yet exist
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::ThreadedRodeo;
    ///
    /// let rodeo = ThreadedRodeo::default();
    ///
    /// // Interned the string
    /// let key = rodeo.get_or_intern("Strings of things with wings and dings");
    /// assert_eq!("Strings of things with wings and dings", rodeo.resolve(&key));
    ///
    /// // No string was interned, as it was already contained
    /// let key = rodeo.get_or_intern("Strings of things with wings and dings");
    /// assert_eq!("Strings of things with wings and dings", rodeo.resolve(&key));
    /// ```
    ///
    #[inline]
    pub fn try_get_or_intern<T>(&self, val: T) -> Option<K>
    where
        T: AsRef<V>,
    {
        if let Some(key) = self.map.get(val.as_ref()) {
            Some(*key)
        } else {
            let shard = self.map.determine_map(val.as_ref());
            // Safety: The indices provided by DashMap always refer to a shard in it's shards
            let shard = unsafe { self.map.shards().get_unchecked(shard) };

            if let Some(key) = shard.read().get(val.as_ref()) {
                return Some(*key.get());
            }

            // Safety: The drop impl removes all references before the arena is dropped
            let string: &'static V = unsafe {
                V::from_raw(
                    self.arena
                        .lock()
                        .unwrap()
                        .store_slice(val.as_ref().to_raw()),
                )
            };
            let key = K::try_from_usize(self.key.fetch_add(1, Ordering::SeqCst))?;

            self.map.insert(string, key);
            self.strings.insert(key, string);

            Some(key)
        }
    }

    /// Get the key value of a string, returning `None` if it doesn't exist
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::ThreadedRodeo;
    ///
    /// let rodeo = ThreadedRodeo::default();
    ///
    /// let key = rodeo.get_or_intern("Strings of things with wings and dings");
    /// assert_eq!(Some(key), rodeo.get("Strings of things with wings and dings"));
    ///
    /// assert_eq!(None, rodeo.get("This string isn't interned"));
    /// ```
    ///
    #[inline]
    pub fn get<T>(&self, val: T) -> Option<K>
    where
        T: AsRef<V>,
    {
        self.map.get(val.as_ref()).map(|k| *k)
    }

    /// Resolves a string by its key. Only keys made by the current ThreadedRodeo may be used
    ///
    /// # Panics
    ///
    /// Panics if the key is out of bounds
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::ThreadedRodeo;
    ///
    /// let rodeo = ThreadedRodeo::default();
    ///
    /// let key = rodeo.get_or_intern("Strings of things with wings and dings");
    /// assert_eq!("Strings of things with wings and dings", rodeo.resolve(&key));
    /// ```
    ///
    #[inline]
    pub fn resolve<'a>(&'a self, key: &K) -> &'a V {
        &*self.strings.get(key).expect("Key out of bounds")
    }

    /// Resolves a string by its key, returning `None` if it is out of bounds. Only keys made by the current
    /// ThreadedRodeo may be used
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::ThreadedRodeo;
    ///
    /// let rodeo = ThreadedRodeo::default();
    ///
    /// let key = rodeo.get_or_intern("Strings of things with wings and dings");
    /// assert_eq!(Some("Strings of things with wings and dings"), rodeo.try_resolve(&key));
    /// ```
    ///
    #[inline]
    pub fn try_resolve<'a>(&'a self, key: &K) -> Option<&'a V> {
        self.strings.get(key).map(|s| *s)
    }

    /// Gets the number of interned strings
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::ThreadedRodeo;
    ///
    /// let rodeo = ThreadedRodeo::default();
    /// rodeo.get_or_intern("Documentation often has little hidden bits in it");
    ///
    /// assert_eq!(rodeo.len(), 1);
    /// ```
    ///
    #[inline]
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Returns `true` if there are no currently interned strings
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::ThreadedRodeo;
    ///
    /// let rodeo = ThreadedRodeo::default();
    /// assert!(rodeo.is_empty());
    /// ```
    ///
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of strings that can be interned without a reallocation
    ///
    /// # Example
    ///
    /// ```no_run
    /// # // Note: The capacity of DashMap isn't reliable
    /// use lasso::{Spur, ThreadedRodeo};
    ///
    /// let rodeo: ThreadedRodeo<str, Spur> = ThreadedRodeo::with_capacity(10);
    /// assert_eq!(rodeo.capacity(), 10);
    /// ```
    ///
    #[inline]
    pub fn capacity(&self) -> usize {
        self.strings.capacity()
    }

    /// Consumes the current ThreadedRodeo, returning a [`RodeoReader`] to allow contention-free access of the interner
    /// from multiple threads
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::ThreadedRodeo;
    ///
    /// let rodeo = ThreadedRodeo::default();
    /// let key = rodeo.get_or_intern("Appear weak when you are strong, and strong when you are weak.");
    ///
    /// let rodeo_reader = rodeo.into_reader();
    /// assert_eq!(
    ///     "Appear weak when you are strong, and strong when you are weak.",
    ///     rodeo_reader.resolve(&key),
    /// );
    /// ```
    ///
    /// [`RodeoReader`]: crate::RodeoReader
    #[inline]
    #[must_use]
    pub fn into_reader(self) -> RodeoReader<V, K, S> {
        // Take the strings vec from the old lasso
        let mut strings = iter::from_fn(|| Some(None))
            .take(self.strings.len())
            .collect::<Vec<Option<&'static V>>>();

        for shard in self.strings.shards() {
            for (key, val) in shard.write().drain() {
                // Safety: The keys of the dashmap should be valid indices
                unsafe {
                    // TODO: get_unchecked?
                    strings[key.into_usize()] = Some(val.into_inner());
                }
            }
        }

        // Drain the DashMap by draining each of its buckets and creating a new hashmap to store their values
        let mut map: HashMap<&'static V, K, S> =
            HashMap::with_capacity_and_hasher(strings.len(), self.map.hasher().clone());

        for shard in self.map.shards() {
            // Extend the new map by the contents of the shard
            map.extend(shard.write().drain().map(|(k, v)| (k, v.into_inner())));
        }

        // Safety: No other references outside of `map` and `strings` to the interned strings exist
        unsafe {
            RodeoReader::new(
                map,
                strings.into_iter().map(|s| s.unwrap()).collect(),
                mem::take(&mut *self.arena.lock().unwrap()),
            )
        }
    }

    /// Consumes the current ThreadedRodeo, returning a [`RodeoResolver`] to allow contention-free access of the interner
    /// from multiple threads with the lowest possible memory consumption
    ///
    /// # Example
    ///
    /// ```rust
    /// use lasso::ThreadedRodeo;
    ///
    /// let rodeo = ThreadedRodeo::default();
    /// let key = rodeo.get_or_intern("Appear weak when you are strong, and strong when you are weak.");
    ///
    /// let rodeo_resolver = rodeo.into_resolver();
    /// assert_eq!(
    ///     "Appear weak when you are strong, and strong when you are weak.",
    ///     rodeo_resolver.resolve(&key),
    /// );
    /// ```
    ///
    /// [`RodeoResolver`]: crate::RodeoResolver
    #[inline]
    #[must_use]
    pub fn into_resolver(self) -> RodeoResolver<V, K> {
        self.map.clear();

        let mut strings = iter::from_fn(|| Some(None))
            .take(self.strings.len())
            .collect::<Vec<Option<&'static V>>>();

        for shard in self.strings.shards() {
            for (key, val) in shard.write().drain() {
                // Safety: The keys of the dashmap should be valid indices
                unsafe {
                    // TODO: get_unchecked?
                    strings[key.into_usize()] = Some(val.into_inner());
                }
            }
        }

        // Safety: No other references to the strings exist
        unsafe {
            RodeoResolver::new(
                strings.into_iter().map(|s| s.unwrap()).collect(),
                mem::take(&mut *self.arena.lock().unwrap()),
            )
        }
    }
}

/// Creates a ThreadedRodeo using [`Spur`] as its key and [`RandomState`] as its hasher
///
/// [`Spur`]: crate::Spur
/// [`RandomState`]: index.html#cargo-features
impl Default for ThreadedRodeo<str, Spur, RandomState> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// Deallocate the leaked strings interned by ThreadedRodeo
impl<V, K, S> Drop for ThreadedRodeo<V, K, S>
where
    V: Internable + ?Sized,
    K: Key + Hash,
    S: BuildHasher + Clone,
{
    #[inline]
    fn drop(&mut self) {
        // Clear the map to remove all other references to the strings in self.strings
        self.map.clear();

        // Drain self.strings while deallocating the strings it holds
        for shard in self.strings.shards() {
            shard.write().drain().for_each(drop);
        }
    }
}

unsafe impl<V, K, S> Sync for ThreadedRodeo<V, K, S>
where
    V: Internable + ?Sized + Sync,
    K: Key + Hash + Sync,
    S: BuildHasher + Clone + Sync,
{
}

unsafe impl<V, K, S> Send for ThreadedRodeo<V, K, S>
where
    V: Internable + ?Sized + Send,
    K: Key + Hash + Send,
    S: BuildHasher + Clone + Send,
{
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hasher::RandomState, MicroSpur};

    #[cfg(not(any(miri, feature = "no-std")))]
    use std::{sync::Arc, thread};

    compile! {
        if #[feature = "no-std"] {
            use alloc::string::ToString;
        } else {
            use std::string::ToString;
        }
    }

    #[test]
    fn new() {
        let _: ThreadedRodeo<str, Spur> = ThreadedRodeo::new();
    }

    // Capacity of DashMap isn't reliable
    // #[test]
    // fn with_capacity() {
    //     let rodeo: ThreadedRodeo<Spur> = ThreadedRodeo::with_capacity(10);
    //     assert_eq!(10, rodeo.capacity());
    // }

    #[test]
    fn with_hasher() {
        let rodeo: ThreadedRodeo<str, Spur, RandomState> =
            ThreadedRodeo::with_hasher(RandomState::new());

        let key = rodeo.get_or_intern("Test");
        assert_eq!("Test", rodeo.resolve(&key));
    }

    #[test]
    fn with_capacity_and_hasher() {
        let rodeo: ThreadedRodeo<str, Spur, RandomState> =
            ThreadedRodeo::with_capacity_and_hasher(10, RandomState::new());

        let key = rodeo.get_or_intern("Test");
        assert_eq!("Test", rodeo.resolve(&key));
    }

    #[test]
    fn get_or_intern() {
        let rodeo = ThreadedRodeo::default();

        let a = rodeo.get_or_intern("A");
        assert_eq!(a, rodeo.get_or_intern("A"));

        let b = rodeo.get_or_intern("B");
        assert_eq!(b, rodeo.get_or_intern("B"));

        let c = rodeo.get_or_intern("C");
        assert_eq!(c, rodeo.get_or_intern("C"));
    }

    #[test]
    #[cfg(not(any(miri, feature = "no-std")))]
    fn get_or_intern_threaded() {
        let rodeo = Arc::new(ThreadedRodeo::default());

        let moved = Arc::clone(&rodeo);
        thread::spawn(move || {
            let a = moved.get_or_intern("A");
            assert_eq!(a, moved.get_or_intern("A"));

            let b = moved.get_or_intern("B");
            assert_eq!(b, moved.get_or_intern("B"));

            let c = moved.get_or_intern("C");
            assert_eq!(c, moved.get_or_intern("C"));
        });

        let a = rodeo.get_or_intern("A");
        assert_eq!(a, rodeo.get_or_intern("A"));

        let b = rodeo.get_or_intern("B");
        assert_eq!(b, rodeo.get_or_intern("B"));

        let c = rodeo.get_or_intern("C");
        assert_eq!(c, rodeo.get_or_intern("C"));
    }

    #[test]
    fn try_get_or_intern() {
        let rodeo: ThreadedRodeo<str, MicroSpur> = ThreadedRodeo::new();

        for i in 0..u8::max_value() as usize - 1 {
            rodeo.get_or_intern(i.to_string());
        }

        let space = rodeo.try_get_or_intern("A").unwrap();
        assert_eq!("A", rodeo.resolve(&space));

        assert!(rodeo.try_get_or_intern("C").is_none());
    }

    #[test]
    #[cfg(not(any(miri, feature = "no-std")))]
    fn try_get_or_intern_threaded() {
        let rodeo: Arc<ThreadedRodeo<str, MicroSpur>> = Arc::new(ThreadedRodeo::new());

        for i in 0..u8::max_value() as usize - 1 {
            rodeo.get_or_intern(i.to_string());
        }

        let moved = Arc::clone(&rodeo);
        thread::spawn(move || {
            let a = moved.try_get_or_intern("A");
            assert_eq!(a, moved.try_get_or_intern("A"));

            let b = moved.try_get_or_intern("B");
            assert_eq!(b, moved.try_get_or_intern("B"));

            let c = moved.try_get_or_intern("C");
            assert_eq!(c, moved.try_get_or_intern("C"));
        });

        let a = rodeo.try_get_or_intern("A");
        assert_eq!(a, rodeo.try_get_or_intern("A"));

        let b = rodeo.try_get_or_intern("B");
        assert_eq!(b, rodeo.try_get_or_intern("B"));

        let c = rodeo.try_get_or_intern("C");
        assert_eq!(c, rodeo.try_get_or_intern("C"));
    }

    #[test]
    fn get() {
        let rodeo = ThreadedRodeo::default();
        let key = rodeo.get_or_intern("A");

        assert_eq!(Some(key), rodeo.get("A"));
    }

    #[test]
    #[cfg(not(any(miri, feature = "no-std")))]
    fn get_threaded() {
        let rodeo = Arc::new(ThreadedRodeo::default());
        let key = rodeo.get_or_intern("A");

        let moved = Arc::clone(&rodeo);
        thread::spawn(move || {
            assert_eq!(Some(key), moved.get("A"));
        });

        assert_eq!(Some(key), rodeo.get("A"));
    }

    #[test]
    fn resolve() {
        let rodeo = ThreadedRodeo::default();
        let key = rodeo.get_or_intern("A");

        assert_eq!("A", rodeo.resolve(&key));
    }

    #[test]
    #[should_panic]
    #[cfg(not(miri))]
    fn resolve_panics() {
        let rodeo = ThreadedRodeo::default();
        rodeo.resolve(&Spur::try_from_usize(100).unwrap());
    }

    #[test]
    #[cfg(not(any(miri, feature = "no-std")))]
    fn resolve_threaded() {
        let rodeo = Arc::new(ThreadedRodeo::default());
        let key = rodeo.get_or_intern("A");

        let moved = Arc::clone(&rodeo);
        thread::spawn(move || {
            assert_eq!("A", moved.resolve(&key));
        });

        assert_eq!("A", rodeo.resolve(&key));
    }

    #[test]
    #[cfg(not(any(miri, feature = "no-std", tarpaulin)))]
    fn resolve_panics_threaded() {
        let rodeo = Arc::new(ThreadedRodeo::default());
        let key = rodeo.get_or_intern("A");

        let moved = Arc::clone(&rodeo);
        let handle = thread::spawn(move || {
            assert_eq!("A", moved.resolve(&key));
            moved.resolve(&Spur::try_from_usize(100).unwrap());
        });

        assert_eq!("A", rodeo.resolve(&key));
        assert!(handle.join().is_err());
    }

    #[test]
    fn try_resolve() {
        let rodeo = ThreadedRodeo::default();
        let key = rodeo.get_or_intern("A");

        assert_eq!(Some("A"), rodeo.try_resolve(&key));
        assert_eq!(None, rodeo.try_resolve(&Spur::try_from_usize(100).unwrap()));
    }

    #[test]
    #[cfg(not(any(miri, feature = "no-std")))]
    fn try_resolve_threaded() {
        let rodeo = Arc::new(ThreadedRodeo::default());
        let key = rodeo.get_or_intern("A");

        let moved = Arc::clone(&rodeo);
        thread::spawn(move || {
            assert_eq!(Some("A"), moved.try_resolve(&key));
            assert_eq!(None, moved.try_resolve(&Spur::try_from_usize(100).unwrap()));
        });

        assert_eq!(Some("A"), rodeo.try_resolve(&key));
        assert_eq!(None, rodeo.try_resolve(&Spur::try_from_usize(100).unwrap()));
    }

    #[test]
    fn len() {
        let rodeo = ThreadedRodeo::default();
        rodeo.get_or_intern("A");
        rodeo.get_or_intern("B");
        rodeo.get_or_intern("C");

        assert_eq!(rodeo.len(), 3);
    }

    #[test]
    fn empty() {
        let rodeo = ThreadedRodeo::default();

        assert!(rodeo.is_empty());
    }

    // #[test]
    // fn clone() {
    //     let rodeo = ThreadedRodeo::default();
    //     let key = rodeo.get_or_intern("Test");
    //
    //     assert_eq!("Test", rodeo.resolve(&key));
    //
    //     let cloned = rodeo.clone();
    //     assert_eq!("Test", cloned.resolve(&key));
    //
    //     drop(rodeo);
    //
    //     assert_eq!("Test", cloned.resolve(&key));
    // }

    #[test]
    fn drops() {
        let _ = ThreadedRodeo::default();
    }

    #[test]
    #[cfg(not(any(miri, feature = "no-std")))]
    fn drop_threaded() {
        let rodeo = Arc::new(ThreadedRodeo::default());

        let moved = Arc::clone(&rodeo);
        thread::spawn(move || {
            let _ = moved;
        });
    }

    #[test]
    #[cfg(not(any(miri, feature = "no-std")))]
    fn debug() {
        let rodeo = ThreadedRodeo::default();
        println!("{:?}", rodeo);
    }

    #[test]
    fn into_resolver() {
        let rodeo = ThreadedRodeo::default();
        let key = rodeo.get_or_intern("A");

        let resolver = rodeo.into_resolver();
        assert_eq!("A", resolver.resolve(&key));
    }

    #[test]
    fn into_reader() {
        let rodeo = ThreadedRodeo::default();
        let key = rodeo.get_or_intern("A");

        let reader = rodeo.into_reader();
        assert_eq!("A", reader.resolve(&key));
    }
}
