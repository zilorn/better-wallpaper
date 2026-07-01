use std::collections::HashMap;

/// Hard safety ceiling. Quality settings may select a lower budget but cannot raise this limit.
pub const MAX_GPU_CACHE_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Resource identity that remains stable across reloads while invalidating changed content.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GpuResourceKey {
    pub path: String,
    pub content_hash: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GpuCacheError {
    #[error("GPU cache budget must be between 1 and {max} bytes, got {requested}")]
    InvalidBudget { requested: u64, max: u64 },
    #[error("GPU resource {path} is {size} bytes and exceeds the cache budget of {budget} bytes")]
    ResourceTooLarge {
        path: String,
        size: u64,
        budget: u64,
    },
}

#[derive(Debug)]
struct CachedResource<T> {
    resource: T,
    size: u64,
    last_used: u64,
}

/// Owning GPU resource cache. Eviction drops the resource immediately, allowing its backend
/// allocation wrapper to enqueue or perform the appropriate GPU destruction.
#[derive(Debug)]
pub struct GpuResourceCache<T> {
    budget: u64,
    used: u64,
    clock: u64,
    entries: HashMap<GpuResourceKey, CachedResource<T>>,
}

impl<T> GpuResourceCache<T> {
    pub fn new(budget: u64) -> Result<Self, GpuCacheError> {
        if budget == 0 || budget > MAX_GPU_CACHE_BUDGET_BYTES {
            return Err(GpuCacheError::InvalidBudget {
                requested: budget,
                max: MAX_GPU_CACHE_BUDGET_BYTES,
            });
        }
        Ok(Self {
            budget,
            used: 0,
            clock: 0,
            entries: HashMap::new(),
        })
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&mut self, key: &GpuResourceKey) -> Option<&T> {
        self.clock = self.clock.saturating_add(1);
        let entry = self.entries.get_mut(key)?;
        entry.last_used = self.clock;
        Some(&entry.resource)
    }

    pub fn insert(
        &mut self,
        key: GpuResourceKey,
        size: u64,
        resource: T,
    ) -> Result<(), GpuCacheError> {
        if size > self.budget {
            return Err(GpuCacheError::ResourceTooLarge {
                path: key.path,
                size,
                budget: self.budget,
            });
        }

        if let Some(previous) = self.entries.remove(&key) {
            self.used -= previous.size;
        }
        while self.used.saturating_add(size) > self.budget {
            let Some(eviction_key) = self
                .entries
                .iter()
                .min_by(|(left_key, left), (right_key, right)| {
                    left.last_used
                        .cmp(&right.last_used)
                        .then_with(|| left_key.path.cmp(&right_key.path))
                        .then_with(|| left_key.content_hash.cmp(&right_key.content_hash))
                })
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&eviction_key) {
                self.used -= evicted.size;
                tracing::debug!(
                    resource = %eviction_key.path,
                    bytes = evicted.size,
                    "Evicted scene resource from GPU cache"
                );
            }
        }

        self.clock = self.clock.saturating_add(1);
        self.used += size;
        self.entries.insert(
            key,
            CachedResource {
                resource,
                size,
                last_used: self.clock,
            },
        );
        Ok(())
    }

    pub fn remove(&mut self, key: &GpuResourceKey) -> Option<T> {
        let entry = self.entries.remove(key)?;
        self.used -= entry.size;
        Some(entry.resource)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.used = 0;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    #[derive(Debug)]
    struct TrackedResource(Arc<AtomicUsize>);

    impl Drop for TrackedResource {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn key(path: &str) -> GpuResourceKey {
        GpuResourceKey {
            path: path.into(),
            content_hash: format!("hash-{path}"),
        }
    }

    #[test]
    fn enforces_budget_and_evicts_the_least_recently_used_resource() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut cache = GpuResourceCache::new(6).unwrap();
        let first = key("first");
        let second = key("second");
        let third = key("third");
        cache
            .insert(first.clone(), 3, TrackedResource(Arc::clone(&drops)))
            .unwrap();
        cache
            .insert(second.clone(), 3, TrackedResource(Arc::clone(&drops)))
            .unwrap();
        assert!(cache.get(&first).is_some());

        cache
            .insert(third.clone(), 3, TrackedResource(Arc::clone(&drops)))
            .unwrap();
        assert!(cache.get(&second).is_none());
        assert!(cache.get(&first).is_some());
        assert!(cache.get(&third).is_some());
        assert_eq!(cache.used(), 6);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn replacement_removal_and_clear_keep_exact_accounting() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut cache = GpuResourceCache::new(5).unwrap();
        let same = key("same");
        cache
            .insert(same.clone(), 4, TrackedResource(Arc::clone(&drops)))
            .unwrap();
        cache
            .insert(same.clone(), 2, TrackedResource(Arc::clone(&drops)))
            .unwrap();
        assert_eq!(cache.used(), 2);
        assert_eq!(drops.load(Ordering::Relaxed), 1);

        drop(cache.remove(&same));
        assert_eq!(cache.used(), 0);
        assert_eq!(drops.load(Ordering::Relaxed), 2);

        cache
            .insert(key("last"), 1, TrackedResource(Arc::clone(&drops)))
            .unwrap();
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(drops.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn rejects_invalid_budgets_and_oversized_resources_without_mutation() {
        assert!(matches!(
            GpuResourceCache::<()>::new(0),
            Err(GpuCacheError::InvalidBudget { .. })
        ));
        assert!(GpuResourceCache::<()>::new(MAX_GPU_CACHE_BUDGET_BYTES + 1).is_err());

        let mut cache = GpuResourceCache::new(4).unwrap();
        let error = cache.insert(key("large"), 5, ()).unwrap_err();
        assert!(matches!(error, GpuCacheError::ResourceTooLarge { .. }));
        assert_eq!(cache.used(), 0);
        assert!(cache.is_empty());
    }
}
