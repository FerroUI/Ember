//! Commands shared by the discussion toolbar, the library and local data settings.
use super::*;
use ferroui_controls::MenuFlyout;

impl ReaderView {
    pub(super) fn offline_actions(&self, id: u64) -> Ref<MenuFlyout> {
        let menu = MenuFlyout::new();
        for (label, icon, release) in [
            ("Check offline copy", "refresh", false),
            ("Stop keeping offline", "download", true),
        ] {
            let item = MenuItem::new();
            item.set_header(Some(Rc::new(label.to_owned())));
            item.set_icon(Some(Control::boxed(crate::icons::icon(icon))));
            let weak = self.to_ref().downgrade();
            item.click(move |_, _| {
                if let Some(view) = weak.upgrade().filter(|v| v.selected.get() == Some(id)) {
                    if release {
                        view.release_download(id);
                    } else {
                        view.download();
                    }
                }
            });
            menu.items().add(Some(Control::boxed(&item)));
        }
        menu
    }

    pub(super) fn release_download(&self, id: u64) {
        if self.cache_work.get() != CacheWork::Idle || !self.state_ready.get() {
            self.status("Offline storage is busy. Try again when the current operation finishes.");
            return;
        }
        if self
            .state
            .borrow_mut()
            .downloaded_threads
            .remove(&id)
            .is_none()
        {
            return;
        }
        self.changed();
        self.update_thread_commands();
        if self.downloaded_shelf() {
            self.load_feed(0, FetchMode::Offline);
        }
        self.render_stories();
        self.status("Removed from Downloads. Temporary pages can be reclaimed in Settings.");
    }

    pub(super) fn measure_storage(&self, report: Rc<dyn Fn(&str)>) {
        let Some(repo) = self.repo() else {
            report("Storage is unavailable.");
            return;
        };
        self.storage_report.replace(Some(report.clone()));
        let request = self.storage_request.get().wrapping_add(1);
        self.storage_request.set(request);
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let result = repo.cache_stats().await;
                let Some(view) = weak.upgrade().filter(|v| {
                    v.storage_request.get() == request && v.storage_report.borrow().is_some()
                }) else {
                    return;
                };
                match result {
                    Ok(stats) => {
                        let downloads = view.state.borrow().downloaded_threads.len();
                        let unit = if downloads == 1 {
                            "download"
                        } else {
                            "downloads"
                        };
                        let item = if stats.items == 1 { "item" } else { "items" };
                        report(&format!(
                            "{} of cached data · {} {item} · {downloads} {unit}",
                            storage_size(stats.bytes),
                            stats.items
                        ));
                    }
                    Err(error) => report(&format!("Could not measure storage. {error}")),
                }
            }),
        );
    }

    pub(super) fn refresh_storage_report(&self) {
        let report = self.storage_report.borrow().clone();
        if let Some(report) = report {
            self.measure_storage(report);
        }
    }

    pub(super) fn clean_temporary_cache(&self) {
        if !self.state_ready.get() || self.cache_work.get() != CacheWork::Idle {
            self.status("Offline storage is busy. Try cleaning it again when the current operation finishes.");
            return;
        }
        let Some(repo) = self.repo() else {
            return;
        };
        self.set_cache_work(CacheWork::Cleaning);
        let state = self.state.borrow().clone();
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let result = repo.clean_temporary_cache(&state).await;
                if let Some(view) = weak.upgrade() {
                    view.forget_feed_windows();
                    view.set_cache_work(CacheWork::Idle);
                    view.refresh_storage_report();
                    match result {
                        Ok(stats) if stats.records == 0 => {
                            view.status("Temporary cache is already clean.")
                        }
                        Ok(stats) => view.status(&format!(
                            "Cleared {} of temporary data. Bookmarks and downloads are kept.",
                            storage_size(stats.bytes)
                        )),
                        Err(error) => {
                            view.status(&format!("Temporary cleanup did not finish. {error}"))
                        }
                    }
                }
            }),
        );
    }
}

fn storage_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
