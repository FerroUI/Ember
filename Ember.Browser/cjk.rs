//! Load a real fallback face only when the text formatter requests a missing CJK glyph.
use ember_core::BoxFuture;
use ferroui_base::media::fonts::{FontCollectionBase, FontCollectionBaseImpl};
use ferroui_base::media::{
    FontFallback, FontFamily, FontManagerOptions, FontStretch, FontStyle, FontWeight,
    GlyphTypeface, UnicodeRange, UnicodeRangeSegment,
};
use ferroui_base::threading::Dispatcher;
use ferroui_base::utilities::Uri;
use ferroui_base::{Ref, Visual};
use ferroui_controls::presenters::TextPresenter;
use ferroui_controls::{AppBuilder, Application, TextBlock, TextBox, TopLevel};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

thread_local! {
    static ACTIVE: RefCell<Weak<CjkCollection>> = RefCell::new(Weak::new());
}
const RETRY_DELAY: Duration = Duration::from_secs(15);
const FAMILY: &str = "Noto Sans CJK SC";
const COLLECTION: &str = "fonts:EmberCjk";
type LoadFont = fn() -> BoxFuture<'static, Option<Vec<u8>>>;

fn fallback_range() -> UnicodeRange {
    UnicodeRange::from_segments(
        [
            (0x1100, 0x11ff), // Hangul jamo
            (0x2e80, 0x2fff), // Han radicals and ideographic descriptions
            (0x3000, 0x33ff), // CJK punctuation, kana, bopomofo and compatibility forms
            (0x3400, 0x4dbf),
            (0x4e00, 0x9fff), // Unified ideographs
            (0xa960, 0xa97f),
            (0xac00, 0xd7ff), // Hangul extensions and syllables
            (0xf900, 0xfaff),
            (0xfe10, 0xfe1f),
            (0xfe30, 0xfe4f),
            (0xff01, 0xffef),
            (0x1aff0, 0x1b16f),
            (0x20000, 0x2fa1f),
            (0x30000, 0x323af),
        ]
        .into_iter()
        .map(|(start, end)| UnicodeRangeSegment::new(start, end))
        .collect(),
    )
}

#[derive(Clone, Copy)]
enum LoadState {
    Idle,
    Loading,
    Ready,
    RetryAfter(Instant),
}

struct CjkCollection {
    base: FontCollectionBase,
    key: Uri,
    weak: Weak<Self>,
    load: LoadFont,
    state: Cell<LoadState>,
    last_online_retry: Cell<Option<Instant>>,
}

impl CjkCollection {
    fn new(load: LoadFont) -> Rc<Self> {
        Rc::new_cyclic(|weak| Self {
            base: FontCollectionBase::new(),
            key: Uri::absolute(COLLECTION).expect("a valid CJK collection key"),
            weak: weak.clone(),
            load,
            state: Cell::new(LoadState::Idle),
            last_online_retry: Cell::new(None),
        })
    }

    fn retry_on_connectivity(&self) {
        if matches!(self.state.get(), LoadState::RetryAfter(_))
            && self
                .last_online_retry
                .get()
                .is_none_or(|last| last.elapsed() >= RETRY_DELAY)
        {
            self.last_online_retry.set(Some(Instant::now()));
            self.state.set(LoadState::Idle);
            self.request();
        }
    }

    fn request(&self) {
        match self.state.get() {
            LoadState::Ready | LoadState::Loading => return,
            LoadState::RetryAfter(time) if Instant::now() < time => return,
            _ => {}
        }
        self.state.set(LoadState::Loading);
        let weak = self.weak.clone();
        let load = self.load;
        // Finish shaping this frame before registering the downloaded face or
        // invalidating controls. The fetch future resumes on the UI dispatcher.
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let bytes = load().await;
                let Some(collection) = weak.upgrade() else {
                    return;
                };
                let font = bytes.and_then(|bytes| {
                    FontCollectionBase::try_add_glyph_typeface_from_stream(
                        &*collection,
                        &mut bytes.as_slice(),
                    )
                });
                if font.is_some_and(|f| {
                    f.family_name() == FAMILY || f.typographic_family_name() == FAMILY
                }) {
                    collection.state.set(LoadState::Ready);
                    if let Some(top) = top_level() {
                        for visual in top.get_visual_descendants() {
                            if let Some(text) = visual.cast::<TextBlock>() {
                                text.invalidate_text_layout();
                            } else if let Some(editor) = visual.cast::<TextPresenter>() {
                                editor.invalidate_text_layout();
                            }
                        }
                    }
                } else {
                    // Further typing can retry after a transient offline/fetch error.
                    collection
                        .state
                        .set(LoadState::RetryAfter(Instant::now() + RETRY_DELAY));
                }
            }),
        );
    }
}

