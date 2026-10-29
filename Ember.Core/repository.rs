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
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FetchMode {
    #[default]
    CacheFirst,
    Refresh,
    Offline,
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
    pub source: DataSource,
    pub stale: bool,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ItemBatch {
    pub items: Vec<HnItem>,
    pub missing_ids: Vec<ItemId>,
    pub source: DataSource,
    pub stale: bool,
    pub warning: Option<String>,
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
        self.store
            .get(key)
            .await?
            .map(|json| {
                serde_json::from_str(&json)
                    .map_err(|e| ReaderError::InvalidData(format!("Invalid cached record: {e}")))
            })
            .transpose()
    }

    async fn load<T: Clone + Serialize + DeserializeOwned>(
        &self,
        key: &str,
        url: &str,
        now: u64,
        ttl: u64,
        mode: FetchMode,
    ) -> Result<Loaded<T>, ReaderError> {
        let cached = self.cached::<T>(key).await;
        let storage_warning = cached.as_ref().err().map(ToString::to_string);
        let cached = cached.ok().flatten();
        if let Some(record) = &cached {
            // Future-dated records can follow a wall-clock correction; refresh them.
            let fresh = record.fetched_at <= now && now - record.fetched_at < ttl;
            if mode == FetchMode::Offline || (mode == FetchMode::CacheFirst && fresh) {
                return Ok(Loaded {
                    value: record.value.clone(),
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
        }
        if mode == FetchMode::Offline {
            return Err(ReaderError::Storage(
                storage_warning.unwrap_or_else(|| "This content has not been downloaded".into()),
            ));
        }
        let loaded = self.request(url).await.and_then(|body| {
            serde_json::from_str::<T>(&body)
                .map_err(|e| ReaderError::InvalidData(format!("Invalid response: {e}")))
        });
        match loaded {
            Ok(value) => {
                let record = CacheRecord {
                    fetched_at: now,
                    value: value.clone(),
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
        if id == 0 {
            return Err(ReaderError::InvalidInput(
                "An item ID must be positive".into(),
            ));
        }
        let loaded = self
            .load::<Option<HnItem>>(
                &format!("cache:item:{id}"),
                &format!("{HN_API}/item/{id}.json"),
                now,
                self.policy.item_ttl,
                mode,
            )
            .await?;
        if loaded.value.as_ref().is_some_and(|item| item.id != id) {
            let _ = self.store.remove(&format!("cache:item:{id}")).await;
            return Err(ReaderError::InvalidData(
                "Response item ID does not match the requested item".into(),
            ));
        }
        Ok(loaded)
    }

    pub async fn load_items(&self, ids: &[ItemId], now: u64, mode: FetchMode) -> ItemBatch {
        let results = stream::iter(
            ids.iter()
                .copied()
                .map(|id| async move { (id, self.load_item(id, now, mode).await) }),
        )
        .buffered(self.policy.concurrency)
        .collect::<Vec<_>>()
        .await;
        let mut batch = ItemBatch::default();
        for (id, result) in results {
            match result {
                Ok(loaded) => {
                    combine_source(&mut batch.source, loaded.source);
                    batch.stale |= loaded.stale;
                    if batch.warning.is_none() {
                        batch.warning = loaded.warning;
                    }
                    match loaded.value {
                        Some(item) => batch.items.push(item),
                        None => batch.missing_ids.push(id),
                    }
                }
                Err(error) => {
                    batch.missing_ids.push(id);
                    if batch.warning.is_none() {
                        batch.warning = Some(error.to_string());
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
            source,
            stale: ids.stale || batch.stale,
            warning: batch.warning.or(ids.warning),
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
        let loaded = self.load_item(id, now, mode).await?;
        let root = loaded
            .value
            .ok_or_else(|| ReaderError::InvalidData("This story is unavailable".into()))?;
        let mut snapshot = ThreadSnapshot {
            root: root.clone(),
            source: loaded.source,
            stale: loaded.stale,
            warning: loaded.warning,
            ..ThreadSnapshot::default()
        };
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
        while !pending.is_empty() && requested < budget {
            let count = pending
                .len()
                .min(self.policy.concurrency)
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
        let mut snapshot = self
            .load_thread(id, budget, now, FetchMode::CacheFirst)
            .await?;
        for id in snapshot.items.keys().copied().collect::<Vec<_>>() {
            let cached = self
                .cached::<Option<HnItem>>(&format!("cache:item:{id}"))
                .await;
            if !matches!(cached, Ok(Some(CacheRecord { value: Some(ref item), .. })) if item.id == id)
            {
                if !snapshot.missing_ids.contains(&id) {
                    snapshot.missing_ids.push(id);
                }
                snapshot.warning = Some(
                    "Some content could not be saved. Free storage and retry the download.".into(),
                );
            }
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
    pub async fn save_state(&self, state: &ReaderState) -> Result<(), ReaderError> {
        self.store.put("reader:state", &state.export_json()?).await
    }
    pub async fn cached_items(&self, ids: &[ItemId]) -> Vec<HnItem> {
        let mut items = Vec::new();
        for id in ids {
            if let Ok(Some(record)) = self
                .cached::<Option<HnItem>>(&format!("cache:item:{id}"))
                .await
            {
                if let Some(item) = record.value {
                    items.push(item);
                }
            }
        }
        items
    }

    /// Evicts oldest unpinned items and excess search/profile records. Reading
    /// state, feed lists and saved/downloaded content survive. Returns removals.
    pub async fn prune_cache(&self, state: &ReaderState) -> Result<usize, ReaderError> {
        let records = self.store.scan_prefix("cache:item:").await?;
        let mut pinned: BTreeSet<_> = state.saved_ids().into_iter().collect();
        for (&root, ids) in &state.downloaded_threads {
            pinned.insert(root);
            pinned.extend(ids.iter().copied());
        }
        let mut candidates = Vec::new();
        for (key, json) in &records {
            let id = key.trim_start_matches("cache:item:").parse::<ItemId>().ok();
            if id.is_some_and(|id| pinned.contains(&id)) {
                continue;
            }
            let fetched_at = cache_record_time(json);
            candidates.push((fetched_at, key));
        }
        candidates.sort();
        let remove = records
            .len()
            .saturating_sub(state.preferences.cache_max_items);
        let mut removed = 0;
        for (_, key) in candidates.into_iter().take(remove) {
            self.store.remove(key).await?;
            removed += 1;
        }
        Ok(removed + self.prune_metadata_cache().await?)
    }

    async fn prune_metadata_cache(&self) -> Result<usize, ReaderError> {
        // Scan only these two small namespaces, not thousands of item payloads.
        // The six feed lists and versioned reader state cannot be evicted here.
        let mut records = self.store.scan_prefix("cache:user:").await?;
        records.extend(self.store.scan_prefix("cache:search:").await?);
        let remove = records.len().saturating_sub(METADATA_CACHE_LIMIT);
        if remove == 0 {
            return Ok(0);
        }
        let mut candidates: Vec<_> = records
            .iter()
            .map(|(key, json)| (cache_record_time(json), key))
            .collect();
        candidates.sort();
        for (_, key) in candidates.into_iter().take(remove) {
            self.store.remove(key).await?;
        }
        Ok(remove)
    }
    pub async fn cache_stats(&self) -> Result<CacheStats, ReaderError> {
        let entries = self.store.scan_prefix("cache:").await?;
        Ok(CacheStats {
            records: entries.len(),
            bytes: entries.iter().map(|(k, v)| k.len() + v.len()).sum(),
            items: entries
                .iter()
                .filter(|(k, _)| k.starts_with("cache:item:"))
                .count(),
        })
    }
    /// Deletes public cached payloads only. Preferences, bookmarks, history and
    /// collapse state remain intact; callers may explicitly clear state separately.
    pub async fn clear_cache(&self) -> Result<(), ReaderError> {
        for (key, _) in self.store.scan_prefix("cache:").await? {
            self.store.remove(&key).await?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct CacheStats {
    pub records: usize,
    pub items: usize,
    pub bytes: usize,
}

fn cache_record_time(json: &str) -> u64 {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| value.get("fetched_at").and_then(|value| value.as_u64()))
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
