//! The building blocks the views of Ember share: text, sections of a form, fields, choices
//! and buttons. Text takes the reading colour from the `ReaderView` it is in, and the WinUI
//! theme colours the controls.
use ferroui_base::layout::{HorizontalAlignment, Orientation, VerticalAlignment};
use ferroui_base::media::{FontWeight, TextWrapping};
use ferroui_base::{BoxedValue, CornerRadius, FerroObject, FerroProperty, Ref, Thickness};
use ferroui_controls::automation::AutomationProperties;
use ferroui_controls::{
    Border, Button, ComboBox, Control, Expander, ItemsSource, ScrollViewer, StackPanel, TextBlock,
    TextBox, ToolTip,
};
use ferroui_markup_xaml::markup_extensions::DynamicResourceExtension;
use std::rc::Rc;
/// Places related form fields side by side only when their own pane has room.
pub(crate) fn field_pair(parent: &StackPanel) -> (Ref<StackPanel>, Ref<StackPanel>) {
    use ferroui_controls::{ColumnDefinitions, Grid, RowDefinitions};

    let grid = Grid::new();
    grid.set_row_definitions(RowDefinitions::parse("Auto,Auto").expect("form rows"));
    grid.set_column_spacing(12.0);
    grid.set_row_spacing(8.0);
    let first = stack(4.0);
    let second = stack(4.0);
    Grid::set_row(&second, 1);
    grid.children().add(first.clone());
    grid.children().add(second.clone());
    let weak_grid = grid.downgrade();
    let weak_second = second.downgrade();
    grid.size_changed(move |_, e| {
        const TWO_COLUMNS: f64 = 400.0;
        let wide = e.new_size().width >= TWO_COLUMNS;
        if wide == (e.previous_size().width >= TWO_COLUMNS) {
            return;
        }
        if let (Some(grid), Some(second)) = (weak_grid.upgrade(), weak_second.upgrade()) {
            let (rows, columns, row, column) = if wide {
                ("Auto", "*,*", 0, 1)
            } else {
                ("Auto,Auto", "*", 1, 0)
            };
            grid.set_column_definitions(ColumnDefinitions::parse(columns).expect("form columns"));
            grid.set_row_definitions(RowDefinitions::parse(rows).expect("form rows"));
            Grid::set_row(&second, row);
            Grid::set_column(&second, column);
        }
    });
    parent.children().add(grid);
    (first, second)
}

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
    let content = stack(8.0);
    let title = label(title, 18.0, false);
    AutomationProperties::set_heading_level(&title, 2);
    content.children().add(title);
    let border = Border::new();
    border.set_padding(Thickness::uniform(14.0));
    border.set_corner_radius(CornerRadius::uniform(8.0));
    border.set_border_thickness(Thickness::uniform(1.0));
    bind_resource(
        &border,
        Border::background_property().as_property(),
        "EmberSurface",
    );
    bind_resource(
        &border,
        Border::border_brush_property().as_property(),
        "EmberBorder",
    );
    border.set_child(content.clone());
    parent.children().add(border);
    content
}
/// A native disclosure keeps the header keyboard-operable and exposes its expanded state.
pub(crate) fn disclosure(parent: &StackPanel, title: &str, expanded: bool) -> Ref<StackPanel> {
    let content = stack(8.0);
    let expander = Expander::new();
    expander.set_header(Some(Rc::new(title.to_owned()) as BoxedValue));
    expander.set_content(Some(Control::boxed(&content)));
    expander.set_horizontal_alignment(HorizontalAlignment::Stretch);
    expander.set_horizontal_content_alignment(HorizontalAlignment::Stretch);
    expander.set_padding(Thickness::uniform(12.0));
    expander.set_is_expanded(expanded);
    AutomationProperties::set_name(&expander, Some(title));
    parent.children().add(expander);
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
    let caption = label(label_text, 13.0, false);
    parent.children().add(caption.clone());
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
    AutomationProperties::set_labeled_by(&input, caption.upcast::<Control>());
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
    let caption = label(label_text, 13.0, false);
    parent.children().add(caption.clone());
    let input = ComboBox::new();
    input.set_items_source(Some(ItemsSource::from_values(
        values.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
    )));
    input.set_selected_index(selected);
    input.set_min_height(44.0);
    input.set_horizontal_alignment(HorizontalAlignment::Stretch);
    AutomationProperties::set_name(&input, Some(label_text));
    AutomationProperties::set_labeled_by(&input, caption.upcast::<Control>());
    parent.children().add(input.clone());
    input
}
/// An intrinsic-width action using the standard WinUI button or accent template.
pub(crate) fn form_button(label_text: &str) -> Ref<Button> {
    let button = Button::new();
    button.set_content(Some(Rc::new(label_text.to_owned()) as BoxedValue));
    button.set_min_height(44.0);
    button.set_padding(Thickness::symmetric(14.0, 6.0));
    button.set_horizontal_alignment(HorizontalAlignment::Left);
    AutomationProperties::set_name(&button, Some(label_text));
    button
}
pub(crate) fn heading(value: &str) -> Ref<TextBlock> {
    let block = text(value, 26.0);
    AutomationProperties::set_heading_level(&block, 1);
    block.set_font_weight(FontWeight::Bold);
    block
}
pub(crate) fn stack(spacing: f64) -> Ref<StackPanel> {
    let panel = StackPanel::new();
    panel.set_spacing(spacing);
    panel
}
/// Action glyphs share their name with pointer and keyboard tooltips.
pub(crate) fn icon_button(label: &str, icon: &str) -> Ref<Button> {
    let button = Button::new();
    button.classes().add("icon-button");
    button.set_content(Some(Control::boxed(&crate::icons::icon(icon))));
    AutomationProperties::set_name(&button, Some(label));
    ToolTip::set_tip(&button, Some(Rc::new(label.to_owned()) as BoxedValue));
    button
}

