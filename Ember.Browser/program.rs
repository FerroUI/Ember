//! The reader is a FerroUI control tree rendered to the browser canvas.
//! JavaScript provides only the platform's network, database and loader services.

#![cfg_attr(target_os = "emscripten", no_main)]

#[cfg(target_os = "emscripten")]
mod platform;

#[cfg(target_os = "emscripten")]
use wasm_bindgen::prelude::*;

#[cfg(target_os = "emscripten")]
#[wasm_bindgen(js_name = runMain)]
pub fn run_main(href: &str) {
    use ferroui_base::logging::LogEventLevel;
    use ferroui_base::media::FontManagerOptions;
    use ferroui_browser::{
        BrowserAppBuilder, BrowserPlatformOptions, BrowserRenderer, BrowserRenderingMode,
    };
    use ferroui_controls::AppBuilder;
    use ferroui_fonts_inter::AppBuilderExtension;
    use std::rc::Rc;

    ember::set_repository_factory(Rc::new(|| {
        Rc::new(ember_core::Repository::new(
            Rc::new(platform::BrowserHttpClient),
            Rc::new(platform::IndexedDbStore),
        ))
    }));
    ember::set_initial_location(href.to_owned());
    ember::set_location_changed(Rc::new(|item| {
        platform::set_location(item.map(|id| id.to_string()))
    }));
    let mut options = BrowserPlatformOptions::default();
    if href.contains("RenderingMode=Software2D") {
        options.rendering_mode = vec![BrowserRenderingMode::Software2D];
    }
    if href.contains("Renderer=Vello") && BrowserRenderer::Vello.is_available() {
        options.renderer = BrowserRenderer::Vello;
    }
    if href.contains("RenderThread=false") {
        options.render_thread = false;
    }
    AppBuilder::configure::<ember::App>()
        .with_inter_font()
        .with(Rc::new(FontManagerOptions {
            default_family_name: Some("fonts:Inter#Inter".to_owned()),
            ..Default::default()
        }))
        .log_to_text_writer(std::io::stdout(), LogEventLevel::Warning, &[])
        .start_browser_app("out", Some(options));
}

#[cfg(target_os = "emscripten")]
#[wasm_bindgen(js_name = emberNavigate)]
pub fn navigate(href: &str) {
    ember::navigate_location(href);
}

/// Read-only visual-tree probe used by browser interaction tests. Coordinates
/// are CSS pixels of the actual FerroUI view, including touch hit testing.
#[cfg(target_os = "emscripten")]
#[wasm_bindgen(js_name = emberState)]
pub fn reader_state() -> String {
    use ferroui_base::{Point, Visual};
    use ferroui_controls::automation::AutomationProperties;
    use ferroui_controls::{Application, Button, ComboBox, TextBlock, TextBox};

    let top_level = Application::current()
        .and_then(|app| app.application_lifetime())
        .and_then(|lifetime| {
            lifetime
                .as_single_top_level_application_lifetime()
                .and_then(|view| view.top_level())
        });
    let Some(top_level) = top_level else {
        return "null".to_owned();
    };
    let viewport = top_level.bounds();
    let mut elements = Vec::new();
    for visual in top_level.get_visual_descendants() {
        let text = if let Some(text) = visual.cast::<TextBlock>() {
            text.text()
        } else if let Some(text) = visual.cast::<TextBox>() {
            text.text()
        } else if visual.cast::<Button>().is_some() || visual.cast::<ComboBox>().is_some() {
            None
        } else {
            continue;
        };
        let size = visual.bounds();
        if !visual.is_effectively_visible() || size.width <= 0.0 || size.height <= 0.0 {
            continue;
        }
        let Some(origin) = visual.translate_point(Point::new(0.0, 0.0), &top_level) else {
            continue;
        };
        if origin.x >= viewport.width
            || origin.y >= viewport.height
            || origin.x + size.width <= 0.0
            || origin.y + size.height <= 0.0
        {
            continue;
        }
        let center = Point::new(origin.x + size.width / 2.0, origin.y + size.height / 2.0);
        let hit = top_level
            .input_hit_test_with(center, false)
            .is_some_and(|hit| {
                let hit: &Visual = &hit;
                std::ptr::eq(hit, &*visual)
                    || hit.is_visual_ancestor_of(&visual)
                    || visual.is_visual_ancestor_of(hit)
            });
        elements.push(serde_json::json!({
            "type": visual.get_type().name(), "name": visual.name(), "text": text, "hit": hit,
            "automationName": AutomationProperties::get_name(&visual),
            "selectedIndex": visual.cast::<ComboBox>().map(|combo| combo.selected_index()),
            "x": origin.x, "y": origin.y, "width": size.width, "height": size.height,
        }));
    }
    serde_json::json!({ "width": viewport.width, "height": viewport.height, "elements": elements })
        .to_string()
}

#[cfg(not(target_os = "emscripten"))]
fn main() {
    println!("Build Ember for the browser with scripts/build-browser.sh");
}
