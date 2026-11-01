use ember_content::HtmlDocument;
use ember_core::{relative_time, HnItem, ItemKind};
use ferroui_base::input::InputElementImpl;
use ferroui_base::interactivity::{InteractiveImpl, RoutedEventArgs};
use ferroui_base::layout::LayoutableImpl;
use ferroui_base::media::TranslateTransform;
use ferroui_base::{
    ferro_class, ferro_class_info, ferro_impl_classes, ferro_properties, instantiate,
    FerroObjectImpl, FerroProperty, Ref, StyledElementImpl, StyledProperty, VisualImpl,
};
use ferroui_controls::automation::AutomationProperties;
use ferroui_controls::primitives::{
    TemplateAppliedEventArgs, TemplatedControlImpl, TemplatedControlImplExt,
};
use ferroui_controls::{
    Border, Button, ButtonImpl, ContentControlImpl, ControlImpl, ControlImplExt,
};
use std::cell::{Cell, RefCell};
/// A touch and keyboard activatable story, styled through a replaceable ControlTheme.
#[repr(C)]
pub struct StoryCard {
    base: Button,
    swipe_distance: Cell<f64>,
    swipe_surface: RefCell<Option<Ref<Border>>>,
    swipe_transform: RefCell<Option<Ref<TranslateTransform>>>,
}
ferro_class!(StoryCard: Button);
ferro_class_info!(StoryCard {
    new: StoryCard::new
});
ferro_impl_classes!(StoryCard: FerroObjectImpl, StyledElementImpl, VisualImpl, LayoutableImpl,
    InteractiveImpl, InputElementImpl, ContentControlImpl, ButtonImpl);

impl TemplatedControlImpl for StoryCard {
    fn on_apply_template(this: &Self, args: &TemplateAppliedEventArgs) {
        Self::parent_on_apply_template(this, args);
        this.finish_swipe();
        this.swipe_surface
            .replace(args.name_scope().find_as::<Border>("CardSurface"));
    }
}
impl ControlImpl for StoryCard {
    fn on_unloaded(this: &Self, args: &RoutedEventArgs) {
        this.finish_swipe();
        Self::parent_on_unloaded(this, args);
    }
}
ferro_properties! {
    impl StoryCard {
        pub fn swipe_label_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("SwipeLabel", None) }
        pub fn headline_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("Headline", None) }
        pub fn compact_metadata_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("CompactMetadata", None) }
        pub fn metadata_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("Metadata", None) }
        pub fn domain_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("Domain", None) }
        pub fn score_label_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("ScoreLabel", None) }
        pub fn comment_count_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("CommentCount", None) }
        pub fn status_label_property() -> StyledProperty<Option<String>> { FerroProperty::register::<StoryCard,_>("StatusLabel", None) }
    }
}
impl StoryCard {
    pub(crate) const SWIPE_THRESHOLD: f64 = 80.0;

    pub fn construct() -> Self {
        Self {
            base: Button::construct(),
            swipe_distance: Cell::new(0.0),
            swipe_surface: RefCell::default(),
            swipe_transform: RefCell::default(),
        }
    }
    pub fn new() -> Ref<Self> {
        instantiate(Self::construct())
    }
    /// Retain the full gesture distance for reversal while limiting visual travel.
    pub(crate) fn track_swipe(&self, displacement: f64) {
        let distance = self.swipe_distance.get() + displacement;
        self.swipe_distance.set(distance);
        let surface = self.swipe_surface.borrow();
        let Some(surface) = surface.as_ref() else {
            return;
        };
        let mut current = self.swipe_transform.borrow_mut();
        let transform = current.get_or_insert_with(|| {
            let transform = TranslateTransform::new();
            surface.set_render_transform(Some(transform.clone().into()));
            transform
        });
        transform.set_x(distance.clamp(-96.0, 96.0));
        let save = distance < 0.0;
        self.classes().set("swipe-save", save);
        self.classes().set("swipe-hide", distance > 0.0);
        self.classes()
            .set("swipe-ready", distance.abs() > Self::SWIPE_THRESHOLD);
        let label = if save {
            if self.classes().contains("saved") {
                "Unsave"
            } else {
                "Save"
            }
        } else if self.classes().contains("hidden") {
            "Restore"
        } else {
            "Hide"
        };
        if self.get_value(Self::swipe_label_property()).as_deref() != Some(label) {
            self.set_value(Self::swipe_label_property(), Some(label.to_owned()));
        }
    }

    pub(crate) fn finish_swipe(&self) -> f64 {
        if self.swipe_transform.take().is_some() {
            if let Some(surface) = self.swipe_surface.borrow().as_ref() {
                surface.set_render_transform(None);
            }
            for name in ["swipe-save", "swipe-hide", "swipe-ready"] {
                self.classes().set(name, false);
            }
        }
        self.swipe_distance.replace(0.0)
    }

    /// Build the summary from the fields HN actually supplied. Jobs have no
    /// discussion count, and a comment search hit can share its parent story's title.
    pub(crate) fn set_item(&self, item: &HnItem, now: u64, show_dead: bool) {
        let summary = StorySummary::new(item, now, show_dead);
        self.set_headline(Some(&summary.headline));
        self.set_domain(Some(&summary.source));
        self.set_metadata(Some(&summary.byline));
        let compact_meta = if summary.byline.is_empty() {
            summary.source.clone()
        } else {
            format!("{} · {}", summary.source, summary.byline)
        };
        self.set_value(Self::compact_metadata_property(), Some(compact_meta));
        self.set_score_label(item.score.map(|n| n.to_string()).as_deref());
        self.set_value(
            Self::comment_count_property(),
            item.descendants.map(|n| n.to_string()),
        );
        self.classes().set("has-score", item.score.is_some());
        self.classes()
            .set("has-comments", item.descendants.is_some());
        let accessible = [
            Some(summary.headline),
            Some(summary.source),
            Some(summary.byline),
            item.score
                .map(|n| format!("{n} {}", if n == 1 { "point" } else { "points" })),
            item.descendants
                .map(|n| format!("{n} {}", if n == 1 { "comment" } else { "comments" })),
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

pub(crate) struct StorySummary {
    pub source: String,
    pub headline: String,
    pub byline: String,
}

impl StorySummary {
    pub(crate) fn new(item: &HnItem, now: u64, show_dead: bool) -> Self {
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
