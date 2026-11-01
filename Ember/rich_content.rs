//! Reader commands for the renderer's selectable document and preformatted samples.
use ember_content::{FindHighlight, HtmlDocument, RenderOptions};
use ferroui_base::layout::VerticalAlignment;
use ferroui_base::{BoxedValue, Ref};
use ferroui_controls::automation::AutomationProperties;
use ferroui_controls::primitives::ToggleButton;
use ferroui_controls::{ColumnDefinitions, Control, Grid, ToolTip};
use std::rc::Rc;

pub(crate) fn render(
    document: &HtmlDocument,
    options: &RenderOptions,
    highlight: Option<FindHighlight<'_>>,
    open_link: Rc<dyn Fn(&str)>,
    copy_code: Rc<dyn Fn(&str)>,
) -> Ref<Control> {
    ember_content::render_with_code_toolbar(
        document,
        options,
        highlight,
        open_link,
        &|code, wrap| {
            let header = Grid::new();
            header.set_column_definitions(
                ColumnDefinitions::parse("*,Auto,Auto").expect("code tools"),
            );
            header.set_column_spacing(3.0);
            let lines = code.lines().count().max(1);
            let count = crate::widgets::muted(&format!(
                "{} {}",
                lines,
                if lines == 1 { "line" } else { "lines" }
            ));
            count.set_vertical_alignment(VerticalAlignment::Center);
            header.children().add(count);

            let toggle = ToggleButton::new();
            toggle.classes().add("feed-filter");
            toggle.set_content(Some(Rc::new("Wrap".to_owned())));
            AutomationProperties::set_name(&toggle, Some("Wrap code lines"));
            ToolTip::set_tip(
                &toggle,
                Some(Rc::new("Wrap code lines".to_owned()) as BoxedValue),
            );
            let weak = toggle.downgrade();
            toggle.is_checked_changed(move |_, _| {
                if let Some(toggle) = weak.upgrade() {
                    let enabled = toggle.is_checked() == Some(true);
                    toggle.classes().set("active", enabled);
                    wrap(enabled);
                }
            });
            Grid::set_column(&toggle, 1);
            header.children().add(toggle);

            let copy = crate::widgets::icon_button("Copy code", "copy");
            let source = code.to_owned();
            let write = copy_code.clone();
            copy.click(move |_, _| write(&source));
            Grid::set_column(&copy, 2);
            header.children().add(copy);
            header.upcast()
        },
    )
}
