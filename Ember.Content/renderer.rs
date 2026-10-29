use crate::{Block, HtmlDocument, Inline};
use ferroui_base::input::InputElementImpl;
use ferroui_base::interactivity::InteractiveImpl;
use ferroui_base::layout::{
    HorizontalAlignment, LayoutableImpl, LayoutableImplExt, VerticalAlignment,
};
use ferroui_base::media::{Brushes, FontFamily, FontWeight, IBrush, TextDecorations, TextWrapping};
use ferroui_base::{
    ferro_class, ferro_class_info, ferro_impl_classes, instantiate, CornerRadius, FerroObjectImpl,
    Ref, Size, StaticType, StyledElementImpl, Thickness, TypeInfo, VisualImpl,
};
use ferroui_controls::automation::AutomationProperties;
use ferroui_controls::documents::{
    Bold, InlineCollection, Italic, LineBreak, Run, Span, Underline,
};
use ferroui_controls::primitives::{ScrollBarVisibility, TemplatedControlImpl};
use ferroui_controls::{
    Border, ButtonImpl, ColumnDefinitions, ContentControlImpl, Control, ControlImpl, Grid,
    HyperlinkButton, ScrollViewer, SelectableTextBlock, StackPanel, TextBlock,
};
use std::rc::Rc;

/// Appearance comes from the host's current theme. Omitted text brushes inherit from the
/// containing FerroUI control, so light/dark changes are not baked into the document model.
#[derive(Clone)]
pub struct RenderOptions {
    pub font_size: f64,
    /// Absolute line height in logical pixels; 0 chooses 1.55 times `font_size`.
    pub line_height: f64,
    pub paragraph_spacing: f64,
    pub foreground: Option<Rc<dyn IBrush>>,
    pub link_brush: Option<Rc<dyn IBrush>>,
    pub code_background: Option<Rc<dyn IBrush>>,
    pub quote_brush: Option<Rc<dyn IBrush>>,
    pub code_font_family: FontFamily,
    /// Links are real keyboard-focusable controls. Set 44 or greater for touch layouts.
    pub link_min_height: f64,
    /// Limit a long inline link to this width so its caption can wrap on narrow screens.
    pub max_link_width: f64,
    /// Keep a long preformatted block inside a scrollable viewport, even in an unbounded row.
    pub max_code_height: f64,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            line_height: 0.0,
            paragraph_spacing: 12.0,
            foreground: None,
            link_brush: None,
            code_background: None,
            quote_brush: None,
            code_font_family: FontFamily::new("Cascadia Code, SFMono-Regular, Consolas, monospace"),
            link_min_height: 32.0,
            max_link_width: 320.0,
            max_code_height: 420.0,
        }
    }
}

/// Render an inert document as native controls, with selectable text and keyboard- and
/// pointer-operable links. `activate_link` receives only normalized HTTP/HTTPS destinations.
/// The callback must capture the host weakly if the host owns this returned control.
pub fn render(
    document: &HtmlDocument,
    options: &RenderOptions,
    activate_link: Rc<dyn Fn(&str)>,
) -> Ref<Control> {
    let root = render_blocks(&document.blocks, options, &activate_link);
    root.classes().add("ember-html");
    if document.truncated {
        let notice = text_block(options);
        notice.set_text(Some(
            "Content shortened for performance. Open the original on Hacker News to read more.",
        ));
        notice.set_opacity(0.65);
        notice.set_font_size((options.font_size * 0.85).max(12.0));
        root.children().add(notice);
    }
    root.upcast()
}

