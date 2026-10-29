//! The iOS host of Ember (UIKit, on macOS with Xcode). FerroUI's `scripts/ios/bundle.sh`
//! packages applications of the FerroUI workspace.

#[cfg(not(target_os = "ios"))]
fn main() {
    eprintln!("ember-ios requires the iOS target and UIKit; use ember-desktop on this platform");
}

#[cfg(target_os = "ios")]
fn main() {
    // HOME is the application's sandbox, not a shared system directory.
    if let Some(home) = std::env::var_os("HOME") {
        ember::set_database_path(
            std::path::PathBuf::from(home).join("Library/Application Support/Ember/reader.sqlite3"),
        );
    }
    ferroui_ios::run_application_with(std::rc::Rc::new(host::AppDelegate))
}

#[cfg(target_os = "ios")]
mod host {
    use ferroui_controls::AppBuilder;
    use ferroui_fonts_inter::AppBuilderExtension;
    use ferroui_ios::{FerroApplicationDelegate, IFerroAppDelegate, IosApplicationExtensions};
    use std::rc::Rc;

    pub struct AppDelegate;

    impl FerroApplicationDelegate for AppDelegate {
        fn create_app_builder(&self, delegate: Rc<dyn IFerroAppDelegate>) -> AppBuilder {
            AppBuilder::configure::<ember::App>()
                .with_inter_font()
                .use_ios_with_delegate(delegate)
        }
    }
}
