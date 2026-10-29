//! Android library loaded by the platform's FerroApplication Java host.

#[cfg(target_os = "android")]
ferroui_android::android_application!(application::build);

#[cfg(target_os = "android")]
mod application {
    use ferroui_android::interop::java::{call_object, string_of};
    use ferroui_android::log::{self, LogPriority};
    use ferroui_android::{AndroidApplicationExtensions, FerroAndroidApplication};
    use ferroui_base::logging::LogEventLevel;
    use ferroui_controls::AppBuilder;
    use ferroui_fonts_inter::AppBuilderExtension;

    pub fn build() -> AppBuilder {
        // Application setup precedes Activity.onResume; there is no current
        // activity yet. Use the existing application context for private storage.
        if let Some(directory) = files_directory() {
            ember::set_database_path(
                std::path::PathBuf::from(directory).join("Ember/reader.sqlite3"),
            );
        }
        AppBuilder::configure::<ember::App>()
            .with_inter_font()
            .use_android()
            .log_to_delegate(
                |line| log::write(LogPriority::Warn, "ferroui-ember", line),
                LogEventLevel::Warning,
                &[],
            )
    }

    fn files_directory() -> Option<String> {
        let context = FerroAndroidApplication::try_context()?;
        let directory = call_object(&context, "getFilesDir", "()Ljava/io/File;", &[])?;
        let path = call_object(&directory, "getAbsolutePath", "()Ljava/lang/String;", &[])?;
        Some(string_of(&path))
    }
}
