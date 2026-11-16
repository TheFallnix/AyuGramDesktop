use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use ayu_core::{AccountId, CoreError, MediaCacheBackend, MediaCacheKind};

use crate::error::StorageError;

struct CacheEntry {
    payload: Box<[u8]>,
    last_access_seq: u64,
    account_id: AccountId,
    kind: MediaCacheKind,
}

struct CacheState {
    entries: HashMap<u64, CacheEntry>,
    lru_order: BTreeMap<u64, u64>,
    current_bytes: usize,
    next_seq: u64,
}

/// Thread-safe LRU cache for decoded images and media blobs with a strict byte ceiling.
pub struct ByteBoundedMediaCache {
    max_bytes: usize,
    state: Mutex<CacheState>,
}

impl ByteBoundedMediaCache {
    #[must_use]
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            state: Mutex::new(CacheState {
                entries: HashMap::new(),
                lru_order: BTreeMap::new(),
                current_bytes: 0,
                next_seq: 1,
            }),
        }
    }

    #[must_use]
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    #[must_use]
    pub fn current_bytes(&self) -> usize {
        self.state
            .lock()
            .map(|guard| guard.current_bytes)
            .unwrap_or(0)
    }

    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.state
            .lock()
            .map(|guard| guard.entries.len())
            .unwrap_or(0)
    }

    #[must_use]
    pub fn account_bytes(&self, account_id: AccountId) -> usize {
        self.state
            .lock()
            .map(|guard| {
                guard
                    .entries
                    .values()
                    .filter(|e| e.account_id == account_id)
                    .map(|e| e.payload.len())
                    .sum()
            })
            .unwrap_or(0)
    }

    pub fn put_bytes(&self, key: u64, data: &[u8]) -> Result<(), StorageError> {
        self.put_internal(key, AccountId(0), MediaCacheKind::Thumbnail, data)
    }

    pub fn put_scoped(
        &self,
        account_id: AccountId,
        kind: MediaCacheKind,
        resource_id: u64,
        data: &[u8],
    ) -> Result<(), StorageError> {
        let key = scoped_cache_key(account_id, kind, resource_id);
        self.put_internal(key, account_id, kind, data)
    }

    fn put_internal(
        &self,
        key: u64,
        account_id: AccountId,
        kind: MediaCacheKind,
        data: &[u8],
    ) -> Result<(), StorageError> {
        if data.len() > self.max_bytes {
            return Err(StorageError::PayloadExceedsCacheLimit {
                payload_bytes: data.len(),
                max_bytes: self.max_bytes,
            });
        }

        let mut guard = self.state.lock().map_err(|_| StorageError::LockPoisoned)?;

        if let Some(old) = guard.entries.remove(&key) {
            guard.current_bytes = guard.current_bytes.saturating_sub(old.payload.len());
            guard.lru_order.remove(&old.last_access_seq);
        }

        while guard.current_bytes.saturating_add(data.len()) > self.max_bytes {
            let Some((_, oldest_key)) = guard.lru_order.pop_first() else {
                break;
            };
            if let Some(evicted) = guard.entries.remove(&oldest_key) {
                guard.current_bytes = guard.current_bytes.saturating_sub(evicted.payload.len());
            }
        }

        let seq = guard.next_seq;
        guard.next_seq = guard.next_seq.saturating_add(1);
        let boxed: Box<[u8]> = data.into();
        guard.current_bytes = guard.current_bytes.saturating_add(boxed.len());
        guard.lru_order.insert(seq, key);
        guard.entries.insert(
            key,
            CacheEntry {
                payload: boxed,
                last_access_seq: seq,
                account_id,
                kind,
            },
        );
        Ok(())
    }

    #[must_use]
    pub fn get_bytes(&self, key: u64) -> Option<Vec<u8>> {
        let mut guard = self.state.lock().ok()?;
        let old_seq = guard.entries.get(&key)?.last_access_seq;
        let new_seq = guard.next_seq;
        guard.next_seq = guard.next_seq.saturating_add(1);

        guard.lru_order.remove(&old_seq);
        guard.lru_order.insert(new_seq, key);

        let entry = guard.entries.get_mut(&key)?;
        entry.last_access_seq = new_seq;
        Some(entry.payload.to_vec())
    }

    #[must_use]
    pub fn get_scoped(
        &self,
        account_id: AccountId,
        kind: MediaCacheKind,
        resource_id: u64,
    ) -> Option<Vec<u8>> {
        self.get_bytes(scoped_cache_key(account_id, kind, resource_id))
    }

    /// Evicts cached blobs belonging to `account_id` (optionally only heavy decoded media).
    pub fn evict_account(&self, account_id: AccountId, heavy_only: bool) -> usize {
        let Ok(mut guard) = self.state.lock() else {
            return 0;
        };
        let keys_to_remove: Vec<u64> = guard
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.account_id == account_id && (!heavy_only || entry.kind.is_heavy())
            })
            .map(|(&k, _)| k)
            .collect();

        let mut freed_bytes: usize = 0;
        for key in keys_to_remove {
            if let Some(removed) = guard.entries.remove(&key) {
                guard.lru_order.remove(&removed.last_access_seq);
                let len = removed.payload.len();
                guard.current_bytes = guard.current_bytes.saturating_sub(len);
                freed_bytes = freed_bytes.saturating_add(len);
            }
        }
        freed_bytes
    }

    /// Evicts least-recently-used entries until cached RAM is at or below `target_bytes`.
    pub fn shrink_for_background_account(&self, target_bytes: usize) -> usize {
        let Ok(mut guard) = self.state.lock() else {
            return 0;
        };
        let mut evicted_bytes: usize = 0;
        while guard.current_bytes > target_bytes {
            let Some((_, oldest_key)) = guard.lru_order.pop_first() else {
                break;
            };
            if let Some(evicted) = guard.entries.remove(&oldest_key) {
                let len = evicted.payload.len();
                guard.current_bytes = guard.current_bytes.saturating_sub(len);
                evicted_bytes = evicted_bytes.saturating_add(len);
            }
        }
        evicted_bytes
    }

    /// Drops all decoded media caches from RAM.
    pub fn evict_all(&self) -> usize {
        self.shrink_for_background_account(0)
    }
}

#[must_use]
const fn scoped_cache_key(account_id: AccountId, kind: MediaCacheKind, resource_id: u64) -> u64 {
    let mut x = account_id.0.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ ((kind as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9))
        ^ resource_id;
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

impl MediaCacheBackend for ByteBoundedMediaCache {
    fn put(&self, key: u64, data: &[u8]) -> Result<(), CoreError> {
        self.put_bytes(key, data).map_err(CoreError::from)
    }

    fn get(&self, key: u64) -> Option<Vec<u8>> {
        self.get_bytes(key)
    }

    fn current_bytes(&self) -> usize {
        self.current_bytes()
    }

    fn max_bytes(&self) -> usize {
        self.max_bytes()
    }

    fn shrink_to(&self, target_bytes: usize) -> usize {
        self.shrink_for_background_account(target_bytes)
    }
}
