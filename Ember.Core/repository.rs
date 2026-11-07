use crate::*;
use futures_util::{
    FutureExt, StreamExt,
    future::{WeakShared, poll_fn},
    stream,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fmt,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Poll, Waker},
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Combined search-page/profile cache budget. Feed lists and local reading state
/// are separate, and saved/downloaded item pinning does not use this budget.
pub const METADATA_CACHE_LIMIT: usize = 256;

// Bound transient payloads, storage requests per transaction and cancellation latency.
const STORE_BATCH: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReaderError {
    Network(String),
    Storage(String),
    InvalidData(String),
    InvalidInput(String),
}
impl fmt::Display for ReaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(s) => write!(f, "Network: {s}"),
            Self::Storage(s) => write!(f, "Storage: {s}"),
            Self::InvalidData(s) => write!(f, "Data: {s}"),
            Self::InvalidInput(s) => write!(f, "{s}"),
        }
    }
}
impl std::error::Error for ReaderError {}

/// Futures are deliberately not `Send`: a browser adapter can use JS promises.
pub trait HttpClient {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, ReaderError>>;
}

/// Durable, asynchronous string records. Implementations must finish a write
/// transaction before resolving `put`; a queued write is not a saved bookmark.
pub trait Store {
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>, ReaderError>>;
    fn put<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFuture<'a, Result<(), ReaderError>>;
    fn remove<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ReaderError>>;
    fn scan_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, String)>, ReaderError>>;

    /// Key and stored UTF-8 value length, without copying payloads into the caller.
    fn scan_sizes<'a>(
        &'a self,
        prefix: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, usize)>, ReaderError>> {
        Box::pin(async move {
            Ok(self
                .scan_prefix(prefix)
                .await?
                .into_iter()
                .map(|(key, value)| (key, value.len()))
                .collect())
        })
    }

    /// Return one slot per input key, including missing records and duplicates.
    fn get_many<'a>(
        &'a self,
        keys: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Option<String>>, ReaderError>> {
        Box::pin(async move {
            let mut values = Vec::with_capacity(keys.len());
            for key in keys {
                values.push(self.get(key).await?);
            }
            Ok(values)
        })
    }

    fn remove_many<'a>(&'a self, keys: &'a [String]) -> BoxFuture<'a, Result<(), ReaderError>> {
        Box::pin(async move {
            for key in keys {
                self.remove(key).await?;
            }
            Ok(())
        })
    }

    /// Clear only records in the requested namespace.
    fn remove_prefix<'a>(&'a self, prefix: &'a str) -> BoxFuture<'a, Result<(), ReaderError>> {
        Box::pin(async move {
            for (key, _) in self.scan_prefix(prefix).await? {
                self.remove(&key).await?;
            }
            Ok(())
        })
    }

    fn count_prefix<'a>(&'a self, prefix: &'a str) -> BoxFuture<'a, Result<usize, ReaderError>> {
        Box::pin(async move { Ok(self.scan_prefix(prefix).await?.len()) })
    }
}

