use super::*;

type Destination = (&'static str, &'static str, &'static str);
const RAIL: &[Destination] = &[
    ("NavTop", "home", "Front page"),
    ("NavNew", "clock", "Latest"),
    ("NavBest", "star", "Best"),
    ("NavAsk", "chat", "Ask HN"),
    ("NavShow", "lightbulb", "Show HN"),
    ("NavJobs", "briefcase", "Jobs"),
    ("NavSaved", "bookmark", "Saved for later"),
    ("NavHistory", "history", "History"),
    ("NavHidden", "eye_off", "Hidden stories"),
    ("NavMore", "more", "More"),
];
const TABS: &[Destination] = &[
    ("MobileHome", "home", "Home"),
    ("MobileNew", "clock", "Latest"),
    ("MobileSearch", "search", "Search"),
    ("MobileSaved", "bookmark", "Saved"),
    ("MobileMore", "more", "More"),
];

impl ReaderView {
    pub(super) fn update_navigation(&self) {
        let feed = self.feed.borrow().clone();
        let detail = self.detail.borrow().clone();
        for (name, route) in [
            ("NavTop", FeedRoute::Feed(FeedKind::Top)),
            ("MobileHome", FeedRoute::Feed(FeedKind::Top)),
            ("NavNew", FeedRoute::Feed(FeedKind::New)),
            ("MobileNew", FeedRoute::Feed(FeedKind::New)),
            ("NavBest", FeedRoute::Feed(FeedKind::Best)),
            ("NavAsk", FeedRoute::Feed(FeedKind::Ask)),
            ("NavShow", FeedRoute::Feed(FeedKind::Show)),
            ("NavJobs", FeedRoute::Feed(FeedKind::Jobs)),
            ("NavSaved", FeedRoute::Saved),
            ("MobileSaved", FeedRoute::Saved),
            ("NavHistory", FeedRoute::History),
            ("NavHidden", FeedRoute::Hidden),
        ] {
            self.mark_destination(
                name,
                feed == route
                    && !matches!(
                        detail,
                        DetailRoute::Search | DetailRoute::Settings | DetailRoute::More
                    ),
            );
        }
        self.mark_destination(
            "MobileSearch",
            detail == DetailRoute::Search
                || (matches!(feed, FeedRoute::Search(_))
                    && !matches!(detail, DetailRoute::Settings | DetailRoute::More)),
        );
        self.mark_destination("NavMore", detail == DetailRoute::More);
        self.mark_destination(
            "MobileMore",
            matches!(detail, DetailRoute::Settings | DetailRoute::More)
                || (!matches!(detail, DetailRoute::Search)
                    && matches!(
                        feed,
                        FeedRoute::Feed(
                            FeedKind::Ask | FeedKind::Show | FeedKind::Jobs | FeedKind::Best
                        ) | FeedRoute::History
                            | FeedRoute::Hidden
                    )),
        );
    }
    pub(super) fn populate_navigation(&self) {
        for (name, icon, label) in [
            ("WelcomeSearch", "search", "Search"),
            ("WelcomeSaved", "bookmark", "Saved stories"),
        ] {
            self.button(name)
                .set_content(Some(Control::boxed(&widgets::nav_content(
                    icon, label, false,
                ))));
        }
        for (index, &(name, icon, label)) in RAIL.iter().enumerate() {
            self.wire_destination_keys(RAIL, index, false);
            let button = self.button(name);
            ToolTip::set_tip(&button, Some(Rc::new(label.to_owned()) as BoxedValue));
            button.set_content(Some(Control::boxed(&widgets::nav_content(
                icon, label, false,
            ))));
        }
        for (index, &(name, icon, label)) in TABS.iter().enumerate() {
            self.wire_destination_keys(TABS, index, true);
            let b = self.button(name);
            b.set_padding(Thickness::symmetric(2.0, 2.0));
            b.set_horizontal_content_alignment(ferroui_base::layout::HorizontalAlignment::Center);
            b.set_content(Some(Control::boxed(&widgets::nav_content(
                icon, label, true,
            ))));
        }
    }

    pub(super) fn navigation_focus_after_resize(
        &self,
        mode: WorkspaceLayout,
    ) -> Option<&'static str> {
        let focused = TopLevel::get_top_level(Some(self))?
            .focus_manager()
            .get_focused_element()?;
        if !self.is_visual_ancestor_of(&focused) {
            return None;
        }
        let name = focused.name()?;
        if matches!(
            mode,
            WorkspaceLayout::Phone | WorkspaceLayout::ShortLandscape
        ) {
            return match name.as_str() {
                "NavTop" => Some("MobileHome"),
                "NavNew" => Some("MobileNew"),
                "NavSaved" => Some("MobileSaved"),
                "CompactSearch" | "SearchBox" | "SearchButton" => Some("MobileSearch"),
                other if RAIL.iter().any(|&(id, _, _)| id == other) => Some("MobileMore"),
                _ => None,
            };
        }
        if mode == WorkspaceLayout::Reading {
            return None;
        }
        match name.as_str() {
            "MobileHome" => Some("NavTop"),
            "MobileNew" => Some("NavNew"),
            "MobileSaved" => Some("NavSaved"),
            "MobileMore" => Some("NavMore"),
            "MobileSearch" | "CompactSearch" if mode == WorkspaceLayout::LabeledRail => {
                Some("SearchBox")
            }
            "MobileSearch" | "SearchBox" | "SearchButton"
                if mode == WorkspaceLayout::CompactRail =>
            {
                Some("CompactSearch")
            }
            _ => None,
        }
    }

    fn mark_destination(&self, name: &str, current: bool) {
        let button = self.button(name);
        if button.classes().contains("active") != current {
            button.classes().set("active", current);
            AutomationProperties::set_item_status(
                &button,
                current.then_some("Current destination"),
            );
        }
    }

    fn wire_destination_keys(&self, group: &'static [Destination], index: usize, primary: bool) {
        let owner = self.to_ref().downgrade();
        self.button(group[index].0).add_handler(
            InputElement::key_down_event(),
            move |_, event: &KeyEventArgs| {
                if event.key_modifiers != KeyModifiers::NONE {
                    return;
                }
                let Some(view) = owner.upgrade() else {
                    return;
                };
                let horizontal = primary && view.layout_mode.get() == Some(WorkspaceLayout::Phone);
                let rtl = view.flow_direction() == ferroui_base::media::FlowDirection::RightToLeft;
                let (previous, next) = match (horizontal, rtl) {
                    (false, _) => (Key::Up, Key::Down),
                    (true, false) => (Key::Left, Key::Right),
                    (true, true) => (Key::Right, Key::Left),
                };
                let next = match event.key {
                    Key::Home => 0,
                    Key::End => group.len() - 1,
                    key if key == previous => index.saturating_sub(1),
                    key if key == next => (index + 1).min(group.len() - 1),
                    _ => return,
                };
                let target = view.button(group[next].0);
                target.focus_with(
                    ferroui_base::input::NavigationMethod::Directional,
                    KeyModifiers::NONE,
                );
                target.bring_into_view();
                event.set_handled(true);
            },
        );
    }
}
