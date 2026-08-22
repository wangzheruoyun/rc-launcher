//! Object pools for reusable buffers and typed objects (task 25).
//!
//! Low-end Android devices choke on per-frame / per-chunk allocations and the
//! resulting GC + page-cache pressure. These pools recycle `Vec<u8>` capacity
//! (and arbitrary typed objects) across calls so the launcher's hot paths —
//! streaming download writes, `.tar.xz` extraction, AWT frame blits — stay
//! allocation-light.
//!
//! Buffers are bucketed by power-of-two capacity so a tiny request never pins a
//! huge recycled buffer (and vice-versa), keeping the working set bounded. This
//! is the launcher-side analogue of cuberite's bounded `RegionCache`: a
//! fixed-size, reused cache instead of unbounded growth.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// ===========================================================================
// BufPool — reusable byte buffers, bucketed by power-of-two capacity.
// ===========================================================================

/// A reference-counted, clone-cheap pool of reusable `Vec<u8>` buffers.
///
/// Cloning a [`BufPool`] only clones the `Arc` to the shared inner state, so it
/// is cheap to hand a pool to many workers / keep one per render session.
#[derive(Debug, Clone)]
pub struct BufPool {
    inner: Arc<BufPoolInner>,
}

#[derive(Debug)]
struct BufPoolInner {
    /// One queue per power-of-two capacity bucket, from `min_cap` to `max_cap`.
    buckets: Vec<Mutex<VecDeque<Vec<u8>>>>,
    min_cap: usize,
    max_cap: usize,
    min_shift: u32,
    max_idle_per_bucket: usize,
}

impl BufPool {
    /// Default pool: buckets of `256 B .. 64 MiB`, at most `4` idle buffers per
    /// bucket. Covers everything from AWT frame rows to multi-MiB download
    /// chunks.
    pub fn new() -> Self {
        Self::with_config(256, 64 * 1024 * 1024, 4)
    }

    /// Build a pool with explicit bounds. `min_cap`/`max_cap` are rounded up to
    /// the nearest power of two; buckets span every power of two in between.
    pub fn with_config(min_cap: usize, max_cap: usize, max_idle_per_bucket: usize) -> Self {
        let min_cap = min_cap.next_power_of_two().max(16);
        let max_cap = max_cap.max(min_cap).next_power_of_two();
        let min_shift = min_cap.trailing_zeros();
        let max_shift = max_cap.trailing_zeros();
        let n = (max_shift - min_shift + 1) as usize;
        let buckets = (0..n).map(|_| Mutex::new(VecDeque::new())).collect();
        Self {
            inner: Arc::new(BufPoolInner {
                buckets,
                min_cap,
                max_cap,
                min_shift,
                max_idle_per_bucket,
            }),
        }
    }

    /// Capacity of bucket `idx` (a power of two).
    fn bucket_cap(&self, idx: usize) -> usize {
        1usize << (self.inner.min_shift as usize + idx)
    }

    /// Bucket index for a capacity that is (or rounds to) a power of two, or
    /// `None` if it is outside `[min_cap, max_cap]`.
    fn bucket_index_for_cap(&self, cap: usize) -> Option<usize> {
        if cap < self.inner.min_cap || cap > self.inner.max_cap {
            return None;
        }
        let shift = cap.next_power_of_two().trailing_zeros();
        let idx = (shift - self.inner.min_shift) as usize;
        if idx < self.inner.buckets.len() {
            Some(idx)
        } else {
            None
        }
    }

    /// Acquire a buffer whose capacity is at least `min_cap`. Reuses a pooled
    /// buffer when one is available, otherwise allocates a fresh one.
    pub fn acquire(&self, min_cap: usize) -> PooledBuf {
        self.acquire_inner(min_cap)
    }

    fn acquire_inner(&self, min_cap: usize) -> PooledBuf {
        let buf = if min_cap <= self.inner.max_cap {
            // Round the request up to a power of two, but never below the
            // smallest bucket (so tiny requests still reuse a pooled buffer
            // rather than allocating a sliver).
            let cap = min_cap.next_power_of_two().max(self.inner.min_cap);
            let idx = self.bucket_index_for_cap(cap).expect("cap within bounds");
            let bucket_cap = self.bucket_cap(idx);
            let mut guard = self.inner.buckets[idx]
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            match guard.pop_front() {
                Some(mut b) => {
                    // Bucket capacity is a power of two >= min_cap, so this
                    // buffer already satisfies the request.
                    b.clear();
                    b
                }
                None => Vec::with_capacity(bucket_cap),
            }
        } else {
            // Oversized request: allocate fresh, never pooled back.
            Vec::with_capacity(min_cap)
        };
        PooledBuf {
            buf: Some(buf),
            pool: self.clone(),
        }
    }

