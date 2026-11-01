use super::*;

impl ReaderView {
    pub(super) fn show_more(&self) {
        self.navigate(DetailRoute::More);
        self.label("DetailEyebrow").set_text(Some("NAVIGATION"));
        self.label("DetailTitle").set_text(Some("Explore Ember"));
        let panel = widgets::stack(3.0);
        panel.set_max_width(620.0);
        panel.set_margin(Thickness::uniform(15.0));
        panel.children().add(widgets::muted("FEEDS"));
        for (label, icon, kind) in [
            ("Best stories", "star", FeedKind::Best),
            ("Ask HN", "chat", FeedKind::Ask),
            ("Show HN", "lightbulb", FeedKind::Show),
            ("Jobs", "briefcase", FeedKind::Jobs),
        ] {
            let weak = self.to_ref().downgrade();
            panel.children().add(
                widgets::action(label, icon, move || {
                    if let Some(v) = weak.upgrade() {
                        v.select_feed(FeedRoute::Feed(kind));
                    }
                })
                .upcast::<Control>(),
            );
        }
        panel.children().add(widgets::muted("LIBRARY"));
        for (label, icon, route) in [
            ("Reading history", "history", FeedRoute::History),
            ("Hidden stories", "eye_off", FeedRoute::Hidden),
        ] {
            let weak = self.to_ref().downgrade();
            panel.children().add(
                widgets::action(label, icon, move || {
                    if let Some(v) = weak.upgrade() {
                        v.select_feed(route.clone());
                    }
                })
                .upcast::<Control>(),
            );
        }
        let weak = self.to_ref().downgrade();
        panel.children().add(
            widgets::action("Appearance and settings", "settings", move || {
                if let Some(v) = weak.upgrade() {
                    v.show_settings();
                }
            })
            .upcast::<Control>(),
        );
        panel.children().add(widgets::muted("HACKER NEWS"));
        for (label, url) in [
            ("Sign in on Hacker News", ember_core::login_url()),
            ("Submit on Hacker News", ember_core::submit_url()),
        ] {
            let weak = self.to_ref().downgrade();
            panel.children().add(
                widgets::action(label, "open", move || {
                    if let Some(v) = weak.upgrade() {
                        v.open_url(url);
                    }
                })
                .upcast::<Control>(),
            );
        }
        panel.children().add(widgets::text("Voting, replying and submitting open Hacker News in your browser. Ember's saved stories and reading history stay on this device.",13.0).upcast::<Control>());
        self.show_content(widgets::scroll(panel.upcast::<Control>()));
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
