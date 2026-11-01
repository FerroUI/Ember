use super::*;
use ferroui_base::layout::{HorizontalAlignment, VerticalAlignment};
use ferroui_controls::primitives::UniformGrid;

fn page() -> Ref<StackPanel> {
    let panel = widgets::stack(10.0);
    panel.set_max_width(680.0);
    panel.set_margin(Thickness::uniform(14.0));
    panel
}

impl ReaderView {
    fn discovery_action(
        &self,
        label: &str,
        icon: &str,
        invoke: impl Fn(&ReaderView) + 'static,
    ) -> Ref<Button> {
        let weak = self.to_ref().downgrade();
        widgets::action(label, icon, move || {
            if let Some(view) = weak.upgrade() {
                invoke(&view);
            }
        })
    }

    pub(super) fn show_more(&self) {
        self.navigate(DetailRoute::More);
        self.label("DetailEyebrow").set_text(Some("NAVIGATION"));
        self.label("DetailTitle").set_text(Some("Explore Ember"));
        let panel = page();
        let feeds = widgets::section(&panel, "Browse feeds");
        let links = UniformGrid::new();
        links.set_columns(2);
        links.set_column_spacing(4.0);
        for (label, icon, kind) in [
            ("Best stories", "star", FeedKind::Best),
            ("Ask HN", "chat", FeedKind::Ask),
            ("Show HN", "lightbulb", FeedKind::Show),
            ("Jobs", "briefcase", FeedKind::Jobs),
        ] {
            let button = self.discovery_action(label, icon, move |v| {
                v.select_feed(FeedRoute::Feed(kind));
            });
            button.set_horizontal_alignment(HorizontalAlignment::Stretch);
            links.children().add(button);
        }
        feeds.children().add(links);
        let library = widgets::section(&panel, "Your library");
        let state = self.state.borrow();
        let downloads = state
            .downloaded_threads
            .keys()
            .filter(|&&id| state.is_saved(id))
            .count();
        let history = state
            .items
            .values()
            .filter(|entry| entry.last_opened_at.is_some())
            .count();
        let hidden = state.items.values().filter(|entry| entry.hidden).count();
        drop(state);
        for (title, icon, count, route) in [
            (
                "Downloaded discussions",
                "download",
                downloads,
                FeedRoute::Saved,
            ),
            ("Reading history", "history", history, FeedRoute::History),
            ("Hidden stories", "eye_off", hidden, FeedRoute::Hidden),
        ] {
            let button = self.discovery_action(title, icon, move |v| {
                if route == FeedRoute::Saved {
                    if *v.feed.borrow() == FeedRoute::Saved {
                        v.back_to_feed(true);
                        if !v.downloaded_only.get() {
                            v.set_downloaded_only(true);
                        }
                    } else {
                        v.downloaded_only.set(true);
                        v.select_feed(FeedRoute::Saved);
                    }
                } else {
                    v.select_feed(route.clone());
                }
            });
            let row = Grid::new();
            row.set_column_definitions(
                ColumnDefinitions::parse("*,Auto").expect("library columns"),
            );
            row.children().add(widgets::nav_content(icon, title, false));
            if self.state_ready.get() {
                let total = widgets::muted(&count.to_string());
                total.set_vertical_alignment(VerticalAlignment::Center);
                Grid::set_column(&total, 1);
                row.children().add(total);
                AutomationProperties::set_item_status(&button, Some(&format!("{count} items")));
            }
            button.set_content(Some(Control::boxed(&row)));
            button.set_horizontal_alignment(HorizontalAlignment::Stretch);
            button.set_horizontal_content_alignment(HorizontalAlignment::Stretch);
            library.children().add(button);
        }
        let tools = widgets::section(&panel, "Reader tools");
        tools.children().add(
            self.discovery_action("Appearance and settings", "settings", |v| v.show_settings()),
        );
        let keys = widgets::disclosure(&tools, "Keyboard shortcuts", false);
        for (action, shortcut) in [
            ("Search", "Ctrl / ⌘ + K"),
            ("Find in discussion", "Ctrl / ⌘ + F"),
            ("Next / previous story", "J / K"),
            ("Save story", "S"),
            ("Open article", "O"),
            ("Focus reading", "F"),
            ("Refresh", "R"),
            ("Go back", "Esc"),
        ] {
            let row = Grid::new();
            row.set_column_definitions(
                ColumnDefinitions::parse("*,Auto").expect("shortcut columns"),
            );
            row.children().add(widgets::label(action, 13.0, false));
            let key = widgets::muted(shortcut);
            Grid::set_column(&key, 1);
            row.children().add(key);
            keys.children().add(row);
        }
        let account = widgets::disclosure(&panel, "Hacker News account", false);
        for (label, url) in [
            ("Sign in on Hacker News", ember_core::login_url()),
            ("Submit on Hacker News", ember_core::submit_url()),
        ] {
            account
                .children()
                .add(self.discovery_action(label, "open", move |v| v.open_url(url)));
        }
        self.show_content(widgets::scroll(panel.upcast()));
    }