    /// Return a buffer to its bucket (or drop it if it is out of range / the
    /// bucket is full). Called automatically by [`PooledBuf::drop`].
    fn put_back(&self, buf: Vec<u8>) {
        let cap = buf.capacity();
        if let Some(idx) = self.bucket_index_for_cap(cap) {
            let mut guard = self.inner.buckets[idx]
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if guard.len() < self.inner.max_idle_per_bucket {
                guard.push_back(buf);
                return;
            }
        }
        // Out of range or bucket full — let it drop (capacity is reclaimed).
        drop(buf);
    }

    /// Total number of idle (pooled, available) buffers across all buckets.
    pub fn idle_count(&self) -> usize {
        self.inner
            .buckets
            .iter()
            .map(|b| b.lock().unwrap_or_else(|e| e.into_inner()).len())
            .sum()
    }

    /// Number of capacity buckets the pool manages.
    pub fn bucket_count(&self) -> usize {
        self.inner.buckets.len()
    }
}

impl Default for BufPool {
    fn default() -> Self {
        Self::new()
    }
}

/// A `Vec<u8>` leased from a [`BufPool`].
///
/// On drop the buffer is returned to the pool (capacity preserved) so the next
/// [`BufPool::acquire`] with a similar size reuses it instead of allocating.
pub struct PooledBuf {
    buf: Option<Vec<u8>>,
    pool: BufPool,
}

impl PooledBuf {
    /// Borrow the buffer contents.
    #[allow(clippy::should_implement_trait)]
    pub fn as_ref(&self) -> &[u8] {
        self.buf.as_deref().unwrap_or(&[])
    }

    /// Mutably borrow the underlying `Vec<u8>` (e.g. to pass as `&mut [u8]`).
    #[allow(clippy::should_implement_trait)]
    pub fn as_mut(&mut self) -> &mut Vec<u8> {
        self.buf.as_mut().expect("pooled buffer already taken")
    }

    /// Number of initialised bytes.
    pub fn len(&self) -> usize {
        self.buf.as_ref().map(|b| b.len()).unwrap_or(0)
    }

    /// Whether the initialised length is zero.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Allocated capacity.
    pub fn capacity(&self) -> usize {
        self.buf.as_ref().map(|b| b.capacity()).unwrap_or(0)
    }

    /// Drop all initialised bytes (capacity is kept for reuse).
    pub fn clear(&mut self) -> &mut Self {
        if let Some(b) = self.buf.as_mut() {
            b.clear();
        }
        self
    }

    /// Resize (zero-filled) so `len == need`, growing the capacity if required.
    /// Used by callers that write exactly `need` bytes (frame blits, chunk
    /// copies) so the pooled capacity is reused rather than reallocated.
    pub fn fit(&mut self, need: usize) -> &mut Self {
        if let Some(b) = self.buf.as_mut() {
            if b.capacity() < need {
                *b = Vec::with_capacity(need);
            }
            b.resize(need, 0);
        }
        self
    }

    /// Detach the underlying buffer from the pool. The returned `Vec<u8>` will
    /// NOT be recycled on drop (use when the caller must keep ownership).
    pub fn into_inner(mut self) -> Vec<u8> {
        self.buf.take().expect("pooled buffer already taken")
    }
}

impl Drop for PooledBuf {
    fn drop(&mut self) {
        if let Some(b) = self.buf.take() {
            self.pool.put_back(b);
        }
    }
}

// ===========================================================================
// ObjectPool<T> — reusable arbitrary typed objects.
// ===========================================================================

/// A reference-counted, clone-cheap pool of reusable objects of type `T`.
///
/// Each leased object is returned to the pool on drop after the `reset` closure
/// is applied, so e.g. parse scratch structures or frame metadata can be
/// recycled instead of reallocated on every use.
pub struct ObjectPool<T> {
    inner: Arc<ObjectPoolInner<T>>,
}

impl<T> Clone for ObjectPool<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

struct ObjectPoolInner<T> {
    factory: Box<dyn Fn() -> T + Send + Sync>,
    reset: Box<dyn Fn(&mut T) + Send + Sync>,
    idle: Mutex<VecDeque<T>>,
    max_idle: usize,
}

impl<T> ObjectPool<T> {
    /// Build a pool. `factory` creates a new object when the pool is empty;
    /// `reset` is applied before an object is returned to the pool; at most
    /// `max_idle` objects are kept idle.
    pub fn new<F, R>(factory: F, reset: R, max_idle: usize) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
        R: Fn(&mut T) + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(ObjectPoolInner {
                factory: Box::new(factory),
                reset: Box::new(reset),
                idle: Mutex::new(VecDeque::new()),
                max_idle,
            }),
        }
    }

    /// Lease an object, creating a new one if the pool is empty.
    pub fn acquire(&self) -> PooledObject<T> {
        let obj = {
            let mut g = self.inner.idle.lock().unwrap_or_else(|e| e.into_inner());
            g.pop_front()
        };
        let obj = obj.unwrap_or_else(|| (self.inner.factory)());
        PooledObject {
            obj: Some(obj),
            pool: self.clone(),
        }
    }

    fn put_back(&self, obj: T) {
        let mut g = self.inner.idle.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() < self.inner.max_idle {
            g.push_back(obj);
        }
        // else: drop (reclaim).
    }

    /// Number of idle objects currently pooled.
    pub fn idle_count(&self) -> usize {
        self.inner
            .idle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }
}