impl FontCollectionBaseImpl for CjkCollection {
    fn base(this: &Self) -> &FontCollectionBase {
        &this.base
    }

    fn key(this: &Self) -> Uri {
        this.key.clone()
    }

    fn try_get_glyph_typeface(
        this: &Self,
        family: &str,
        style: FontStyle,
        weight: FontWeight,
        stretch: FontStretch,
    ) -> Option<Rc<GlyphTypeface>> {
        if family != FAMILY {
            return None;
        }
        this.request();
        if !matches!(this.state.get(), LoadState::Ready) {
            return None;
        }
        FontCollectionBase::try_get_glyph_typeface(this, family, style, weight, stretch)
    }
}

fn font_options() -> FontManagerOptions {
    FontManagerOptions {
        default_family_name: Some("fonts:Inter#Inter".into()),
        font_fallbacks: Some(vec![FontFallback {
            font_family: FontFamily::new(&format!("{COLLECTION}#{FAMILY}")),
            unicode_range: fallback_range(),
        }]),
        ..Default::default()
    }
}

pub(super) fn configure(builder: &AppBuilder, load: LoadFont) -> AppBuilder {
    builder
        .with(Rc::new(font_options()))
        .configure_fonts(move |manager| {
            let collection = CjkCollection::new(load);
            ACTIVE.with(|active| *active.borrow_mut() = Rc::downgrade(&collection));
            manager.add_font_collection(collection);
        })
}

pub(super) fn retry_after_online() {
    if let Some(collection) = ACTIVE.with(|active| active.borrow().upgrade()) {
        collection.retry_on_connectivity();
    }
}

pub(super) fn top_level() -> Option<Ref<TopLevel>> {
    Application::current()?
        .application_lifetime()?
        .as_single_top_level_application_lifetime()?
        .top_level()
}

