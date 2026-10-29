//! Desktop entry point: `cargo run -p ember-desktop -- https://news.ycombinator.com/item?id=8863`.

use ferroui_base::logging::LogEventLevel;
use ferroui_controls::AppBuilder;
use ferroui_desktop::AppBuilderDesktopExtensions;
use ferroui_fonts_inter::AppBuilderExtension;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(location) = args.iter().find(|value| !value.starts_with('-')) {
        ember::set_initial_location(location.clone());
    }
    let code = AppBuilder::configure::<ember::App>()
        .with_inter_font()
        .use_platform_detect()
        .after_setup(|_| sample_support::smoke_run())
        .log_to_trace(LogEventLevel::Warning, &[])
        .start_with_classic_desktop_lifetime(&args);
    std::process::ExitCode::from(code as u8)
}
