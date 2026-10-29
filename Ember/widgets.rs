//! The building blocks the views of Ember share: text, sections of a form, fields, choices
//! and buttons. Text takes the reading colour from the `ReaderView` it is in, and the WinUI
//! theme colours the controls.
use ferroui_base::layout::{HorizontalAlignment, Orientation, VerticalAlignment};
use ferroui_base::media::{FontWeight, TextWrapping};
use ferroui_base::{BoxedValue, CornerRadius, FerroObject, FerroProperty, Ref, Thickness};
use ferroui_controls::automation::AutomationProperties;
use ferroui_controls::{
    Border, Button, ComboBox, Control, ItemsSource, ScrollViewer, StackPanel, TextBlock, TextBox,
};
use ferroui_markup_xaml::markup_extensions::DynamicResourceExtension;
use std::rc::Rc;
pub(crate) fn text(value: &str, size: f64) -> Ref<TextBlock> {
    let block = TextBlock::new();
    block.set_text(Some(value));
    block.set_font_size(size);
    block.set_text_wrapping(TextWrapping::Wrap);
    block
}
/// Text of a form or a panel: semibold from 18 up, and in the secondary colour when `muted`.
pub(crate) fn label(value: &str, size: f64, muted: bool) -> Ref<TextBlock> {
    let block = text(value, size);
    if size >= 18.0 {
        block.set_font_weight(FontWeight::SemiBold);
    }
    if muted {
        block.classes().add("muted");
    }
    block
}
/// Small text in the secondary colour.
pub(crate) fn muted(value: &str) -> Ref<TextBlock> {
    label(value, 12.0, true)
}
/// Binds `property` of `target` to the dynamic resource `key`.
pub(crate) fn bind_resource(target: &FerroObject, property: &'static FerroProperty, key: &str) {
    let resource =
        DynamicResourceExtension::with_resource_key(Some(Rc::new(key.to_owned()) as BoxedValue));
    target.bind_binding(property, &*resource.as_binding());
}
/// A titled card of a form, added to `parent`; returns the panel its rows go in.
pub(crate) fn section(parent: &StackPanel, title: &str) -> Ref<StackPanel> {
    let content = stack(12.0);
    content.children().add(label(title, 19.0, false));
    let border = Border::new();
    border.set_padding(Thickness::uniform(20.0));
    border.set_corner_radius(CornerRadius::uniform(12.0));
    border.set_border_thickness(Thickness::uniform(1.0));
    bind_resource(&border, Border::background_property().as_property(), "EmberSurface");
    bind_resource(&border, Border::border_brush_property().as_property(), "EmberBorder");
    border.set_child(content.clone());
    parent.children().add(border);
    content
}
/// A labelled text field of at most `max_length` characters, added to `parent`.
pub(crate) fn field(
    parent: &StackPanel,
    label_text: &str,
    value: &str,
    placeholder: &str,
    max_length: i32,
    multiline: bool,
) -> Ref<TextBox> {
    parent.children().add(label(label_text, 13.0, false));
    let input = TextBox::new();
    input.set_text(Some(value));
    input.set_placeholder_text(Some(placeholder));
    input.set_min_height(44.0);
    input.set_max_length(max_length);
    if multiline {
        input.set_accepts_return(true);
        input.set_text_wrapping(TextWrapping::Wrap);
        input.set_min_height(76.0);
        input.set_max_height(160.0);
    }
    AutomationProperties::set_name(&input, Some(label_text));
    parent.children().add(input.clone());
    input
}
/// A labelled choice among `values`, added to `parent`.
pub(crate) fn combo(
    parent: &StackPanel,
    label_text: &str,
    values: &[&str],
    selected: i32,
) -> Ref<ComboBox> {
    parent.children().add(label(label_text, 13.0, false));
    let input = ComboBox::new();
    input.set_items_source(Some(ItemsSource::from_values(
        values.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
    )));
    input.set_selected_index(selected);
    input.set_min_height(44.0);
    input.set_horizontal_alignment(HorizontalAlignment::Stretch);
    AutomationProperties::set_name(&input, Some(label_text));
    parent.children().add(input.clone());
    input
}
/// A full-width form button; its text is plain content, so the WinUI button templates
/// (`accent` included) colour it.
pub(crate) fn form_button(label_text: &str) -> Ref<Button> {
    let button = Button::new();
    button.set_content(Some(Rc::new(label_text.to_owned()) as BoxedValue));
    button.set_min_height(44.0);
    button.set_padding(Thickness::symmetric(16.0, 10.0));
    button.set_horizontal_alignment(HorizontalAlignment::Stretch);
    AutomationProperties::set_name(&button, Some(label_text));
    button
}
pub(crate) fn heading(value: &str) -> Ref<TextBlock> {
    let block = text(value, 28.0);
    block.set_font_weight(FontWeight::Bold);
    block
}
pub(crate) fn stack(spacing: f64) -> Ref<StackPanel> {
    let panel = StackPanel::new();
    panel.set_spacing(spacing);
    panel
}
pub(crate) fn action(label: &str, icon: &str, invoke: impl Fn() + 'static) -> Ref<Button> {
    let button = Button::new();
    button.set_min_height(44.0);
    button.set_min_width(44.0);
    button.set_content(Some(Control::boxed(&nav_content(
        icon, label, false, false,
    ))));
    AutomationProperties::set_name(&button, Some(label));
    button.click(move |_, _| invoke());
    button
}
pub(crate) fn nav_content(
    icon: &str,
    label: &str,
    compact: bool,
    vertical: bool,
) -> Ref<StackPanel> {
    let panel = stack(if vertical { 4.0 } else { 12.0 });
    panel.set_orientation(if vertical {
        Orientation::Vertical
    } else {
        Orientation::Horizontal
    });
    panel.set_vertical_alignment(VerticalAlignment::Center);
    let glyph = crate::icons::icon(icon);
    panel.children().add(glyph.upcast::<Control>());
    if !compact {
        let label = text(label, if vertical { 10.0 } else { 13.0 });
        label.set_vertical_alignment(VerticalAlignment::Center);
        label.set_horizontal_alignment(if vertical {
            HorizontalAlignment::Center
        } else {
            HorizontalAlignment::Left
        });
        panel.children().add(label.upcast::<Control>());
    }
    panel
}
pub(crate) fn scroll(control: Ref<Control>) -> Ref<Control> {
    let scroll = ScrollViewer::new();
    scroll.set_content(Some(Control::boxed(&control)));
    scroll.upcast::<Control>()
}
pub(crate) fn welcome(open_search: impl Fn() + 'static) -> Ref<Control> {
    let panel = stack(22.0);
    panel.set_margin(Thickness::new(38.0, 60.0, 38.0, 36.0));
    panel.set_max_width(660.0);
    panel.set_horizontal_alignment(HorizontalAlignment::Center);
    let eyebrow = text("STAY CURIOUS.", 11.0);
    eyebrow.classes().add("eyebrow");
    panel.children().add(eyebrow.upcast::<Control>());
    let title = text("A little signal.\nA lot to discover.", 42.0);
    title.set_font_weight(FontWeight::Bold);
    title.set_line_height(50.0);
    panel.children().add(title.upcast::<Control>());
    let body = text(
        "A considered space for the stories, ideas and conversations that make Hacker News worth reading.",
        17.0,
    );
    body.classes().add("muted");
    body.set_line_height(28.0);
    panel.children().add(body.upcast::<Control>());
    panel
        .children()
        .add(action("Find your next rabbit hole", "search", open_search).upcast::<Control>());
    for (icon, title, description) in [
        (
            "chat",
            "Follow the conversation",
            "Thoughtful threads, native rich text, and room to read.",
        ),
        (
            "bookmark",
            "Make it yours",
            "Save a story, hide the noise, and pick up where you left off.",
        ),
        (
            "download",
            "Take it with you",
            "Download discussions and keep reading offline.",
        ),
    ] {
        let row = stack(8.0);
        row.set_margin(Thickness::new(0.0, 12.0, 0.0, 0.0));
        row.children()
            .add(nav_content(icon, title, false, false).upcast::<Control>());
        let caption = text(description, 13.0);
        caption.classes().add("muted");
        row.children().add(caption.upcast::<Control>());
        panel.children().add(row.upcast::<Control>());
    }
    scroll(panel.upcast::<Control>())
}
