use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

pub type ItemId = u64;
pub const HN_API: &str = "https://hacker-news.firebaseio.com/v0";
pub const SEARCH_API: &str = "https://hn.algolia.com/api/v1";
pub const STATE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedKind {
    #[default]
    Top,
    New,
    Best,
    Ask,
    Show,
    Jobs,
}

impl FeedKind {
    pub const ALL: [Self; 6] = [
        Self::Top,
        Self::New,
        Self::Best,
        Self::Ask,
        Self::Show,
        Self::Jobs,
    ];
    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Top => "topstories",
            Self::New => "newstories",
            Self::Best => "beststories",
            Self::Ask => "askstories",
            Self::Show => "showstories",
            Self::Jobs => "jobstories",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Top => "Front page",
            Self::New => "Latest",
            Self::Best => "Best",
            Self::Ask => "Ask HN",
            Self::Show => "Show HN",
            Self::Jobs => "Jobs",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Story,
    Comment,
    Job,
    Poll,
    Pollopt,
    #[serde(other)]
    Other,
}

/// Optional API fields deliberately remain optional, including score and author.
/// Deleted items, jobs and poll options do not have the same shape as stories.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HnItem {
    pub id: ItemId,
    #[serde(rename = "type")]
    pub kind: Option<ItemKind>,
    pub by: Option<String>,
    pub time: Option<u64>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub text: Option<String>,
    pub score: Option<i64>,
    pub descendants: Option<u64>,
    pub parent: Option<ItemId>,
    pub poll: Option<ItemId>,
    pub kids: Vec<ItemId>,
    pub parts: Vec<ItemId>,
    pub deleted: bool,
    pub dead: bool,
}