    pub(super) fn show_profile(&self, name: String) {
        self.navigate(DetailRoute::Profile(name.clone()));
        self.label("DetailEyebrow").set_text(Some("PROFILE"));
        self.label("DetailTitle").set_text(Some(&name));
        let loading = widgets::muted("Loading profile…");
        loading.set_margin(Thickness::uniform(18.0));
        self.show_content(loading.upcast());
        let Some(repo) = self.repo() else {
            return;
        };
        let generation = self.detail_generation.get();
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let result = repo
                    .load_user(&name, services::now(), FetchMode::CacheFirst)
                    .await;
                if let Some(v) = weak.upgrade() {
                    if v.detail_generation.get() != generation {
                        return;
                    }
                    match result {
                        Ok(loaded) => {
                            if let Some(warning) = loaded.warning.as_deref() {
                                v.status(warning);
                            }
                            if let Some(user) = loaded.value {
                                let panel = widgets::stack(10.0);
                                panel.set_margin(Thickness::uniform(18.0));
                                panel.set_max_width(660.0);
                                panel
                                    .children()
                                    .add(widgets::heading(&user.id).upcast::<Control>());
                                let mut facts = Vec::new();
                                if let Some(karma) = user.karma {
                                    facts.push(format!("{karma} karma"));
                                }
                                if user.created.is_some() {
                                    facts.push(format!(
                                        "Joined {}",
                                        ember_core::relative_time(user.created, services::now())
                                    ));
                                }
                                let count = user.submitted.len();
                                facts.push(format!(
                                    "{count} {}",
                                    if count == 1 {
                                        "submission"
                                    } else {
                                        "submissions"
                                    }
                                ));
                                panel.children().add(widgets::muted(&facts.join(" · ")));
                                let link = v.to_ref().downgrade();
                                let options = crate::thread_view::render_options(&v.state.borrow());
                                panel.children().add(ember_content::render(
                                    &ember_content::HtmlDocument::parse(
                                        user.about.as_deref().unwrap_or("No public biography."),
                                    ),
                                    &options,
                                    Rc::new(move |url| {
                                        if let Some(v) = link.upgrade() {
                                            v.open_url(url);
                                        }
                                    }),
                                ));
                                for (label, icon, kind) in [
                                    ("Stories", "home", SearchKind::Stories),
                                    ("Comments", "chat", SearchKind::Comments),
                                ] {
                                    let username = user.id.clone();
                                    let target = v.to_ref().downgrade();
                                    panel.children().add(
                                        widgets::action(label, icon, move || {
                                            if let Some(v) = target.upgrade() {
                                                v.run_search(SearchQuery {
                                                    author: Some(username.clone()),
                                                    kind,
                                                    ..Default::default()
                                                });
                                            }
                                        })
                                        .upcast::<Control>(),
                                    );
                                }
                                let link = v.to_ref().downgrade();
                                let url = ember_core::user_url(&user.id);
                                panel.children().add(
                                    widgets::action("Profile on Hacker News", "open", move || {
                                        if let Some(v) = link.upgrade() {
                                            v.open_url(&url);
                                        }
                                    })
                                    .upcast::<Control>(),
                                );
                                v.show_content(widgets::scroll(panel.upcast::<Control>()));
                            } else {
                                let message = widgets::muted("This user has no public profile.");
                                message.set_margin(Thickness::uniform(18.0));
                                v.show_content(message.upcast());
                            }
                        }
                        Err(error) => {
                            v.status(&format!("Profile unavailable: {error}"));
                            let panel = widgets::stack(8.0);
                            panel.set_margin(Thickness::uniform(18.0));
                            panel
                                .children()
                                .add(widgets::heading("Couldn’t load profile"));
                            panel.children().add(widgets::muted(
                                "Check the username or your connection, then try again.",
                            ));
                            let target = v.to_ref().downgrade();
                            let retry = widgets::action("Try again", "refresh", move || {
                                if let Some(view) = target.upgrade() {
                                    view.show_profile(name.clone());
                                }
                            });
                            retry.set_horizontal_alignment(
                                ferroui_base::layout::HorizontalAlignment::Left,
                            );
                            panel.children().add(retry);
                            v.show_content(panel.upcast());
                        }
                    }
                }
            }),
        );
    }
}
