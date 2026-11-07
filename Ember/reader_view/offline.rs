//! Commands shared by the discussion toolbar, the library and local data settings.
use super::*;
use ferroui_controls::MenuFlyout;
use futures_util::future::{AbortHandle, Abortable, Aborted};

pub(super) struct Transfer {
    cancel: AbortHandle,
    pub(super) keep_bookmark_choice: bool,
}

impl ReaderView {
    pub(super) fn start_download(&self, id: u64) {
        if !self.state_ready.get() {
            self.status("Download is paused until reading data is restored. Import a backup in Settings first.");
            return;
        }
        if self.cache_work.get() != CacheWork::Idle {
            return;
        }
        let Some(repo) = self.repo() else { return };
        let checking = self.state.borrow().downloaded_threads.contains_key(&id);
        let mode = if checking {
            FetchMode::Offline
        } else {
            FetchMode::CacheFirst
        };
        let title = self
            .current_item()
            .filter(|item| item.id == id)
            .map(Rc::new)
            .or_else(|| {
                self.stories
                    .borrow()
                    .iter()
                    .find(|item| item.id == id)
                    .cloned()
            })
            .map(|item| {
                StorySummary::new(
                    &item,
                    services::now(),
                    self.state.borrow().preferences.show_dead,
                )
                .headline
            })
            .unwrap_or_else(|| format!("Discussion {id}"));
        let (cancel, registration) = AbortHandle::new_pair();
        self.transfer.replace(Some(Transfer {
            cancel,
            keep_bookmark_choice: false,
        }));
        self.set_cache_work(CacheWork::Downloading(id));
        self.label("TransferTitle").set_text(Some(&title));
        self.label("TransferCount").set_text(Some(if checking {
            "Checking local copy…"
        } else {
            "Preparing download…"
        }));
        self.button("CancelTransfer").set_is_enabled(true);
        self.get_control::<ferroui_controls::ProgressBar>("TransferProgress")
            .set_is_visible(false);
        self.get_control::<Border>("TransferBar")
            .set_is_visible(true);

        let generation = self.library_generation.get();
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let transfer = repo.download_thread_with_progress(
                    id,
                    10000,
                    services::now(),
                    mode,
                    |progress| {
                        if let Some(view) = weak.upgrade() {
                            view.show_transfer_progress(progress, checking);
                        }
                    },
                );
                let result = Abortable::new(transfer, registration).await;
                let Some(view) = weak.upgrade() else { return };
                view.finish_download(id, generation, checking, result);
            }),
        );
    }

    fn finish_download(
        &self,
        id: u64,
        generation: u64,
        checking: bool,
        result: Result<Result<ThreadSnapshot, ember_core::ReaderError>, Aborted>,
    ) {
        let Some(transfer) = self.transfer.borrow_mut().take() else {
            return;
        };
        let had_focus = self.button("CancelTransfer").is_focused();
        self.get_control::<Border>("TransferBar")
            .set_is_visible(false);
        self.set_cache_work(CacheWork::Idle);
        if had_focus {
            let toolbar = ["DownloadButton", "ToolbarMore", "BackButton"]
                .into_iter()
                .map(|name| self.button(name))
                .find(|button| button.is_effectively_visible() && button.is_enabled());
            if let Some(command) = toolbar {
                command.focus();
            } else if self.get_control::<ListBox>("Stories").items().count() > 0 {
                self.get_control::<ListBox>("Stories").focus();
            } else if self.button("RetryButton").is_effectively_visible() {
                self.button("RetryButton").focus();
            }
        }
        if generation != self.library_generation.get() {
            return;
        }
        let Ok(result) = result else {
            self.status(if checking {
                "Offline check canceled."
            } else {
                "Download canceled."
            });
            return;
        };
        let library_changed =
            if let Some(thread) = result.as_ref().ok().filter(|thread| thread.complete) {
                let mut state = self.state.borrow_mut();
                let first_download = state
                    .downloaded_threads
                    .insert(id, thread.items.keys().copied().collect())
                    .is_none();
                if first_download && !transfer.keep_bookmark_choice {
                    state
                        .items
                        .entry(id)
                        .or_default()
                        .saved_at
                        .get_or_insert_with(services::now);
                }
                true
            } else {
                self.state
                    .borrow_mut()
                    .downloaded_threads
                    .remove(&id)
                    .is_some()
            };
        if library_changed {
            self.changed();
            self.update_thread_commands();
            if *self.feed.borrow() == FeedRoute::Saved {
                self.load_feed(0, FetchMode::CacheFirst);
            }
        }
        match result {
            Ok(thread) => {
                let message = if !thread.complete {
                    format!("Download incomplete. {} items missing or pending. Retry Download to continue.",
                            thread.missing_ids.len() + thread.pending_ids.len())
                } else if checking {
                    "Offline copy checked. All items are available on this device.".to_owned()
                } else if !self.state.borrow().is_saved(id) {
                    "Discussion downloaded. It is available in Downloads.".to_owned()
                } else {
                    "Discussion downloaded. It is available offline in Saved stories.".to_owned()
                };
                if self.selected.get() == Some(id)
                    && matches!(*self.detail.borrow(), DetailRoute::Thread(_))
                    && !checking
                {
                    self.accept_thread(thread);
                }
                self.status(&message);
            }
            Err(error) => self.status(&format!("Offline copy unavailable: {error}")),
        }
        self.prune_cache();
    }

    pub(super) fn cancel_transfer(&self) {
        if let Some(transfer) = self.transfer.borrow().as_ref() {
            transfer.cancel.abort();
        }
    }

    fn show_transfer_progress(&self, progress: ember_core::DownloadProgress, checking: bool) {
        use ember_core::DownloadProgress;
        let bar = self.get_control::<ferroui_controls::ProgressBar>("TransferProgress");
        let message = match progress {
            DownloadProgress::Collecting { loaded, pending } => {
                let action = if checking { "Checking" } else { "Downloading" };
                let unit = if loaded == 1 { "item" } else { "items" };
                format!("{action} · {loaded} {unit} loaded · {pending} pending")
            }
            DownloadProgress::Verifying { checked, total } => {
                bar.set_maximum(total as f64);
                bar.set_range_value(checked as f64);
                bar.set_is_visible(true);
                format!("Verifying offline copy · {checked} of {total}")
            }
        };
        self.label("TransferCount").set_text(Some(&message));
    }

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
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let Some(view) = weak.upgrade() else { return };
                let clean = repo.clean_temporary_cache(&view.state.borrow());
                drop(view);
                let result = clean.await;
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
