use ember_content::HtmlDocument;
use ember_core::{relative_time, HnItem, ItemKind};
use ferroui_base::input::InputElementImpl;
use ferroui_base::interactivity::InteractiveImpl;
use ferroui_base::layout::LayoutableImpl;
use ferroui_base::{
    ferro_class, ferro_class_info, ferro_impl_classes, ferro_properties, instantiate,
    FerroObjectImpl, FerroProperty, Ref, StyledElementImpl, StyledProperty, VisualImpl,
};
use ferroui_controls::automation::AutomationProperties;
use ferroui_controls::primitives::TemplatedControlImpl;
use ferroui_controls::{Button, ButtonImpl, ContentControlImpl, ControlImpl};
/// A touch and keyboard activatable story, styled through a replaceable ControlTheme.
#[repr(C)]
pub struct StoryCard {
    base: Button,
}
ferro_class!(StoryCard: Button);
ferro_class_info!(StoryCard {
    new: StoryCard::new
});
ferro_impl_classes!(StoryCard: FerroObjectImpl, StyledElementImpl, VisualImpl, LayoutableImpl,
    InteractiveImpl, InputElementImpl, ControlImpl, TemplatedControlImpl, ContentControlImpl, ButtonImpl);
ferro_properties! {
    impl StoryCard {
        pub fn headline_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("Headline", None) }
        pub fn metadata_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("Metadata", None) }
        pub fn domain_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("Domain", None) }
        pub fn score_label_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("ScoreLabel", None) }
        pub fn status_label_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("StatusLabel", None) }
    }
}
impl StoryCard {
    pub fn construct() -> Self {
        Self {
            base: Button::construct(),
        }
    }
    pub fn new() -> Ref<Self> {
        instantiate(Self::construct())
    }
    /// Build the summary from the fields HN actually supplied. Jobs have no
    /// discussion count, and a comment search hit can share its parent story's title.
    pub(crate) fn set_item(&self, item: &HnItem, now: u64, show_dead: bool) {
        let summary = StorySummary::new(item, now, show_dead);
        self.set_headline(Some(&summary.headline));
        self.set_domain(Some(&summary.source));
        self.set_metadata(Some(&summary.byline));
        let score = item
            .score
            .map(|n| format!("{n} {}", if n == 1 { "pt" } else { "pts" }));
        self.set_score_label(score.as_deref());
        self.classes().set("has-score", score.is_some());
        let accessible = [
            Some(summary.headline),
            Some(summary.source),
            Some(summary.byline),
            score,
        ]
        .into_iter()
        .flatten()
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
        AutomationProperties::set_name(self, Some(&accessible));
    }
    pub fn headline(&self) -> Option<String> {
        self.get_value(Self::headline_property())
    }
    pub fn set_headline(&self, value: Option<&str>) {
        self.set_value(Self::headline_property(), value.map(str::to_owned));
    }
    pub fn metadata(&self) -> Option<String> {
        self.get_value(Self::metadata_property())
    }
    pub fn set_metadata(&self, value: Option<&str>) {
        self.set_value(Self::metadata_property(), value.map(str::to_owned));
    }
    pub fn domain(&self) -> Option<String> {
        self.get_value(Self::domain_property())
    }
    pub fn set_domain(&self, value: Option<&str>) {
        self.set_value(Self::domain_property(), value.map(str::to_owned));
    }
    pub fn score_label(&self) -> Option<String> {
        self.get_value(Self::score_label_property())
    }
    pub fn set_score_label(&self, value: Option<&str>) {
        self.set_value(Self::score_label_property(), value.map(str::to_owned));
    }
    pub fn status_label(&self) -> Option<String> {
        self.get_value(Self::status_label_property())
    }
    pub fn set_status_label(&self, value: Option<&str>) {
        self.set_value(Self::status_label_property(), value.map(str::to_owned));
    }
}

struct StorySummary {
    source: String,
    headline: String,
    byline: String,
}

impl StorySummary {
    fn new(item: &HnItem, now: u64, show_dead: bool) -> Self {
        let comment = item.kind == Some(ItemKind::Comment);
        let headline = if item.deleted {
            "[deleted]".to_owned()
        } else if item.dead && !show_dead {
            "[dead]".to_owned()
        } else {
            let text = if comment {
                item.text.as_deref().or(item.title.as_deref())
            } else {
                item.title.as_deref().or(item.text.as_deref())
            };
            let decoded = HtmlDocument::parse(text.unwrap_or_default()).plain_text();
            // A search hit or saved comment can contain several pages of prose.
            // Keep the feed preview and its accessibility announcement bounded.
            let mut chars = decoded.chars();
            let mut preview: String = chars.by_ref().take(240).collect();
            if chars.next().is_some() {
                preview.push('…');
            }
            if preview.trim().is_empty() {
                if comment { "Comment" } else { "Discussion" }.to_owned()
            } else {
                preview
            }
        };
        let source = match item.kind {
            Some(ItemKind::Comment) if !item.deleted && (!item.dead || show_dead) => item
                .title
                .as_deref()
                .map(|title| format!("Comment on {}", HtmlDocument::parse(title).plain_text()))
                .unwrap_or_else(|| "Comment on Hacker News".to_owned()),
            Some(ItemKind::Job) => format!("Job · {}", item.domain()),
            Some(ItemKind::Poll) => "Hacker News poll".to_owned(),
            _ if item.url.is_none() => "Hacker News".to_owned(),
            _ => item.domain(),
        };
        let mut byline = Vec::new();
        if let Some(author) = item.by.as_ref().filter(|_| !item.deleted) {
            byline.push(author.clone());
        }
        if item.time.is_some() {
            byline.push(relative_time(item.time, now));
        }
        if let Some(n) = item.descendants {
            byline.push(format!(
                "{n} {}",
                if n == 1 { "comment" } else { "comments" }
            ));
        }
        Self {
            source,
            headline,
            byline: byline.join(" · "),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_and_comment_results_show_their_own_metadata() {
        let item = HnItem {
            kind: Some(ItemKind::Job),
            title: Some("Engineer".into()),
            ..Default::default()
        };
        let summary = StorySummary::new(&item, 0, false);
        assert_eq!(summary.byline, "");
        assert!(summary.source.starts_with("Job · "));
        let item = HnItem {
            kind: Some(ItemKind::Comment),
            title: Some("Parent &amp; story".into()),
            text: Some("<p>Matching <b>reply</b></p>".into()),
            by: Some("reader".into()),
            ..Default::default()
        };
        let summary = StorySummary::new(&item, 0, false);
        assert_eq!(summary.headline, "Matching reply");
        assert_eq!(summary.source, "Comment on Parent & story");
        assert_eq!(summary.byline, "reader");
    }
}
