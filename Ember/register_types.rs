use crate::controls::StoryCard;
use crate::markup::XamlClass;
use crate::{App, ReaderView};
use ferroui_base::metadata::MarkupAssembly;
use ferroui_base::TypeInfo;
const NAMESPACES: &[(&str, &str)] = &[("ember", "Ember"), ("ember::controls", "Ember.Controls")];
const TYPES: &[&TypeInfo] = &[App::TYPE, ReaderView::TYPE, StoryCard::TYPE];
pub static ASSEMBLY: MarkupAssembly = MarkupAssembly {
    name: "Ember",
    crate_name: "ember",
    xmlns_definitions: &[],
    xmlns_prefixes: &[],
    metadata: &[],
};
pub(crate) const CLASSES: &[&XamlClass] = &[&App::XAML_CLASS, &ReaderView::XAML_CLASS];
pub fn register_types() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        ferroui_markup_xaml::register_types();
        ferroui_themes_winui::register_types();
        TypeInfo::register_namespaces(NAMESPACES);
        TypeInfo::register_all(TYPES);
        MarkupAssembly::register(&ASSEMBLY);
        crate::SAMPLE.register_assets();
        crate::compiled_markup::register();
    });
}