fn render_blocks(
    blocks: &[Block],
    options: &RenderOptions,
    activate_link: &Rc<dyn Fn(&str)>,
) -> Ref<StackPanel> {
    let panel = StackPanel::new();
    panel.set_spacing(finite(options.paragraph_spacing, 12.0).clamp(0.0, 64.0));
    for block in blocks {
        match block {
            Block::Paragraph(inlines) => {
                panel
                    .children()
                    .add(paragraph(inlines, options, activate_link));
            }
            Block::Heading { level, content } => {
                let scale = match level {
                    1 => 1.65,
                    2 => 1.4,
                    3 => 1.2,
                    _ => 1.05,
                };
                let mut heading_options = options.clone();
                heading_options.font_size =
                    finite(options.font_size, 16.0).clamp(10.0, 48.0) * scale;
                heading_options.line_height = heading_options.font_size * 1.3;
                let heading = paragraph(content, &heading_options, activate_link);
                heading.set_font_weight(FontWeight::SemiBold);
                heading.classes().add("ember-html-heading");
                panel.children().add(heading);
            }
            Block::Quote(children) => {
                let border = Border::new();
                border.classes().add("ember-html-quote");
                border.set_border_thickness(Thickness::new(3.0, 0.0, 0.0, 0.0));
                border.set_border_brush(
                    options
                        .quote_brush
                        .clone()
                        .or_else(|| Some(Brushes::gray())),
                );
                border.set_padding(Thickness::new(14.0, 4.0, 0.0, 4.0));
                border.set_child(render_blocks(children, options, activate_link));
                panel.children().add(border);
            }
            Block::List {
                ordered,
                start,
                items,
            } => {
                let list = StackPanel::new();
                list.set_spacing(8.0);
                list.classes().add("ember-html-list");
                for (index, item) in items.iter().enumerate() {
                    let row = Grid::new();
                    row.set_column_definitions(
                        ColumnDefinitions::parse("Auto,*").expect("constant grid columns"),
                    );
                    let marker = text_block(options);
                    marker.set_text(Some(&if *ordered {
                        format!("{}.", start.saturating_add(index as i64))
                    } else {
                        "•".to_owned()
                    }));
                    marker.set_min_width(22.0);
                    marker.set_margin(Thickness::new(0.0, 0.0, 8.0, 0.0));
                    let content = render_blocks(item, options, activate_link);
                    Grid::set_column(&content, 1);
                    row.children().add(marker);
                    row.children().add(content);
                    list.children().add(row);
                }
                panel.children().add(list);
            }
            Block::Code(code) => {
                let text = text_block(options);
                text.set_text(Some(code));
                text.set_text_wrapping(TextWrapping::NoWrap);
                text.set_font_family(options.code_font_family.clone());
                text.set_font_size((finite(options.font_size, 16.0) * 0.9).clamp(10.0, 36.0));
                let scroll = ScrollViewer::new();
                scroll.set_horizontal_scroll_bar_visibility(ScrollBarVisibility::Auto);
                scroll.set_vertical_scroll_bar_visibility(ScrollBarVisibility::Auto);
                scroll.set_is_scroll_chaining_enabled(true);
                scroll.set_max_height(finite(options.max_code_height, 420.0).clamp(120.0, 2000.0));
                AutomationProperties::set_name(&scroll, Some("Code block"));
                AutomationProperties::set_help_text(
                    &scroll,
                    Some("Scroll vertically or horizontally to read the code."),
                );
                scroll.set_content(Some(Control::boxed(&text)));
                let border = Border::new();
                border.classes().add("ember-html-code");
                border.set_corner_radius(CornerRadius::uniform(8.0));
                border.set_padding(Thickness::uniform(12.0));
                border.set_background(options.code_background.clone());
                border.set_child(scroll);
                panel.children().add(border);
            }
            Block::Rule => {
                let rule = Border::new();
                rule.set_height(1.0);
                rule.set_background(
                    options
                        .quote_brush
                        .clone()
                        .or_else(|| Some(Brushes::gray())),
                );
                rule.set_opacity(0.4);
                panel.children().add(rule);
            }
        }
    }
    panel
}

fn paragraph(
    inlines: &[Inline],
    options: &RenderOptions,
    activate_link: &Rc<dyn Fn(&str)>,
) -> Ref<SelectableTextBlock> {
    let text = text_block(options);
    if contains_link(inlines) {
        // A fixed text line height clamps an embedded control's natural height. A wrapped
        // caption or 44px touch target would then overlap the following line/paragraph.
        // Let the inline control establish its height while preserving extra reader spacing.
        let extra_spacing = (text.line_height() - text.font_size() * 1.2).max(0.0);
        text.set_line_height(f64::NAN);
        text.set_line_spacing(extra_spacing);
    }
    let collection = InlineCollection::new();
    add_inlines(inlines, &collection, options, activate_link);
    text.set_inlines(Some(collection));
    text
}

fn contains_link(inlines: &[Inline]) -> bool {
    inlines.iter().any(|inline| match inline {
        Inline::Link { .. } => true,
        Inline::Strong(children)
        | Inline::Emphasis(children)
        | Inline::Underline(children)
        | Inline::Strike(children) => contains_link(children),
        _ => false,
    })
}

fn text_block(options: &RenderOptions) -> Ref<SelectableTextBlock> {
    let text = SelectableTextBlock::new();
    let font_size = finite(options.font_size, 16.0).clamp(10.0, 48.0);
    text.set_font_size(font_size);
    text.set_line_height(
        if options.line_height.is_finite() && options.line_height >= font_size {
            options.line_height.min(120.0)
        } else {
            font_size * 1.55
        },
    );
    text.set_text_wrapping(TextWrapping::Wrap);
    if let Some(brush) = &options.foreground {
        text.set_foreground(Some(brush.clone()));
    }
    text
}