pub(crate) fn action(label: &str, icon: &str, invoke: impl Fn() + 'static) -> Ref<Button> {
    let button = Button::new();
    button.set_min_height(44.0);
    button.set_horizontal_content_alignment(HorizontalAlignment::Left);
    button.classes().add("quiet");
    button.set_content(Some(Control::boxed(&nav_content(icon, label, false))));
    AutomationProperties::set_name(&button, Some(label));
    button.click(move |_, _| invoke());
    button
}
pub(crate) fn nav_content(icon: &str, label: &str, vertical: bool) -> Ref<StackPanel> {
    let panel = stack(if vertical { 2.0 } else { 8.0 });
    panel.set_orientation(if vertical {
        Orientation::Vertical
    } else {
        Orientation::Horizontal
    });
    panel.set_vertical_alignment(VerticalAlignment::Center);
    let glyph = crate::icons::icon(icon);
    panel.children().add(glyph.upcast::<Control>());
    let label = text(label, if vertical { 11.0 } else { 13.0 });
    label.classes().add("nav-caption");
    label.set_vertical_alignment(VerticalAlignment::Center);
    label.set_horizontal_alignment(if vertical {
        HorizontalAlignment::Center
    } else {
        HorizontalAlignment::Left
    });
    panel.children().add(label.upcast::<Control>());
    panel
}
pub(crate) fn scroll(control: Ref<Control>) -> Ref<Control> {
    let scroll = ScrollViewer::new();
    scroll.set_content(Some(Control::boxed(&control)));
    scroll.upcast::<Control>()
}
pub(crate) fn welcome(open_search: impl Fn() + 'static) -> Ref<Control> {
    let panel = stack(12.0);
    panel.set_margin(Thickness::new(24.0, 52.0, 24.0, 24.0));
    panel.set_max_width(400.0);
    panel.set_horizontal_alignment(HorizontalAlignment::Center);
    let mark = Border::new();
    mark.set_width(48.0);
    mark.set_height(48.0);
    mark.set_corner_radius(CornerRadius::uniform(12.0));
    mark.set_horizontal_alignment(HorizontalAlignment::Left);
    bind_resource(
        &mark,
        Border::background_property().as_property(),
        "EmberTint",
    );
    mark.set_child(crate::icons::icon("chat"));
    panel.children().add(mark);
    panel.children().add(heading("Choose a story to read"));
    let body = label(
        "Select a headline to read the discussion, open the article, or save it for later.",
        15.0,
        true,
    );
    body.set_line_height(23.0);
    panel.children().add(body);
    let search = action("Search Hacker News", "search", open_search);
    search.set_horizontal_alignment(HorizontalAlignment::Left);
    panel.children().add(search);
    scroll(panel.upcast::<Control>())
}
