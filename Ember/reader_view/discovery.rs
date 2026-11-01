use super::*;
use ferroui_base::layout::{HorizontalAlignment, VerticalAlignment};
use ferroui_controls::primitives::UniformGrid;

pub(super) struct ExploreCount {
    route: FeedRoute,
    button: Ref<Button>,
    value: Ref<TextBlock>,
}

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
        let mut counters = Vec::new();
        for (title, icon, route) in [
            ("Downloaded discussions", "download", FeedRoute::Saved),
            ("Reading history", "history", FeedRoute::History),
            ("Hidden stories", "eye_off", FeedRoute::Hidden),
        ] {
            let target = route.clone();
            let button = self.discovery_action(title, icon, move |v| {
                if target == FeedRoute::Saved {
                    if *v.feed.borrow() == FeedRoute::Saved {
                        let was_searching = !v.library_find.borrow().query.is_empty();
                        v.reset_library_find();
                        v.back_to_feed(true);
                        if was_searching || !v.downloaded_only.get() {
                            v.set_downloaded_only(true);
                        }
                    } else {
                        v.downloaded_only.set(true);
                        v.select_feed(FeedRoute::Saved);
                    }
                } else {
                    v.select_feed(target.clone());
                }
            });
            let row = Grid::new();
            row.set_column_definitions(
                ColumnDefinitions::parse("*,Auto").expect("library columns"),
            );
            row.children().add(widgets::nav_content(icon, title, false));
            let value = widgets::muted("");
            value.set_vertical_alignment(VerticalAlignment::Center);
            Grid::set_column(&value, 1);
            row.children().add(value.clone());
            counters.push(ExploreCount {
                route,
                button: button.clone(),
                value,
            });
            button.set_content(Some(Control::boxed(&row)));
            button.set_horizontal_alignment(HorizontalAlignment::Stretch);
            button.set_horizontal_content_alignment(HorizontalAlignment::Stretch);
            library.children().add(button);
        }
        self.explore_counts.replace(counters);
        self.update_explore_counts();
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

    pub(super) fn update_explore_counts(&self) {
        let counters = self.explore_counts.borrow();
        if counters.is_empty() || !self.state_ready.get() {
            return;
        }
        let state = self.state.borrow();
        for counter in counters.iter() {
            let count = match counter.route {
                FeedRoute::Saved => state
                    .downloaded_threads
                    .keys()
                    .filter(|&&id| state.is_saved(id))
                    .count(),
                FeedRoute::History => state
                    .items
                    .values()
                    .filter(|entry| entry.last_opened_at.is_some())
                    .count(),
                FeedRoute::Hidden => state.items.values().filter(|entry| entry.hidden).count(),
                _ => continue,
            };
            counter.value.set_text(Some(&count.to_string()));
            let noun = if count == 1 { "item" } else { "items" };
            AutomationProperties::set_item_status(
                &counter.button,
                Some(&format!("{count} {noun}")),
            );
        }
    }

    pub(super) fn show_profile(&self, name: String) {
        self.fetch_profile(name, FetchMode::CacheFirst);
    }

    fn fetch_profile(&self, name: String, mode: FetchMode) {
        self.navigate(DetailRoute::Profile(name.clone()));
        self.label("DetailEyebrow").set_text(Some("PROFILE"));
        self.label("DetailTitle").set_text(Some(&name));
        let loading = page();
        loading.children().add(widgets::muted("Loading profile…"));
        self.show_content(loading.upcast());
        let Some(repo) = self.repo() else {
            self.profile_unavailable(&name, "The reading database is unavailable.");
            return;
        };
        let generation = self.detail_generation.get();
        let weak = self.to_ref().downgrade();
        drop(
            Dispatcher::ui_thread().invoke_async_task_local(move || async move {
                let result = repo.load_user(&name, services::now(), mode).await;
                let Some(view) = weak
                    .upgrade()
                    .filter(|v| v.detail_generation.get() == generation)
                else {
                    return;
                };
                let user = match result {
                    Ok(loaded) => {
                        if let Some(warning) = loaded.warning.as_deref() {
                            view.status(warning);
                        }
                        let Some(user) = loaded.value else {
                            view.profile_unavailable(&name, "This user has no public profile.");
                            return;
                        };
                        user
                    }
                    Err(error) => {
                        view.profile_unavailable(&name, &error.to_string());
                        return;
                    }
                };
                let activity = view.render_profile(&user);
                if user.submitted.is_empty() {
                    return;
                }
                drop(view);
                // The public user record can contain many thousands of submission IDs.
                // Render the bio immediately and fetch just the visible preview, in HN order.
                let ids = &user.submitted[..user.submitted.len().min(5)];
                let batch = repo.load_items(ids, services::now(), mode).await;
                if let Some(view) = weak
                    .upgrade()
                    .filter(|v| v.detail_generation.get() == generation)
                {
                    view.render_profile_activity(&activity, &user.id, batch);
                }
            }),
        );
    }

    fn profile_unavailable(&self, name: &str, reason: &str) {
        let panel = page();
        panel
            .children()
            .add(widgets::heading("Profile unavailable"));
        panel.children().add(widgets::label(reason, 15.0, true));
        let actions = ferroui_controls::WrapPanel::new();
        let lookup = name.to_owned();
        actions
            .children()
            .add(self.discovery_action("Edit username", "search", move |v| {
                v.show_profile_lookup(&lookup)
            }));
        let retry = name.to_owned();
        actions
            .children()
            .add(self.discovery_action("Try again", "refresh", move |v| {
                v.fetch_profile(retry.clone(), FetchMode::Refresh)
            }));
        panel.children().add(actions);
        self.show_content(widgets::scroll(panel.upcast()));
    }

    fn render_profile(&self, user: &ember_core::HnUser) -> Ref<StackPanel> {
        let panel = page();
        let bio = widgets::section(&panel, &user.id);
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
        bio.children().add(widgets::muted(&facts.join("  ·  ")));
        if let Some(about) = user
            .about
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            let link = self.to_ref().downgrade();
            bio.children().add(ember_content::render(
                &ember_content::HtmlDocument::parse(about),
                &crate::thread_view::render_options(&self.state.borrow()),
                Rc::new(move |url| {
                    if let Some(v) = link.upgrade() {
                        v.open_url(url);
                    }
                }),
            ));
        }
        let actions = ferroui_controls::WrapPanel::new();
        let url = ember_core::user_url(&user.id);
        let link = url.clone();
        actions.children().add(
            self.discovery_action("Profile on Hacker News", "open", move |v| v.open_url(&link)),
        );
        let copy = widgets::icon_button("Copy profile link", "copy");
        let weak = self.to_ref().downgrade();
        copy.click(move |_, _| {
            if let Some(v) = weak.upgrade() {
                v.copy_text(&url, "Profile link copied.");
            }
        });
        actions.children().add(copy);
        let mute = ToggleButton::new();
        mute.classes().add("feed-filter");
        mute.set_content(Some(Control::boxed(&widgets::nav_content(
            "eye_off",
            "Mute in feeds",
            false,
        ))));
        AutomationProperties::set_name(&mute, Some(&format!("Mute {} in feeds", user.id)));
        AutomationProperties::set_help_text(
            &mute,
            Some("Hide this author's stories from feeds and search results."),
        );
        let weak = self.to_ref().downgrade();
        let author = user.id.clone();
        mute.click(move |_, _| {
            let Some(view) = weak.upgrade().filter(|view| view.state_ready.get()) else {
                return;
            };
            let muted = {
                let mut state = view.state.borrow_mut();
                if state.mutes.users.remove(&author) {
                    false
                } else {
                    state.mutes.users.insert(author.clone());
                    true
                }
            };
            view.changed();
            view.update_profile_mute();
            view.render_stories();
            view.status(&format!(
                "{} {} in feeds.",
                author,
                if muted { "muted" } else { "unmuted" }
            ));
        });
        self.profile_mute.replace(Some(mute.clone()));
        self.update_profile_mute();
        actions.children().add(mute);
        bio.children().add(actions);

        let activity = widgets::section(&panel, "Recent activity");
        let list = widgets::stack(0.0);
        list.children().add(widgets::muted(if count == 0 {
            "No public submissions yet."
        } else {
            "Loading recent submissions…"
        }));
        activity.children().add(list.clone());
        if count > 0 {
            let routes = ferroui_controls::WrapPanel::new();
            for (label, icon, kind) in [
                ("All stories", "home", SearchKind::Stories),
                ("All comments", "chat", SearchKind::Comments),
            ] {
                let author = user.id.clone();
                routes
                    .children()
                    .add(self.discovery_action(label, icon, move |v| {
                        v.run_search(SearchQuery {
                            author: Some(author.clone()),
                            kind,
                            sort: ember_core::SearchSort::Recent,
                            ..Default::default()
                        });
                    }));
            }
            activity.children().add(routes);
        }
        self.show_content(widgets::scroll(panel.upcast()));
        list
    }

    pub(super) fn update_profile_mute(&self) {
        let mute = self.profile_mute.borrow();
        let route = self.detail.borrow();
        if let (Some(button), DetailRoute::Profile(author)) = (mute.as_ref(), &*route) {
            button.set_is_enabled(self.state_ready.get());
            let muted = self.state.borrow().mutes.users.contains(author);
            button.set_is_checked(Some(muted));
            let hint = if muted {
                "Show this author in feeds"
            } else {
                "Hide this author in feeds"
            };
            ToolTip::set_tip(button, Some(Rc::new(hint.to_owned()) as BoxedValue));
        }
    }

    fn render_profile_activity(&self, list: &StackPanel, user: &str, batch: ember_core::ItemBatch) {
        list.children().clear();
        let state = self.state.borrow();
        let show_dead = state.preferences.show_dead;
        for item in batch.items.iter().filter(|item| {
            item.by.as_deref() == Some(user)
                && !item.deleted
                && (!item.dead || show_dead)
                && !state.items.get(&item.id).is_some_and(|local| local.hidden)
        }) {
            let card = StoryCard::new();
            card.classes().add("compact");
            card.set_item(item, services::now(), show_dead);
            let weak = self.to_ref().downgrade();
            let id = item.id;
            card.click(move |_, _| {
                if let Some(view) = weak.upgrade() {
                    view.open_item_internal(id, true);
                }
            });
            list.children().add(card);
        }
        if batch.warning.is_some() && !batch.missing_ids.is_empty() {
            list.children().add(widgets::muted(
                "Some recent submissions could not be loaded.",
            ));
            let author = user.to_owned();
            list.children().add(
                self.discovery_action("Retry activity", "refresh", move |v| {
                    v.fetch_profile(author.clone(), FetchMode::Refresh)
                }),
            );
        } else if list.children().count() == 0 {
            list.children()
                .add(widgets::muted("No recent submissions to show."));
        }
    }
}
