use ferroui_base::input::InputElementImpl;
use ferroui_base::interactivity::InteractiveImpl;
use ferroui_base::layout::LayoutableImpl;
use ferroui_base::{
    ferro_class, ferro_class_info, ferro_impl_classes, ferro_properties, instantiate,
    FerroObjectImpl, FerroProperty, Ref, StyledElementImpl, StyledProperty, VisualImpl,
};
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