/// Read shaped output only for an explicitly requested control in the browser UI probe.
pub(super) fn probe(visual: &Visual, element: &mut serde_json::Value) {
    use ferroui_base::media::text_formatting::{ShapedTextRun, TextRun};
    let layout = if let Some(editor) = visual.to_ref().cast::<TextBox>() {
        element["caretIndex"] = editor.caret_index().into();
        element["selectionStart"] = editor.selection_start().into();
        element["selectionEnd"] = editor.selection_end().into();
        element["focused"] = editor.is_focused().into();
        editor
            .get_visual_descendants()
            .find_map(|v| v.cast::<TextPresenter>())
            .map(|p| p.text_layout())
    } else {
        visual.to_ref().cast::<TextBlock>().map(|b| b.text_layout())
    };
    if let Some(layout) = layout {
        let mut runs = Vec::new();
        for line in layout.text_lines() {
            for run in line.text_runs().iter() {
                if let Some(run) = run.downcast_ref::<ShapedTextRun>() {
                    let buffer = run.shaped_buffer();
                    runs.push(serde_json::json!({
                        "text": String::from_utf16_lossy(run.text_span()),
                        "family": buffer.glyph_typeface().family_name(),
                        "glyphs": &*buffer.glyph_indices(),
                    }));
                }
            }
        }
        element["glyphRuns"] = runs.into();
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "emscripten"))]
    #[test]
    fn connectivity_retries_a_failed_face_only_once_while_offline() {
        use super::*;
        use ferroui_base::platform::IFontManagerImpl;
        use ferroui_base::FerroLocator;

        thread_local! { static ATTEMPTS: Cell<usize> = const { Cell::new(0) }; }
        let _dispatcher = Dispatcher::unit_test_scope();
        let scope = FerroLocator::enter_scope();
        FerroLocator::current_mutable()
            .bind::<dyn IFontManagerImpl>()
            .to_constant(Rc::new(ferroui_skia::FontManagerImpl::new()));
        let cjk = CjkCollection::new(|| {
            ATTEMPTS.with(|count| count.set(count.get() + 1));
            Box::pin(async { None })
        });
        cjk.retry_on_connectivity();
        Dispatcher::ui_thread().run_jobs(None);
        assert_eq!(ATTEMPTS.with(Cell::get), 0);
        cjk.request();
        Dispatcher::ui_thread().run_jobs(None);
        assert_eq!(ATTEMPTS.with(Cell::get), 1);
        cjk.request();
        cjk.retry_on_connectivity();
        cjk.retry_on_connectivity();
        Dispatcher::ui_thread().run_jobs(None);
        cjk.retry_on_connectivity();
        Dispatcher::ui_thread().run_jobs(None);
        assert_eq!(ATTEMPTS.with(Cell::get), 2);
        scope.dispose();
    }

    #[cfg(not(target_os = "emscripten"))]
    #[test]
    #[ignore = "uses the complete pinned OTF from EMBER_CJK_FONT"]
    fn one_deferred_load_supplies_real_han_kana_and_hangul_glyphs() {
        use super::*;
        use ferroui_base::media::{FontManager, Typeface};
        use ferroui_base::platform::{IAssetLoader, IFontManagerImpl, StandardAssetLoader};
        use ferroui_base::FerroLocator;
        use ferroui_fonts_inter::InterFontCollection;

        thread_local! { static LOADS: Cell<usize> = const { Cell::new(0) }; }
        let _dispatcher = Dispatcher::unit_test_scope();
        let scope = FerroLocator::enter_scope();
        let locator = FerroLocator::current_mutable();
        locator
            .bind::<dyn IFontManagerImpl>()
            .to_constant(Rc::new(ferroui_skia::FontManagerImpl::new()));
        locator
            .bind::<dyn IAssetLoader>()
            .to_constant(Rc::new(StandardAssetLoader::new(None)));
        locator
            .bind::<FontManagerOptions>()
            .to_constant(Rc::new(font_options()));
        let manager = FontManager::current();
        manager.add_font_collection(Rc::new(InterFontCollection::new()));
        let cjk = CjkCollection::new(|| {
            LOADS.with(|loads| loads.set(loads.get() + 1));
            Box::pin(async {
                Some(
                    std::fs::read(
                        std::env::var_os("EMBER_CJK_FONT").expect("path to the pinned font"),
                    )
                    .unwrap(),
                )
            })
        });
        manager.add_font_collection(cjk.clone());
        let inter = FontFamily::new("fonts:Inter#Inter");
        let original = manager
            .try_get_glyph_typeface(&Typeface::new(inter.clone()))
            .unwrap();
        assert!(
            original
                .character_to_glyph_map()
                .try_get_glyph('A' as i32)
                .unwrap()
                != 0
        );
        assert!(matches!(cjk.state.get(), LoadState::Idle));
        let match_face = |c: char| {
            manager.try_match_character(
                c as i32,
                FontStyle::Normal,
                FontWeight::Normal,
                FontStretch::Normal,
                Some(&inter),
                None,
            )
        };
        for c in ['界', 'あ', '한'] {
            assert_eq!(
                original
                    .character_to_glyph_map()
                    .try_get_glyph(c as i32)
                    .unwrap_or(0),
                0
            );
            match_face(c);
        }
        Dispatcher::ui_thread().run_jobs(None);
        assert_eq!(LOADS.with(Cell::get), 1);
        for c in ['界', 'あ', '한'] {
            let face = manager
                .try_get_glyph_typeface(&match_face(c).unwrap())
                .unwrap();
            assert_eq!(face.family_name(), FAMILY);
            assert_ne!(
                face.character_to_glyph_map()
                    .try_get_glyph(c as i32)
                    .unwrap_or(0),
                0
            );
        }
        scope.dispose();
    }

    #[test]
    fn only_cjk_ranges_are_eligible_for_download() {
        let range = super::fallback_range();
        for c in ['界', 'あ', 'カ', '한', '。', '𠮷'] {
            assert!(range.is_in_range(c as i32));
        }
        for c in ['A', 'ż', 'ش', 'א', '🔎'] {
            assert!(!range.is_in_range(c as i32));
        }
    }
}