/// An object of type `T` leased from an [`ObjectPool`]. Dereferences to `T` and
/// returns to the pool (after `reset`) on drop.
pub struct PooledObject<T> {
    obj: Option<T>,
    pool: ObjectPool<T>,
}

impl<T> PooledObject<T> {
    #[allow(clippy::should_implement_trait)]
    pub fn as_ref(&self) -> &T {
        self.obj.as_ref().expect("pooled object already taken")
    }
    #[allow(clippy::should_implement_trait)]
    pub fn as_mut(&mut self) -> &mut T {
        self.obj.as_mut().expect("pooled object already taken")
    }
    /// Detach ownership from the pool (will NOT be recycled on drop).
    pub fn into_inner(mut self) -> T {
        self.obj.take().expect("pooled object already taken")
    }
}

impl<T> std::ops::Deref for PooledObject<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.obj.as_ref().expect("pooled object already taken")
    }
}

impl<T> std::ops::DerefMut for PooledObject<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.obj.as_mut().expect("pooled object already taken")
    }
}

impl<T> Drop for PooledObject<T> {
    fn drop(&mut self) {
        if let Some(mut o) = self.obj.take() {
            (self.pool.inner.reset)(&mut o);
            self.pool.put_back(o);
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buf_pool_reuses_capacity() {
        let pool = BufPool::new();
        assert_eq!(pool.idle_count(), 0);
        let mut a = pool.acquire(1024);
        a.fit(1024);
        assert_eq!(a.len(), 1024);
        let cap = a.capacity();
        // lease out, then drop -> returns to pool
        drop(a);
        assert_eq!(pool.idle_count(), 1);
        // a request that rounds into the *same* 1024 bucket reuses the pooled
        // buffer (capacity preserved, length reset to 0 on acquire).
        let b = pool.acquire(800);
        assert_eq!(b.capacity(), cap);
        assert_eq!(b.len(), 0);
        assert_eq!(pool.idle_count(), 0);
    }

    #[test]
    fn buf_pool_respects_bucket_bounds() {
        let pool = BufPool::with_config(256, 4096, 2);
        // two buffers in the same bucket fill it; a third is allocated and then
        // dropped (bucket full -> not stored).
        let _x = pool.acquire(300);
        let _y = pool.acquire(300);
        assert_eq!(pool.idle_count(), 0); // not yet returned
        drop(_x);
        drop(_y);
        assert_eq!(pool.idle_count(), 2);
        // oversized request bypasses the pool entirely
        let big = pool.acquire(1 << 30);
        assert!(big.capacity() >= (1 << 30));
    }

    #[test]
    fn buf_pool_fit_grows_only_when_needed() {
        let pool = BufPool::new();
        let mut b = pool.acquire(64);
        b.fit(64);
        // 64 < min bucket (256) so the buffer is sized to the smallest bucket.
        let first_cap = b.capacity();
        assert!(first_cap >= 64);
        drop(b);
        let mut b2 = pool.acquire(64);
        b2.fit(64);
        assert_eq!(b2.capacity(), first_cap);
        // a larger fit reallocates and the new capacity is reused next time
        b2.fit(8192);
        let grown = b2.capacity();
        assert!(grown >= 8192);
        drop(b2);
        let b3 = pool.acquire(8192);
        assert_eq!(b3.capacity(), grown);
    }

    #[test]
    fn object_pool_recycles() {
        let pool: ObjectPool<Vec<u32>> =
            ObjectPool::new(|| Vec::with_capacity(8), |v| v.clear(), 4);
        assert_eq!(pool.idle_count(), 0);
        {
            let mut o = pool.acquire();
            o.push(1);
            o.push(2);
        }
        assert_eq!(pool.idle_count(), 1);
        let o2 = pool.acquire();
        // reset ran on return, so the recycled object is empty
        assert!(o2.is_empty());
        assert_eq!(o2.capacity(), 8);
    }

    #[test]
    fn pooled_object_derefs_to_t() {
        let pool: ObjectPool<String> = ObjectPool::new(String::new, |s| s.clear(), 2);
        let mut o = pool.acquire();
        o.push_str("hi");
        assert_eq!(o.len(), 2);
        assert!(o.contains('i'));
    }
}
