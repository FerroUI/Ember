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

#[cfg(test)]
mod tests {
    use super::*;
    use ferroui_base::media::{Brushes, TextWrapping};
    use ferroui_base::{Thickness, Vector};
    use ferroui_controls::documents::Run;
    use ferroui_controls::{Button, ScrollViewer, SelectableTextBlock, Window};
    use sample_testing::Shell;
    use std::cell::RefCell;

    #[test]
    fn code_tools_wrap_only_the_chosen_block_and_copy_exact_source_even_during_find() {
        let shell = Shell::start(&crate::SAMPLE, 360.0, 720.0, || crate::App::new().upcast());
        let html = format!("<blockquote><pre>\tlet MARK = &quot;λ &amp; {}&quot;;\n  finish();\n</pre></blockquote><pre>second();</pre>", "long_name_".repeat(12));
        let expected = format!(
            "\tlet MARK = \"λ & {}\";\n  finish();\n",
            "long_name_".repeat(12)
        );
        let document = HtmlDocument::parse(&html);
        let copied = Rc::new(RefCell::new(Vec::new()));
        let writes = copied.clone();
        let content = render(
            &document,
            &crate::thread_view::render_options(&Default::default()),
            Some(FindHighlight {
                query: "mark",
                foreground: Brushes::black(),
                background: Brushes::yellow(),
            }),
            Rc::new(|_| panic!("copy must never navigate")),
            Rc::new(move |source| writes.borrow_mut().push(source.to_owned())),
        );
        content.set_margin(Thickness::uniform(12.0));
        let window = Window::new();
        window.set_width(360.0);
        window.set_height(720.0);
        window.set_content(Some(Control::boxed(&content)));
        window.show();
        shell.settle();

        let buttons = |name: &str| {
            content
                .get_visual_descendants()
                .filter_map(|v| v.cast::<Button>())
                .filter(|b| AutomationProperties::get_name(b).as_deref() == Some(name))
                .collect::<Vec<_>>()
        };
        let wraps = buttons("Wrap code lines");
        let copies = buttons("Copy code");
        let scrolls: Vec<_> = content
            .get_visual_descendants()
            .filter_map(|v| v.cast::<ScrollViewer>())
            .filter(|v| AutomationProperties::get_name(v).as_deref() == Some("Code block"))
            .collect();
        assert_eq!((wraps.len(), copies.len(), scrolls.len()), (2, 2, 2));
        let text = scrolls[0]
            .get_visual_descendants()
            .find_map(|v| v.cast::<SelectableTextBlock>())
            .unwrap();
        assert!(text
            .get_logical_descendants()
            .filter_map(|v| v.cast::<Run>())
            .any(|run| run.text().as_deref() == Some("MARK") && run.background().is_some()));
        assert!(scrolls[0].extent().width > scrolls[0].viewport().width);
        let unwrapped_height = text.bounds().height;
        scrolls[0].set_offset(Vector::new(42.0, 0.0));
        let tap = |button: &Button| {
            let bounds = Shell::bounds_of(&window, button);
            assert!(bounds.width >= 44.0 && bounds.height >= 44.0);
            shell.click(0, bounds.center());
            shell.settle();
        };
        text.set_selection_start(1);
        text.set_selection_end(4);
        assert_eq!(text.selected_text(), "let");
        tap(&wraps[0]);
        assert_eq!(text.text_wrapping(), TextWrapping::Wrap);
        assert_eq!(text.selected_text(), "let");
        assert_eq!(scrolls[0].offset().x, 0.0);
        assert!(scrolls[0].extent().width <= scrolls[0].viewport().width + 1.0);
        assert!(
            text.bounds().height > unwrapped_height,
            "long code must reflow vertically"
        );
        assert_eq!(
            wraps[1]
                .clone()
                .cast::<ToggleButton>()
                .unwrap()
                .is_checked(),
            Some(false)
        );
        tap(&copies[0]);
        tap(&copies[1]);
        tap(&wraps[0]);
        assert_eq!(&*copied.borrow(), &[expected, "second();".to_owned()]);
        assert_eq!(text.text_wrapping(), TextWrapping::NoWrap);
        assert!(scrolls[0].extent().width > scrolls[0].viewport().width);
        window.close();
    }
}