fn add_inlines(
    inlines: &[Inline],
    output: &InlineCollection,
    options: &RenderOptions,
    activate_link: &Rc<dyn Fn(&str)>,
) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => output.add(Run::with_text(Some(text))),
            Inline::LineBreak => output.add(LineBreak::new()),
            Inline::Strong(children) => {
                let span = Bold::new();
                add_inlines(children, &span.inlines(), options, activate_link);
                output.add(span);
            }
            Inline::Emphasis(children) => {
                let span = Italic::new();
                add_inlines(children, &span.inlines(), options, activate_link);
                output.add(span);
            }
            Inline::Underline(children) => {
                let span = Underline::new();
                add_inlines(children, &span.inlines(), options, activate_link);
                output.add(span);
            }
            Inline::Strike(children) => {
                let span = Span::new();
                span.set_text_decorations(Some(TextDecorations::strikethrough()));
                add_inlines(children, &span.inlines(), options, activate_link);
                output.add(span);
            }
            Inline::Code(code) => {
                let run = Run::with_text(Some(code));
                run.set_font_family(options.code_font_family.clone());
                run.set_font_size((finite(options.font_size, 16.0) * 0.9).clamp(10.0, 36.0));
                run.set_background(options.code_background.clone());
                output.add(run);
            }
            Inline::Link { url, content } => {
                // Defense in depth: the public document model can be constructed manually.
                let Some(destination) = crate::safe_link(url) else {
                    add_inlines(content, output, options, activate_link);
                    continue;
                };
                let link = InlineLink::new();
                link.classes().add("ember-html-link");
                link.set_padding(Thickness::symmetric(2.0, 0.0));
                link.set_min_height(finite(options.link_min_height, 32.0).clamp(24.0, 80.0));
                link.set_max_width(finite(options.max_link_width, 320.0).clamp(80.0, 1600.0));
                link.set_horizontal_content_alignment(HorizontalAlignment::Left);
                link.set_vertical_content_alignment(VerticalAlignment::Center);
                link.set_font_size(finite(options.font_size, 16.0).clamp(10.0, 48.0));
                if let Some(brush) = &options.link_brush {
                    link.set_foreground(Some(brush.clone()));
                }
                let label = Inline::plain_text(content);
                let caption = &link.caption;
                caption.set_text(Some(&label));
                caption.set_text_wrapping(TextWrapping::Wrap);
                caption.set_text_decorations(Some(TextDecorations::underline()));
                if let Some(brush) = &options.link_brush {
                    caption.set_foreground(Some(brush.clone()));
                }
                link.set_content(Some(Control::boxed(caption)));
                AutomationProperties::set_name(&link, Some(&label));
                AutomationProperties::set_help_text(&link, Some(&destination));
                // The destination is shown before the link is followed, whatever its caption says.
                ferroui_controls::ToolTip::set_tip(
                    &link,
                    Some(Rc::new(destination.clone()) as ferroui_base::BoxedValue),
                );
                // Leave NavigateUri unset: only the host decides how to launch, route internal
                // HN links, or copy a destination. There is never a second implicit launch.
                let activate = activate_link.clone();
                link.click(move |_, _| activate(&destination));
                output.add_control(link);
            }
        }
    }
}

/// An embedded control otherwise uses its bottom edge as a text baseline. Publish the
/// caption's measured baseline instead, so a centered touch target stays aligned with the
/// surrounding sentence, including when its caption wraps over several lines.
#[repr(C)]
struct InlineLink {
    base: HyperlinkButton,
    caption: Ref<TextBlock>,
}

ferro_class!(InlineLink: HyperlinkButton);
ferro_class_info!(InlineLink {
    new: InlineLink::new
});
ferro_impl_classes!(
    InlineLink: FerroObjectImpl,
    VisualImpl,
    InteractiveImpl,
    InputElementImpl,
    ControlImpl,
    TemplatedControlImpl,
    ContentControlImpl,
    ButtonImpl
);

impl StyledElementImpl for InlineLink {
    fn style_key_override(_this: &Self) -> &'static TypeInfo {
        <HyperlinkButton as StaticType>::TYPE
    }
}

impl LayoutableImpl for InlineLink {
    fn measure_core(this: &Self, available_size: Size) -> Size {
        let size = Self::parent_measure_core(this, available_size);
        let caption = &this.caption;
        let top = ((size.height - caption.desired_size().height) / 2.0).max(0.0);
        let baseline = top
            + caption.padding().top
            + caption.layout_rounding_offset()
            + caption.text_layout().baseline();
        TextBlock::set_baseline_offset_on(this, baseline);
        size
    }
}

impl InlineLink {
    fn new() -> Ref<Self> {
        instantiate(Self {
            base: HyperlinkButton::construct(),
            caption: TextBlock::new(),
        })
    }
}

fn finite(value: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}