impl HnItem {
    pub fn title_text(&self) -> &str {
        self.title.as_deref().unwrap_or(if self.deleted {
            "[deleted]"
        } else {
            "Discussion"
        })
    }
    pub fn author(&self) -> &str {
        self.by.as_deref().unwrap_or("[deleted]")
    }
    pub fn discussion_url(&self) -> String {
        item_url(self.id)
    }
    pub fn article_url(&self) -> Option<String> {
        self.url.as_deref().and_then(safe_web_url)
    }
    pub fn domain(&self) -> String {
        self.url
            .as_deref()
            .and_then(|s| Url::parse(s).ok())
            .and_then(|u| {
                u.host_str()
                    .map(|s| s.trim_start_matches("www.").to_owned())
            })
            .unwrap_or_else(|| "news.ycombinator.com".to_owned())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HnUser {
    pub id: String,
    pub created: Option<u64>,
    pub karma: Option<i64>,
    pub about: Option<String>,
    pub submitted: Vec<ItemId>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchSort {
    #[default]
    Relevance,
    Recent,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    #[default]
    Stories,
    Comments,
    All,
    Ask,
    Show,
    Polls,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchQuery {
    pub query: String,
    pub sort: SearchSort,
    pub kind: SearchKind,
    pub author: Option<String>,
    pub story_id: Option<ItemId>,
    pub since: Option<u64>,
    pub until: Option<u64>,
    pub min_points: Option<u64>,
    pub min_comments: Option<u64>,
    pub page: usize,
    pub page_size: usize,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            sort: SearchSort::Relevance,
            kind: SearchKind::Stories,
            author: None,
            story_id: None,
            since: None,
            until: None,
            min_points: None,
            min_comments: None,
            page: 0,
            page_size: 30,
        }
    }
}

impl SearchQuery {
    pub fn url(&self) -> Result<String, crate::ReaderError> {
        if self.query.len() > 2048 {
            return Err(crate::ReaderError::InvalidInput(
                "Search is too long".into(),
            ));
        }
        if self
            .since
            .zip(self.until)
            .is_some_and(|(start, end)| start > end)
        {
            return Err(crate::ReaderError::InvalidInput(
                "The start date must precede the end date".into(),
            ));
        }
        let endpoint = if self.sort == SearchSort::Recent {
            "search_by_date"
        } else {
            "search"
        };
        let mut url = Url::parse(&format!("{SEARCH_API}/{endpoint}")).expect("static API URL");
        let mut tags = Vec::<String>::new();
        match self.kind {
            SearchKind::Stories => tags.push("story".into()),
            SearchKind::Comments => tags.push("comment".into()),
            SearchKind::Ask => tags.push("ask_hn".into()),
            SearchKind::Show => tags.push("show_hn".into()),
            SearchKind::Polls => tags.push("poll".into()),
            SearchKind::All => {}
        }
        if let Some(author) = &self.author {
            validate_username(author)?;
            tags.push(format!("author_{author}"));
        }
        if let Some(id) = self.story_id {
            tags.push(format!("story_{id}"));
        }
        let mut numeric = Vec::<String>::new();
        if let Some(v) = self.since {
            numeric.push(format!("created_at_i>={v}"));
        }
        if let Some(v) = self.until {
            numeric.push(format!("created_at_i<={v}"));
        }
        if let Some(v) = self.min_points {
            numeric.push(format!("points>={v}"));
        }
        if let Some(v) = self.min_comments {
            numeric.push(format!("num_comments>={v}"));
        }
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("query", &self.query)
            .append_pair("tags", &tags.join(","))
            .append_pair("page", &self.page.to_string())
            .append_pair("hitsPerPage", &self.page_size.clamp(1, 100).to_string());
        if !numeric.is_empty() {
            pairs.append_pair("numericFilters", &numeric.join(","));
        }
        drop(pairs);
        Ok(url.into())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: ThemeMode,
    pub accent: String,
    pub font_size: f64,
    pub line_height: f64,
    pub custom_background: Option<String>,
    pub compact: bool,
    pub reduced_motion: bool,
    pub show_dead: bool,
    pub hide_read: bool,
    pub cache_max_items: usize,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            accent: "#FF6B35".into(),
            font_size: 16.0,
            line_height: 1.5,
            custom_background: None,
            compact: false,
            reduced_motion: false,
            show_dead: false,
            hide_read: false,
            cache_max_items: 5000,
        }
    }
}

impl Preferences {
    /// Normalize imported/user-editable settings before applying them to layout.
    pub fn normalize(&mut self) {
        self.font_size = if self.font_size.is_finite() {
            self.font_size.clamp(12.0, 32.0)
        } else {
            16.0
        };
        self.line_height = if self.line_height.is_finite() {
            self.line_height.clamp(1.1, 2.2)
        } else {
            1.5
        };
        self.cache_max_items = self.cache_max_items.clamp(100, 100_000);
        if self.accent.len() != 7
            || !self.accent.starts_with('#')
            || !self.accent[1..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            self.accent = "#FF6B35".into();
        }
        if self.custom_background.as_ref().is_some_and(|s| {
            s.len() != 7 || !s.starts_with('#') || !s[1..].bytes().all(|b| b.is_ascii_hexdigit())
        }) {
            self.custom_background = None;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemState {
    pub saved_at: Option<u64>,
    pub read_at: Option<u64>,
    pub hidden: bool,
    pub last_opened_at: Option<u64>,
    pub scroll_anchor: Option<ItemId>,
    pub collapsed: BTreeSet<ItemId>,
    pub seen_comments: BTreeSet<ItemId>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MuteRules {
    pub domains: BTreeSet<String>,
    pub users: BTreeSet<String>,
    pub keywords: BTreeSet<String>,
}

impl MuteRules {
    pub fn matches(&self, item: &HnItem) -> bool {
        let domain = item.domain().to_ascii_lowercase();
        let title = item.title_text().to_lowercase();
        self.users.contains(item.author())
            || self.domains.iter().any(|value| {
                let value = value.trim().trim_start_matches("www.").to_ascii_lowercase();
                !value.is_empty() && (domain == value || domain.ends_with(&format!(".{value}")))
            })
            || self.keywords.iter().any(|value| {
                !value.trim().is_empty() && title.contains(&value.trim().to_lowercase())
            })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReaderState {
    pub version: u32,
    pub preferences: Preferences,
    pub items: BTreeMap<ItemId, ItemState>,
    pub mutes: MuteRules,
    pub recent_searches: Vec<String>,
    /// Payloads belonging to completed downloads are pinned during cache eviction.
    pub downloaded_threads: BTreeMap<ItemId, Vec<ItemId>>,
}

impl Default for ReaderState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            preferences: Preferences::default(),
            items: BTreeMap::new(),
            mutes: MuteRules::default(),
            recent_searches: Vec::new(),
            downloaded_threads: BTreeMap::new(),
        }
    }
}

impl ReaderState {
    pub fn toggle_saved(&mut self, id: ItemId, now: u64) -> bool {
        let item = self.items.entry(id).or_default();
        item.saved_at = if item.saved_at.is_some() {
            None
        } else {
            Some(now)
        };
        item.saved_at.is_some()
    }
    pub fn mark_read(&mut self, id: ItemId, now: u64) {
        let item = self.items.entry(id).or_default();
        item.read_at = Some(now);
        item.last_opened_at = Some(now);
    }
    pub fn toggle_hidden(&mut self, id: ItemId) -> bool {
        let item = self.items.entry(id).or_default();
        item.hidden = !item.hidden;
        item.hidden
    }
    pub fn is_saved(&self, id: ItemId) -> bool {
        self.items.get(&id).is_some_and(|s| s.saved_at.is_some())
    }
    pub fn is_read(&self, id: ItemId) -> bool {
        self.items.get(&id).is_some_and(|s| s.read_at.is_some())
    }
    pub fn is_visible(&self, item: &HnItem) -> bool {
        !item.deleted
            && (!item.dead || self.preferences.show_dead)
            && !self.mutes.matches(item)
            && !self
                .items
                .get(&item.id)
                .is_some_and(|s| s.hidden || (self.preferences.hide_read && s.read_at.is_some()))
    }
    pub fn saved_ids(&self) -> Vec<ItemId> {
        let mut values: Vec<_> = self
            .items
            .iter()
            .filter_map(|(&id, s)| s.saved_at.map(|at| (at, id)))
            .collect();
        values.sort_by(|a, b| b.cmp(a));
        values.into_iter().map(|(_, id)| id).collect()
    }
    pub fn history_ids(&self) -> Vec<ItemId> {
        let mut values: Vec<_> = self
            .items
            .iter()
            .filter_map(|(&id, s)| s.last_opened_at.map(|at| (at, id)))
            .collect();
        values.sort_by(|a, b| b.cmp(a));
        values.into_iter().map(|(_, id)| id).collect()
    }
    pub fn remember_search(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.recent_searches.retain(|s| s != query);
        self.recent_searches
            .insert(0, query.chars().take(2048).collect());
        self.recent_searches.truncate(20);
    }
    pub fn export_json(&self) -> Result<String, crate::ReaderError> {
        serde_json::to_string_pretty(self)
            .map_err(|e| crate::ReaderError::InvalidData(e.to_string()))
    }
    pub fn import_json(json: &str) -> Result<Self, crate::ReaderError> {
        if json.len() > 16 * 1024 * 1024 {
            return Err(crate::ReaderError::InvalidInput(
                "Reading state exceeds 16 MiB".into(),
            ));
        }
        let mut state: Self = serde_json::from_str(json)
            .map_err(|e| crate::ReaderError::InvalidData(e.to_string()))?;
        if state.version > STATE_VERSION {
            return Err(crate::ReaderError::InvalidData(
                "Reading state was created by a newer Ember version".into(),
            ));
        }
        state.version = STATE_VERSION;
        state.preferences.normalize();
        state.recent_searches.truncate(20);
        Ok(state)
    }
}

/// Only absolute web URLs with a host and without embedded credentials can leave
/// the content renderer. Never pass raw HN HTML attributes to a shell/browser.
pub fn safe_web_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.chars().any(char::is_control) {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.into())
}

pub fn item_url(id: ItemId) -> String {
    format!("https://news.ycombinator.com/item?id={id}")
}
pub fn reply_url(id: ItemId) -> String {
    format!("https://news.ycombinator.com/reply?id={id}")
}
pub fn submit_url() -> &'static str {
    "https://news.ycombinator.com/submit"
}
pub fn login_url() -> &'static str {
    "https://news.ycombinator.com/login?goto=news"
}
pub fn user_url(username: &str) -> String {
    let mut url = Url::parse("https://news.ycombinator.com/user").expect("static HN URL");
    url.query_pairs_mut().append_pair("id", username);
    url.into()
}

pub fn validate_username(username: &str) -> Result<(), crate::ReaderError> {
    if username.is_empty()
        || username.len() > 64
        || !username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(crate::ReaderError::InvalidInput(
            "Enter a valid Hacker News username".into(),
        ));
    }
    Ok(())
}

pub fn relative_time(timestamp: Option<u64>, now: u64) -> String {
    let Some(timestamp) = timestamp else {
        return "unknown time".into();
    };
    let age = now.saturating_sub(timestamp);
    match age {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", age / 60),
        3600..=86399 => format!("{}h ago", age / 3600),
        86400..=604799 => format!("{}d ago", age / 86400),
        604800..=31535999 => format!("{}w ago", age / 604800),
        _ => format!("{}y ago", age / 31536000),
    }
}