#[derive(Default)]
pub struct MemoryStore {
    entries: RefCell<BTreeMap<String, String>>,
}
impl Store for MemoryStore {
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>, ReaderError>> {
        Box::pin(async move { Ok(self.entries.borrow().get(key).cloned()) })
    }
    fn put<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFuture<'a, Result<(), ReaderError>> {
        Box::pin(async move {
            self.entries.borrow_mut().insert(key.into(), value.into());
            Ok(())
        })
    }
    fn remove<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ReaderError>> {
        Box::pin(async move {
            self.entries.borrow_mut().remove(key);
            Ok(())
        })
    }
    fn scan_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, String)>, ReaderError>> {
        Box::pin(async move {
            Ok(self
                .entries
                .borrow()
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect())
        })
    }
    fn scan_sizes<'a>(
        &'a self,
        prefix: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, usize)>, ReaderError>> {
        Box::pin(async move {
            Ok(self
                .entries
                .borrow()
                .range(prefix.to_owned()..)
                .take_while(|(k, _)| k.starts_with(prefix))
                .map(|(k, v)| (k.clone(), v.len()))
                .collect())
        })
    }

    fn count_prefix<'a>(&'a self, prefix: &'a str) -> BoxFuture<'a, Result<usize, ReaderError>> {
        Box::pin(async move {
            Ok(self
                .entries
                .borrow()
                .range(prefix.to_owned()..)
                .take_while(|(key, _)| key.starts_with(prefix))
                .count())
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FetchMode {
    #[default]
    CacheFirst,
    Refresh,
    Offline,
}

/// Counts include the root story, comments and poll options.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadProgress {
    Collecting { loaded: usize, pending: usize },
    Verifying { checked: usize, total: usize },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum DataSource {
    Network,
    #[default]
    Cache,
    StaleCache,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Loaded<T> {
    pub value: T,
    pub source: DataSource,
    pub stale: bool,
    pub fetched_at: u64,
    pub warning: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CachePolicy {
    pub feed_ttl: u64,
    pub item_ttl: u64,
    pub user_ttl: u64,
    pub search_ttl: u64,
    pub concurrency: usize,
    pub max_response_bytes: usize,
}
impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            feed_ttl: 60,
            item_ttl: 120,
            user_ttl: 900,
            search_ttl: 300,
            concurrency: 8,
            max_response_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FeedPage {
    pub kind: FeedKind,
    pub items: Vec<HnItem>,
    pub total: usize,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub missing_ids: Vec<ItemId>,
    /// Transient failures and the number of successful rows preceding each one.
    /// An explicit null response is unavailable, but does not require another request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_slots: Vec<(usize, ItemId)>,
    /// The ranked ID list can fall back to cache while individual stories load online.
    #[serde(default)]
    pub ranking_source: DataSource,
    pub source: DataSource,
    pub stale: bool,
    pub warning: Option<String>,
    /// True when the general warning describes a row already reported in failed_slots.
    #[serde(default)]
    pub warning_from_failed_item: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ItemBatch {
    pub items: Vec<HnItem>,
    pub missing_ids: Vec<ItemId>,
    /// Transient failures and the number of successful rows preceding each one.
    /// An explicit null response is unavailable, but does not require another request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_slots: Vec<(usize, ItemId)>,
    pub source: DataSource,
    pub stale: bool,
    pub warning: Option<String>,
    /// True when the general warning describes a row already reported in failed_slots.
    #[serde(default)]
    pub warning_from_failed_item: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchPage {
    pub items: Vec<HnItem>,
    /// Algolia comment hits identify the containing story separately from parent.
    pub story_ids: BTreeMap<ItemId, ItemId>,
    pub page: usize,
    pub pages: usize,
    pub total: usize,
    pub source: DataSource,
    pub stale: bool,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheRecord<T> {
    fetched_at: u64,
    value: T,
}
type InFlight = WeakShared<BoxFuture<'static, Result<String, ReaderError>>>;

struct RequestLimiter {
    active: Cell<usize>,
    limit: usize,
    waiters: RefCell<Vec<Waker>>,
}
struct RequestPermit(Rc<RequestLimiter>);
impl Drop for RequestPermit {
    fn drop(&mut self) {
        self.0.active.set(self.0.active.get().saturating_sub(1));
        let waiters = std::mem::take(&mut *self.0.waiters.borrow_mut());
        for waker in waiters {
            waker.wake();
        }
    }
}
impl RequestLimiter {
    fn new(limit: usize) -> Rc<Self> {
        Rc::new(Self {
            active: Cell::new(0),
            limit,
            waiters: RefCell::new(Vec::new()),
        })
    }
    async fn acquire(self: &Rc<Self>) -> RequestPermit {
        poll_fn(|cx| {
            if self.active.get() < self.limit {
                self.active.set(self.active.get() + 1);
                Poll::Ready(RequestPermit(self.clone()))
            } else {
                let mut waiters = self.waiters.borrow_mut();
                if !waiters.iter().any(|w| w.will_wake(cx.waker())) {
                    waiters.push(cx.waker().clone());
                }
                Poll::Pending
            }
        })
        .await
    }
}

pub struct Repository {
    http: Rc<dyn HttpClient>,
    store: Rc<dyn Store>,
    policy: CachePolicy,
    in_flight: RefCell<HashMap<String, InFlight>>,
    limiter: Rc<RequestLimiter>,
}

impl Repository {
    pub fn new(http: Rc<dyn HttpClient>, store: Rc<dyn Store>) -> Self {
        Self {
            http,
            store,
            policy: CachePolicy::default(),
            in_flight: RefCell::new(HashMap::new()),
            limiter: RequestLimiter::new(CachePolicy::default().concurrency),
        }
    }
    pub fn with_policy(mut self, mut policy: CachePolicy) -> Self {
        policy.concurrency = policy.concurrency.clamp(1, 32);
        policy.max_response_bytes = policy.max_response_bytes.clamp(1024, 32 * 1024 * 1024);
        self.policy = policy;
        self.limiter = RequestLimiter::new(self.policy.concurrency);
        self
    }
    pub fn store(&self) -> &Rc<dyn Store> {
        &self.store
    }

    async fn request(&self, url: &str) -> Result<String, ReaderError> {
        let pending = {
            let mut in_flight = self.in_flight.borrow_mut();
            if let Some(pending) = in_flight.get(url).and_then(WeakShared::upgrade) {
                pending
            } else {
                let http = self.http.clone();
                let url = url.to_owned();
                let max = self.policy.max_response_bytes;
                let limiter = self.limiter.clone();
                let request_url = url.clone();
                let pending = async move {
                    let _permit = limiter.acquire().await;
                    let body = http.get(&request_url).await?;
                    if body.len() > max {
                        return Err(ReaderError::InvalidData(
                            "Response exceeds the reader's size limit".into(),
                        ));
                    }
                    Ok(body)
                }
                .boxed_local()
                .shared();
                // Weak ownership lets canceled requests release their future
                // and permit. The registry must not keep them alive forever.
                in_flight.retain(|_, value| value.upgrade().is_some());
                in_flight.insert(url, pending.downgrade().expect("new shared future"));
                pending
            }
        };
        let result = pending.await;
        self.in_flight.borrow_mut().remove(url);
        result
    }

    async fn cached<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<CacheRecord<T>>, ReaderError> {
        decode_record(self.store.get(key).await)
    }

    // Retry an exceptional batch failure individually: one corrupt adapter record
    // must not make its healthy neighbors unavailable offline.
    async fn read_item_records(
        &self,
        keys: &[String],
    ) -> Vec<Result<Option<CacheRecord<Option<HnItem>>>, ReaderError>> {
        if let Ok(values) = self.store.get_many(keys).await {
            if values.len() == keys.len() {
                return values
                    .into_iter()
                    .map(|value| decode_record(Ok(value)))
                    .collect();
            }
        }
        let mut records = Vec::with_capacity(keys.len());
        for key in keys {
            records.push(self.cached(key).await);
        }
        records
    }

    async fn load<T: Serialize + DeserializeOwned>(
        &self,
        key: &str,
        url: &str,
        now: u64,
        ttl: u64,
        mode: FetchMode,
    ) -> Result<Loaded<T>, ReaderError> {
        let cached = self.cached(key).await;
        self.resolve(key, || url.to_owned(), now, ttl, mode, cached)
            .await
    }

    async fn resolve<T: Serialize + DeserializeOwned>(
        &self,
        key: &str,
        url: impl FnOnce() -> String,
        now: u64,
        ttl: u64,
        mode: FetchMode,
        cached: Result<Option<CacheRecord<T>>, ReaderError>,
    ) -> Result<Loaded<T>, ReaderError> {
        let storage_warning = cached.as_ref().err().map(ToString::to_string);
        let cached = if let Some(record) = cached.ok().flatten() {
            // Future-dated records can follow a wall-clock correction; refresh them.
            let fresh = record.fetched_at <= now && now - record.fetched_at < ttl;
            if mode == FetchMode::Offline || (mode == FetchMode::CacheFirst && fresh) {
                return Ok(Loaded {
                    value: record.value,
                    source: if fresh {
                        DataSource::Cache
                    } else {
                        DataSource::StaleCache
                    },
                    stale: !fresh,
                    fetched_at: record.fetched_at,
                    warning: storage_warning,
                });
            }
            Some(record)
        } else {
            None
        };
        if mode == FetchMode::Offline {
            return Err(ReaderError::Storage(
                storage_warning.unwrap_or_else(|| "This content has not been downloaded".into()),
            ));
        }
        let loaded = self.request(&url()).await.and_then(|body| {
            serde_json::from_str::<T>(&body)
                .map_err(|e| ReaderError::InvalidData(format!("Invalid response: {e}")))
        });
        match loaded {
            Ok(value) => {
                let record = CacheRecord {
                    fetched_at: now,
                    value: &value,
                };
                let json = serde_json::to_string(&record)
                    .map_err(|e| ReaderError::InvalidData(e.to_string()))?;
                let cached = self.store.put(key, &json).await;
                let cleanup = if cached.is_ok()
                    && (key.starts_with("cache:user:") || key.starts_with("cache:search:"))
                {
                    self.prune_metadata_cache().await.err()
                } else {
                    None
                };
                let warning = cached
                    .err()
                    .map(|e| format!("Loaded online; could not cache: {e}"))
                    .or_else(|| {
                        cleanup.map(|e| format!("Loaded online; cache cleanup failed: {e}"))
                    })
                    .or(storage_warning);
                Ok(Loaded {
                    value,
                    source: DataSource::Network,
                    stale: false,
                    fetched_at: now,
                    warning,
                })
            }
            Err(error) => match cached {
                Some(record) => Ok(Loaded {
                    value: record.value,
                    source: DataSource::StaleCache,
                    stale: true,
                    fetched_at: record.fetched_at,
                    warning: Some(format!("Showing cached content. {error}")),
                }),
                None => Err(error),
            },
        }
    }

    pub async fn load_item(
        &self,
        id: ItemId,
        now: u64,
        mode: FetchMode,
    ) -> Result<Loaded<Option<HnItem>>, ReaderError> {
        let key = format!("cache:item:{id}");
        if id == 0 {
            return Err(ReaderError::InvalidInput(
                "An item ID must be positive".into(),
            ));
        }
        let cached = self.cached(&key).await;
        self.resolve_item(id, &key, now, mode, cached).await
    }

    async fn resolve_item(
        &self,
        id: ItemId,
        key: &str,
        now: u64,
        mode: FetchMode,
        cached: Result<Option<CacheRecord<Option<HnItem>>>, ReaderError>,
    ) -> Result<Loaded<Option<HnItem>>, ReaderError> {
        if id == 0 {
            return Err(ReaderError::InvalidInput(
                "An item ID must be positive".into(),
            ));
        }
        let loaded = self
            .resolve(
                key,
                || format!("{HN_API}/item/{id}.json"),
                now,
                self.policy.item_ttl,
                mode,
                cached,
            )
            .await?;
        if loaded.value.as_ref().is_some_and(|item| item.id != id) {
            let _ = self.store.remove(key).await;
            return Err(ReaderError::InvalidData(
                "Response item ID does not match the requested item".into(),
            ));
        }
        Ok(loaded)
    }

    pub async fn load_items(&self, ids: &[ItemId], now: u64, mode: FetchMode) -> ItemBatch {
        let mut batch = ItemBatch::default();
        for ids in ids.chunks(STORE_BATCH) {
            let keys: Vec<_> = ids.iter().map(|id| format!("cache:item:{id}")).collect();
            let cached = self.read_item_records(&keys).await;
            let mut results = stream::iter(ids.iter().zip(&keys).zip(cached).enumerate().map(
                |(index, ((&id, key), record))| async move {
                    // A preceding occurrence may have populated the cache after
                    // this page was read. Let it share that request or fresh value.
                    let result = if mode != FetchMode::Offline && ids[..index].contains(&id) {
                        self.load_item(id, now, mode).await
                    } else {
                        self.resolve_item(id, key, now, mode, record).await
                    };
                    (id, result)
                },
            ))
            .buffered(self.policy.concurrency);
            while let Some((id, result)) = results.next().await {
                match result {
                    Ok(loaded) => {
                        combine_source(&mut batch.source, loaded.source);
                        batch.stale |= loaded.stale;
                        if loaded.warning.is_some() && (batch.warning.is_none() || batch.warning_from_failed_item) {
                            // A storage or fallback warning on a readable row needs its own feedback.
                            batch.warning = loaded.warning;
                            batch.warning_from_failed_item = false;
                        }
                        match loaded.value {
                            Some(item) => batch.items.push(item),
                            None => batch.missing_ids.push(id),
                        }
                    }
                    Err(error) => {
                        batch.failed_slots.push((batch.items.len(), id));
                        batch.missing_ids.push(id);
                        if batch.warning.is_none() {
                            batch.warning = Some(error.to_string());
                            batch.warning_from_failed_item = true;
                        }
                    }
                }
            }
        }
        batch
    }

    pub async fn load_feed(
        &self,
        kind: FeedKind,
        offset: usize,
        limit: usize,
        now: u64,
        mode: FetchMode,
    ) -> Result<FeedPage, ReaderError> {
        let ids = self
            .load::<Vec<ItemId>>(
                &format!("cache:feed:{}", kind.endpoint()),
                &format!("{HN_API}/{}.json", kind.endpoint()),
                now,
                self.policy.feed_ttl,
                mode,
            )
            .await?;
        let end = offset
            .saturating_add(limit.clamp(1, 100))
            .min(ids.value.len());
        let slice = ids.value.get(offset..end).unwrap_or_default();
        let batch = self.load_items(slice, now, mode).await;
        let mut source = ids.source;
        combine_source(&mut source, batch.source);
        Ok(FeedPage {
            kind,
            items: batch.items,
            total: ids.value.len(),
            offset,
            next_offset: (end < ids.value.len()).then_some(end),
            missing_ids: batch.missing_ids,
            failed_slots: batch.failed_slots,
            ranking_source: ids.source,
            source,
            stale: ids.stale || batch.stale,
            warning_from_failed_item: ids.warning.is_none() && batch.warning_from_failed_item,
            warning: ids.warning.or(batch.warning),
        })
    }

    pub async fn load_user(
        &self,
        username: &str,
        now: u64,
        mode: FetchMode,
    ) -> Result<Loaded<Option<HnUser>>, ReaderError> {
        validate_username(username)?;
        let loaded = self
            .load::<Option<HnUser>>(
                &format!("cache:user:{username}"),
                &format!("{HN_API}/user/{username}.json"),
                now,
                self.policy.user_ttl,
                mode,
            )
            .await?;
        if loaded
            .value
            .as_ref()
            .is_some_and(|user| user.id != username)
        {
            let _ = self.store.remove(&format!("cache:user:{username}")).await;
            return Err(ReaderError::InvalidData(
                "Response username does not match the requested profile".into(),
            ));
        }
        Ok(loaded)
    }

    pub async fn load_thread(
        &self,
        id: ItemId,
        budget: usize,
        now: u64,
        mode: FetchMode,
    ) -> Result<ThreadSnapshot, ReaderError> {
        self.load_thread_reporting(id, budget, now, mode, |_, _| {})
            .await
    }

    async fn load_thread_reporting(
        &self,
        id: ItemId,
        budget: usize,
        now: u64,
        mode: FetchMode,
        report: impl Fn(usize, usize),
    ) -> Result<ThreadSnapshot, ReaderError> {
        let loaded = self.load_item(id, now, mode).await?;
        let root = loaded
            .value
            .ok_or_else(|| ReaderError::InvalidData("This story is unavailable".into()))?;
        let mut snapshot = ThreadSnapshot {
            root,
            source: loaded.source,
            stale: loaded.stale,
            warning: loaded.warning,
            ..ThreadSnapshot::default()
        };
        let root = &snapshot.root;
        snapshot.items.insert(id, root.clone());
        let budget = budget.clamp(1, 20_000);
        let mut seen = BTreeSet::from([id]);
        let mut pending: VecDeque<_> = root
            .kids
            .iter()
            .chain(root.parts.iter())
            .copied()
            .filter(|id| seen.insert(*id))
            .collect();
        let mut requested = 1;
        report(snapshot.items.len(), pending.len());
        while !pending.is_empty() && requested < budget {
            let count = pending
                .len()
                .min(if mode == FetchMode::Offline {
                    STORE_BATCH
                } else {
                    self.policy.concurrency
                })
                .min(budget - requested);
            let ids: Vec<_> = pending.drain(..count).collect();
            requested += ids.len();
            let batch = self.load_items(&ids, now, mode).await;
            combine_source(&mut snapshot.source, batch.source);
            snapshot.stale |= batch.stale;
            if snapshot.warning.is_none() {
                snapshot.warning = batch.warning;
            }
            snapshot.missing_ids.extend(batch.missing_ids);
            for item in batch.items {
                for child in item.kids.iter().chain(item.parts.iter()).copied() {
                    if seen.insert(child) {
                        pending.push_back(child);
                    }
                }
                snapshot.items.insert(item.id, item);
            }
            report(snapshot.items.len(), pending.len());
        }
        snapshot.pending_ids = pending.into();
        snapshot.complete = snapshot.missing_ids.is_empty() && snapshot.pending_ids.is_empty();
        Ok(snapshot)
    }

    /// Completeness additionally verifies durable payloads, so quota/IO failures
    /// cannot produce a misleading successful offline-download indicator.
    pub async fn download_thread(
        &self,
        id: ItemId,
        budget: usize,
        now: u64,
    ) -> Result<ThreadSnapshot, ReaderError> {
        self.download_thread_with_progress(id, budget, now, FetchMode::CacheFirst, |_| {})
            .await
    }

    /// Use Offline to check an existing copy without contacting the provider.
    /// Dropping this future stops scheduling requests and verification reads.
    pub async fn download_thread_with_progress(
        &self,
        id: ItemId,
        budget: usize,
        now: u64,
        mode: FetchMode,
        report: impl Fn(DownloadProgress),
    ) -> Result<ThreadSnapshot, ReaderError> {
        let mut snapshot = self
            .load_thread_reporting(id, budget, now, mode, |loaded, pending| {
                report(DownloadProgress::Collecting { loaded, pending });
            })
            .await?;
        let total = snapshot.items.len();
        report(DownloadProgress::Verifying { checked: 0, total });
        let mut items = snapshot.items.iter();
        let mut checked = 0;
        while checked < total {
            let group: Vec<_> = items.by_ref().take(STORE_BATCH).collect();
            let keys: Vec<_> = group
                .iter()
                .map(|(id, _)| format!("cache:item:{id}"))
                .collect();
            let records = self.read_item_records(&keys).await;
            for ((&id, expected), record) in group.into_iter().zip(records) {
                if !matches!(record, Ok(Some(CacheRecord { value: Some(ref item), .. })) if item == expected)
                {
                    // Traversal visits each ID once. A successfully loaded item
                    // cannot also be in the traversal's missing-ID collection.
                    snapshot.missing_ids.push(id);
                    snapshot.warning = Some(
                        "Some content could not be saved. Free storage and retry the download."
                            .into(),
                    );
                }
            }
            checked += keys.len();
            report(DownloadProgress::Verifying { checked, total });
        }
        snapshot.complete = snapshot.missing_ids.is_empty() && snapshot.pending_ids.is_empty();
        Ok(snapshot)
    }

    pub async fn search(
        &self,
        query: &SearchQuery,
        now: u64,
        mode: FetchMode,
    ) -> Result<SearchPage, ReaderError> {
        let url = query.url()?;
        let loaded = self
            .load::<AlgoliaPage>(
                &format!("cache:search:{url}"),
                &url,
                now,
                self.policy.search_ttl,
                mode,
            )
            .await?;
        let mut result = SearchPage {
            page: loaded.value.page,
            pages: loaded.value.nb_pages,
            total: loaded.value.nb_hits,
            source: loaded.source,
            stale: loaded.stale,
            warning: loaded.warning,
            ..SearchPage::default()
        };
        for hit in loaded.value.hits {
            let Some(id) = hit.object_id.parse::<ItemId>().ok().filter(|id| *id > 0) else {
                continue;
            };
            if let Some(story_id) = hit.story_id {
                result.story_ids.insert(id, story_id);
            }
            let kind = if hit.tags.iter().any(|t| t == "comment") || hit.comment_text.is_some() {
                ItemKind::Comment
            } else if hit.tags.iter().any(|t| t == "poll") {
                ItemKind::Poll
            } else {
                ItemKind::Story
            };
            result.items.push(HnItem {
                id,
                kind: Some(kind),
                by: hit.author,
                time: hit.created_at_i,
                title: hit.title.or(hit.story_title),
                text: hit.comment_text.or(hit.story_text),
                url: hit.url.or(hit.story_url),
                score: hit.points,
                descendants: hit.num_comments,
                parent: hit.parent_id,
                ..HnItem::default()
            });
        }
        Ok(result)
    }

    pub async fn load_state(&self) -> Result<ReaderState, ReaderError> {
        match self.store.get("reader:state").await? {
            Some(json) => ReaderState::import_json(&json),
            None => Ok(ReaderState::default()),
        }
    }
    /// Encode a compact snapshot before returning the write future. The caller can
    /// release its UI-state borrow while the storage transaction is in flight.
    pub fn save_state<'a>(
        &'a self,
        state: &ReaderState,
    ) -> impl Future<Output = Result<(), ReaderError>> + use<'a> {
        let json = serde_json::to_string(state)
            .map_err(|error| ReaderError::InvalidData(error.to_string()));
        async move { self.store.put("reader:state", &json?).await }
    }
    pub async fn cached_items(&self, ids: &[ItemId]) -> Vec<HnItem> {
        let mut items = Vec::new();
        for group in ids.chunks(STORE_BATCH) {
            let keys: Vec<_> = group.iter().map(|id| format!("cache:item:{id}")).collect();
            for record in self
                .read_item_records(&keys)
                .await
                .into_iter()
                .flatten()
                .flatten()
            {
                items.extend(record.value);
            }
        }
        items
    }

    /// Evicts oldest unpinned items and excess search/profile records. Reading
    /// state, feed lists and saved/downloaded content survive. Returns removals.
    pub fn prune_cache<'a>(
        &'a self,
        state: &ReaderState,
    ) -> BoxFuture<'a, Result<usize, ReaderError>> {
        let limit = state.preferences.cache_max_items;
        let pinned = state.pinned_items();
        Box::pin(async move {
            if self.store.count_prefix("cache:item:").await? <= limit {
                return self.prune_metadata_cache().await;
            }
            let records = self.store.scan_prefix("cache:item:").await?;
            let mut candidates = Vec::new();
            let remove = records.len().saturating_sub(limit);
            for (key, json) in records {
                let id = key.trim_start_matches("cache:item:").parse::<ItemId>().ok();
                if id.is_some_and(|id| pinned.contains(&id)) {
                    continue;
                }
                let fetched_at = cache_record_time(&json);
                candidates.push((fetched_at, key));
            }
            let keys = oldest_keys(candidates, remove);
            self.remove_keys(&keys).await?;
            Ok(keys.len() + self.prune_metadata_cache().await?)
        })
    }

    async fn remove_keys(&self, keys: &[String]) -> Result<(), ReaderError> {
        for batch in keys.chunks(STORE_BATCH) {
            self.store.remove_many(batch).await?;
        }
        Ok(())
    }

    async fn prune_metadata_cache(&self) -> Result<usize, ReaderError> {
        let count = self.store.count_prefix("cache:user:").await?
            + self.store.count_prefix("cache:search:").await?;
        if count <= METADATA_CACHE_LIMIT {
            return Ok(0);
        }
        let mut records = self.store.scan_prefix("cache:user:").await?;
        records.extend(self.store.scan_prefix("cache:search:").await?);
        let remove = records.len().saturating_sub(METADATA_CACHE_LIMIT);
        let candidates = records
            .into_iter()
            .map(|(key, json)| (cache_record_time(&json), key))
            .collect();
        let keys = oldest_keys(candidates, remove);
        self.remove_keys(&keys).await?;
        Ok(keys.len())
    }
    pub async fn cache_stats(&self) -> Result<CacheStats, ReaderError> {
        let entries = self.store.scan_sizes("cache:").await?;
        Ok(CacheStats {
            records: entries.len(),
            bytes: entries.iter().map(|(k, v)| k.len() + v).sum(),
            items: entries
                .iter()
                .filter(|(k, _)| k.starts_with("cache:item:"))
                .count(),
        })
    }
    /// Release browsing cache while retaining feed lists, bookmarks and every verified
    /// download. Protect both the active library and any state still durable on disk.
    pub fn clean_temporary_cache<'a>(
        &'a self,
        state: &ReaderState,
    ) -> BoxFuture<'a, Result<CacheStats, ReaderError>> {
        let mut pinned = state.pinned_items();
        Box::pin(async move {
            pinned.extend(self.load_state().await?.pinned_items());
            let mut removed = CacheStats::default();
            let mut keys = Vec::new();
            for (key, value_bytes) in self.store.scan_sizes("cache:").await? {
                let item = key.strip_prefix("cache:item:");
                let temporary = match item {
                    Some(id) => !id.parse::<ItemId>().is_ok_and(|id| pinned.contains(&id)),
                    None => key.starts_with("cache:user:") || key.starts_with("cache:search:"),
                };
                if temporary {
                    removed.records += 1;
                    removed.items += usize::from(item.is_some());
                    removed.bytes += key.len() + value_bytes;
                    keys.push(key);
                }
            }
            self.remove_keys(&keys).await?;
            Ok(removed)
        })
    }

    /// Deletes public cached payloads only. Preferences, bookmarks, history and
    /// collapse state remain intact; callers may explicitly clear state separately.
    pub async fn clear_cache(&self) -> Result<(), ReaderError> {
        self.store.remove_prefix("cache:").await
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct CacheStats {
    pub records: usize,
    pub items: usize,
    pub bytes: usize,
}

/// Select only the records above the limit. The usual eviction removes one
/// page from a much larger cache. Order that subset for deterministic retries.
fn oldest_keys(mut records: Vec<(u64, String)>, count: usize) -> Vec<String> {
    if count < records.len() {
        records.select_nth_unstable(count);
        records.truncate(count);
    }
    records.sort_unstable();
    records.into_iter().map(|(_, key)| key).collect()
}

fn decode_record<T: DeserializeOwned>(
    raw: Result<Option<String>, ReaderError>,
) -> Result<Option<CacheRecord<T>>, ReaderError> {
    raw?.map(|json| {
        serde_json::from_str(&json)
            .map_err(|e| ReaderError::InvalidData(format!("Invalid cached record: {e}")))
    })
    .transpose()
}

fn cache_record_time(json: &str) -> u64 {
    #[derive(Deserialize)]
    struct Timestamp {
        fetched_at: u64,
    }
    serde_json::from_str::<Timestamp>(json)
        .map(|r| r.fetched_at)
        .unwrap_or(0)
}

fn combine_source(current: &mut DataSource, other: DataSource) {
    *current = if *current == DataSource::StaleCache || other == DataSource::StaleCache {
        DataSource::StaleCache
    } else if *current == DataSource::Network || other == DataSource::Network {
        DataSource::Network
    } else {
        DataSource::Cache
    };
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct AlgoliaPage {
    hits: Vec<AlgoliaHit>,
    page: usize,
    #[serde(rename = "nbPages")]
    nb_pages: usize,
    #[serde(rename = "nbHits")]
    nb_hits: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct AlgoliaHit {
    #[serde(rename = "objectID")]
    object_id: String,
    author: Option<String>,
    title: Option<String>,
    story_title: Option<String>,
    url: Option<String>,
    story_url: Option<String>,
    story_text: Option<String>,
    comment_text: Option<String>,
    created_at_i: Option<u64>,
    points: Option<i64>,
    num_comments: Option<u64>,
    parent_id: Option<ItemId>,
    story_id: Option<ItemId>,
    #[serde(rename = "_tags")]
    tags: Vec<String>,
}
