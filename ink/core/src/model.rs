//! What the device knows, and the view it draws from that: a notifier for what waits on me, not a copy of the list
//! window. The clock comes in as local time, so every time on screen is the device's.

use std::collections::HashSet;

use chrono::{DateTime, FixedOffset};

use crate::api::{Fetched, List, Row, Tab, Tier};
use crate::config::Config;

/// Lines per page: ten one-line items and two banners fit the Era Color's 483 logical pixels.
pub const ITEMS_PER_PAGE: usize = 10;
/// How long "Nothing new" answers a tap that had nothing to mark seen.
const NOTHING_NEW_FOR: chrono::TimeDelta = chrono::TimeDelta::seconds(3);
/// Tally dots that fit between the margins; the rest show as "+N".
pub const TALLY_DOTS: usize = 30;

/// Why the last list request got no list.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    Unreachable,
    /// Rado answered with an error, or with something the device can't read.
    Answer(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Link {
    Connecting,
    Answering,
    Failing(Failure),
    /// Rado answered 401. Rado also does that while it can't read its own token, so a later list clears it.
    Rejected,
}

#[derive(Debug)]
pub struct Model {
    config: Result<Config, String>,
    link: Link,
    list: Option<List>,
    /// When Rado last answered: the list was current then.
    heard_at: Option<DateTime<FixedOffset>>,
    /// Waiting items I have seen, by key and tab: those on the first list or waiting when I last acknowledged, for as
    /// long as they keep waiting in that tab. One that comes back, or moves to Re-review between two polls, is new.
    seen: HashSet<(String, Tab)>,
    page: usize,
    /// Until when a tap with nothing new says so.
    nothing_new_until: Option<DateTime<FixedOffset>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    /// The config file is missing or wrong, or Rado rejected its token.
    Setup { problem: String },
    /// No list yet.
    Waiting { text: String },
    List(ListView),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListView {
    pub workspace: String,
    /// The counter's two numbers.
    pub reviews: u32,
    pub mine: u32,
    /// How the load feels, from the reviews waiting: "All clear!" to "Review avalanche!".
    pub mood: &'static str,
    pub heat: Heat,
    /// Urgent reviews, and new items, across all pages.
    pub urgent: usize,
    pub new: usize,
    /// One mark per waiting review, in Rado's order, up to `TALLY_DOTS`: the load and its urgency at a glance.
    pub tally: Vec<Tier>,
    /// Waiting reviews past the tally's dots.
    pub tally_more: usize,
    /// Under the chips: how to mark new items seen, or "Nothing new" right after a tap that had none; else empty.
    pub hint: &'static str,
    /// "updated 14:12", Rado's last refresh in the device's time.
    pub updated: String,
    /// The offline banner, then Rado's source error.
    pub banners: Vec<String>,
    pub items: Vec<ItemView>,
    /// The page shown, from 1, and how many there are.
    pub page: usize,
    pub pages: usize,
}

/// The reviews waiting, in four steps, for the colour of the big number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heat {
    Clear,
    Calm,
    Warm,
    Hot,
}

/// One thing waiting on me: a review, then my own PRs.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemView {
    pub title: String,
    /// "<1h", "5h", "2d".
    pub age: String,
    pub tier: Tier,
    pub new: bool,
    pub re_review: bool,
    pub yours: bool,
    pub draft: bool,
}

impl Model {
    pub fn new(config: Result<Config, String>) -> Self {
        Model { config, link: Link::Connecting, list: None, heard_at: None, seen: HashSet::new(), page: 0, nothing_new_until: None }
    }

    pub fn fetched(&mut self, fetched: Fetched, now: DateTime<FixedOffset>) {
        match fetched {
            Fetched::Changed { list, .. } => {
                let first = self.list.is_none();
                self.list = Some(*list);
                let waiting = self.waiting_keys();
                if first {
                    self.seen = waiting;
                } else {
                    self.seen.retain(|seen| waiting.contains(seen));
                }
                self.page = self.page.min(self.pages() - 1);
            }
            Fetched::NotModified => {}
            Fetched::Unauthorized => {
                self.link = Link::Rejected;
                return;
            }
        }
        self.link = Link::Answering;
        self.heard_at = Some(now);
    }

    /// A rejected token stays on screen until Rado answers again.
    pub fn failed(&mut self, failure: Failure) {
        if self.link != Link::Rejected {
            self.link = Link::Failing(failure);
        }
    }

    /// I looked: nothing waiting now is new any more.
    pub fn acknowledge(&mut self, now: DateTime<FixedOffset>) {
        let waiting = self.waiting_keys();
        self.nothing_new_until = waiting.is_subset(&self.seen).then(|| now + NOTHING_NEW_FOR);
        self.seen = waiting;
    }

    fn waiting_keys(&self) -> HashSet<(String, Tab)> {
        self.waiting().iter().map(|r| seen_key(r)).collect()
    }

    /// Turns a page forward or back, staying within the pages.
    pub fn turn_page(&mut self, forward: bool) {
        self.page = if forward { (self.page + 1).min(self.pages() - 1) } else { self.page.saturating_sub(1) };
    }

    pub fn view(&self, now: DateTime<FixedOffset>) -> View {
        let config = match &self.config {
            Ok(config) => config,
            Err(problem) => return View::Setup { problem: problem.clone() },
        };
        if self.link == Link::Rejected {
            return View::Setup { problem: "Rado rejected the token in the config file.".into() };
        }
        let Some(list) = &self.list else {
            let at = format!("Rado at {}:{}", config.host, config.port);
            let text = match &self.link {
                Link::Failing(Failure::Unreachable) => format!("Can't reach {at}. Trying again."),
                Link::Failing(Failure::Answer(text)) => format!("{at} answered: {text}"),
                _ => format!("Connecting to {at}…"),
            };
            return View::Waiting { text };
        };
        let mut banners = Vec::new();
        if let (Link::Failing(failure), Some(heard_at)) = (&self.link, self.heard_at) {
            let from = heard_at.with_timezone(&now.timezone()).format("%H:%M");
            banners.push(match failure {
                Failure::Unreachable => format!("Offline, list from {from}"),
                Failure::Answer(text) => format!("{text} · list from {from}"),
            });
        }
        banners.extend(list.error.clone());
        let items = self.waiting();
        let reviews_waiting: Vec<Tier> = items.iter().filter(|r| r.tab != Tab::Mine).map(|r| r.tier).collect();
        let urgent = reviews_waiting.iter().filter(|&&t| t == Tier::Urgent).count();
        let new = items.iter().filter(|r| !self.seen.contains(&seen_key(r))).count();
        let reviews = list.counts.reviews;
        let (mood, heat) = match reviews {
            0 if items.is_empty() => ("All clear!", Heat::Clear),
            0 => ("No reviews waiting", Heat::Clear),
            1 => ("Just one. Quick win?", Heat::Calm),
            2..=3 => ("A few waiting", Heat::Calm),
            // Hot and the avalanche at 70% and 100% of the 40 PRs where the laptop's counter turns full red.
            4..=27 => ("Getting busy", Heat::Warm),
            28..=39 => ("Stacking up", Heat::Hot),
            _ => ("Review avalanche!", Heat::Hot),
        };
        View::List(ListView {
            workspace: list.workspace.clone(),
            reviews: list.counts.reviews,
            mine: list.counts.mine,
            mood,
            heat,
            urgent,
            new,
            tally: reviews_waiting.iter().copied().take(TALLY_DOTS).collect(),
            tally_more: reviews_waiting.len().saturating_sub(TALLY_DOTS),
            hint: if new > 0 {
                "Tap here to mark them seen"
            // Bounded both ways: the device clock can jump back when Wi-Fi syncs it.
            } else if self.nothing_new_until.is_some_and(|until| now < until && until - now <= NOTHING_NEW_FOR) {
                "Nothing new"
            } else {
                ""
            },
            updated: list
                .last_refreshed
                .map_or("not updated yet".into(), |at| format!("updated {}", at.with_timezone(&now.timezone()).format("%H:%M"))),
            banners,
            items: items
                .iter()
                .skip(self.page * ITEMS_PER_PAGE)
                .take(ITEMS_PER_PAGE)
                .map(|row| ItemView {
                    title: row.title.clone(),
                    age: age(row.age_hours),
                    tier: row.tier,
                    new: !self.seen.contains(&seen_key(row)),
                    re_review: row.tab == Tab::ReReview,
                    yours: row.tab == Tab::Mine,
                    draft: row.draft.is_some(),
                })
                .collect(),
            page: self.page + 1,
            pages: self.pages(),
        })
    }

    /// What the counter counts, reviews first and then my own PRs, each in Rado's order.
    fn waiting(&self) -> Vec<&Row> {
        let rows: Vec<&Row> = self.list.iter().flat_map(|l| &l.rows).filter(|r| r.waiting).collect();
        let (yours, reviews): (Vec<&Row>, Vec<&Row>) = rows.into_iter().partition(|r| r.tab == Tab::Mine);
        reviews.into_iter().chain(yours).collect()
    }

    fn pages(&self) -> usize {
        self.waiting().len().div_ceil(ITEMS_PER_PAGE).max(1)
    }
}

fn seen_key(row: &Row) -> (String, Tab) {
    (row.key.clone(), row.tab)
}

fn age(hours: i64) -> String {
    match hours {
        ..1 => "<1h".into(),
        1..24 => format!("{hours}h"),
        _ => format!("{}d", hours / 24),
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::api::{Counts, Draft};

    fn at(hm: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(&format!("2026-11-25T{hm}:00+02:00")).unwrap()
    }

    fn config() -> Config {
        Config { host: "192.168.1.14".into(), port: 7878, token: "t".into() }
    }

    fn row(key: &str, tab: Tab, waiting: bool) -> Row {
        Row { key: key.into(), tab, tier: Tier::Normal, title: format!("Change {key}"), age_hours: 5, waiting, draft: None }
    }

    fn list_of(rows: Vec<Row>) -> Fetched {
        Fetched::Changed {
            list: Box::new(List {
                workspace: "Acme".into(),
                counts: Counts { reviews: 3, mine: 1 },
                last_refreshed: Some(at("14:10")),
                error: None,
                rows,
            }),
            etag: Some("\"1\"".into()),
        }
    }

    fn loaded(rows: Vec<Row>) -> Model {
        let mut model = Model::new(Ok(config()));
        model.fetched(list_of(rows), at("14:12"));
        model
    }

    fn shown(model: &Model, now: &str) -> ListView {
        match model.view(at(now)) {
            View::List(view) => view,
            other => panic!("not a list: {other:?}"),
        }
    }

    fn titles(view: &ListView) -> Vec<&str> {
        view.items.iter().map(|i| i.title.as_str()).collect()
    }

    #[test]
    fn fixture_showsWhatWaitsOnMe_reviewsThenMine() {
        let fixture = include_str!("../../fixtures/list.json");
        let mut model = Model::new(Ok(config()));
        model.fetched(Fetched::Changed { list: serde_json::from_str(fixture).unwrap(), etag: None }, at("14:12"));
        let view = shown(&model, "14:12");
        assert_eq!((view.workspace.as_str(), view.reviews, view.mine), ("Acme", 3, 1));
        assert_eq!(view.updated, "updated 11:30", "Rado's 09:30 UTC in the device's +02:00");
        assert_eq!((view.urgent, view.new, view.mood, view.heat), (1, 0, "A few waiting", Heat::Calm));
        assert_eq!(view.tally, [Tier::Urgent, Tier::Normal, Tier::Normal], "the three waiting reviews, not mine");
        assert_eq!(titles(&view), ["Change 1", "Change 4", "Change 2", "Change 6"], "waiting rows; Hidden, Approved and an unwaiting Mine row left out");
        let first = &view.items[0];
        assert_eq!((first.age.as_str(), first.tier, first.draft, first.new), ("1d", Tier::Urgent, true, false));
        assert!(view.items[1].re_review);
        assert!(view.items[3].yours);
    }

    #[test]
    fn nothingWaiting_isAllClear() {
        let mut model = Model::new(Ok(config()));
        let Fetched::Changed { mut list, etag } = list_of(vec![row("m", Tab::Mine, false), row("o", Tab::Other, false)]) else {
            unreachable!()
        };
        list.counts = Counts { reviews: 0, mine: 0 };
        model.fetched(Fetched::Changed { list, etag }, at("14:12"));
        let view = shown(&model, "14:12");
        assert_eq!((view.mood, view.heat, view.items.len(), view.tally.len()), ("All clear!", Heat::Clear, 0, 0));
    }

    #[test]
    fn rowsThatArrive_areNew_untilAcknowledged() {
        let mut model = loaded(vec![row("a", Tab::Other, true)]);
        assert!(shown(&model, "14:12").items.iter().all(|i| !i.new), "the first list is not news");

        model.fetched(list_of(vec![row("b", Tab::Other, true), row("a", Tab::Other, true)]), at("14:13"));
        let view = shown(&model, "14:13");
        assert_eq!(view.items.iter().map(|i| i.new).collect::<Vec<_>>(), [true, false]);
        assert_eq!(view.new, 1);

        assert_eq!(view.hint, "Tap here to mark them seen");

        model.acknowledge(at("14:14"));
        let view = shown(&model, "14:14");
        assert!(view.items.iter().all(|i| !i.new));
        assert_eq!((view.new, view.hint), (0, ""));
    }

    #[test]
    fn aTapWithNothingNew_saysSoForAFewSeconds() {
        let mut model = loaded(vec![row("a", Tab::Other, true)]);
        model.acknowledge(DateTime::parse_from_rfc3339("2026-11-25T14:12:00+02:00").unwrap());
        let at_second = |s: u32| DateTime::parse_from_rfc3339(&format!("2026-11-25T14:12:0{s}+02:00")).unwrap();
        let hint = |model: &Model, s| match model.view(at_second(s)) {
            View::List(view) => view.hint,
            other => panic!("not a list: {other:?}"),
        };
        assert_eq!((hint(&model, 2), hint(&model, 3)), ("Nothing new", ""));
        let an_hour_earlier = DateTime::parse_from_rfc3339("2026-11-25T13:12:01+02:00").unwrap();
        assert!(matches!(model.view(an_hour_earlier), View::List(view) if view.hint.is_empty()), "the clock jumped back");
    }

    #[test]
    fn anItemThatStartsWaitingAgain_isNew() {
        let mut model = loaded(vec![row("approved", Tab::Other, false), row("r", Tab::Other, true)]);
        model.fetched(list_of(vec![row("approved", Tab::ReReview, true), row("r", Tab::Other, true)]), at("14:13"));
        assert_eq!(shown(&model, "14:13").items.iter().map(|i| i.new).collect::<Vec<_>>(), [true, false], "approved, then a re-review");

        model.acknowledge(at("14:13"));
        model.fetched(list_of(vec![row("approved", Tab::Other, false), row("r", Tab::Other, true)]), at("14:14"));
        model.fetched(list_of(vec![row("approved", Tab::ReReview, true), row("r", Tab::Other, true)]), at("14:15"));
        assert!(shown(&model, "14:15").items[0].new, "seen, gone, back");

        model.acknowledge(at("14:13"));
        model.fetched(list_of(vec![row("approved", Tab::Other, true), row("r", Tab::Other, true)]), at("14:16"));
        model.acknowledge(at("14:13"));
        model.fetched(list_of(vec![row("approved", Tab::ReReview, true), row("r", Tab::Other, true)]), at("14:17"));
        assert!(shown(&model, "14:17").items[0].new, "approved and pushed to between two polls: a re-review");
    }

    #[test]
    fn urgentNewAndTally_countEveryPage_andTheTallyStopsAtItsDots() {
        let urgent = |key: &str, tab| Row { tier: Tier::Urgent, ..row(key, tab, true) };
        let mut rows: Vec<Row> = (0..34).map(|i| row(&format!("r{i}"), Tab::Other, true)).collect();
        rows.push(urgent("u", Tab::Other));
        rows.push(urgent("mine", Tab::Mine));
        let mut model = loaded(rows.clone());
        rows.push(row("n", Tab::Other, true));
        model.fetched(list_of(rows), at("14:13"));
        let view = shown(&model, "14:13");
        assert_eq!((view.urgent, view.new), (1, 1), "urgent counts reviews, as the tally does; mine is not one");
        assert_eq!((view.tally.len(), view.tally_more), (TALLY_DOTS, 6), "36 waiting reviews");
        assert_eq!(view.items.len(), ITEMS_PER_PAGE);
    }

    #[test]
    fn moodAndHeat_followTheReviewsWaiting() {
        let mood = |reviews: u32, rows: Vec<Row>| {
            let mut model = Model::new(Ok(config()));
            let Fetched::Changed { mut list, etag } = list_of(rows) else { unreachable!() };
            list.counts.reviews = reviews;
            model.fetched(Fetched::Changed { list, etag }, at("14:12"));
            let view = shown(&model, "14:12");
            (view.mood, view.heat)
        };
        assert_eq!(mood(0, vec![row("m", Tab::Mine, true)]), ("No reviews waiting", Heat::Clear));
        assert_eq!(mood(1, vec![]), ("Just one. Quick win?", Heat::Calm));
        assert_eq!(mood(3, vec![]), ("A few waiting", Heat::Calm));
        assert_eq!(mood(4, vec![]), ("Getting busy", Heat::Warm));
        assert_eq!(mood(27, vec![]), ("Getting busy", Heat::Warm));
        assert_eq!(mood(28, vec![]), ("Stacking up", Heat::Hot));
        assert_eq!(mood(39, vec![]), ("Stacking up", Heat::Hot));
        assert_eq!(mood(40, vec![]), ("Review avalanche!", Heat::Hot));
    }

    #[test]
    fn pages_holdTenItems_andTurnWithinThem() {
        let mut model = loaded((0..23).map(|i| row(&format!("r{i}"), Tab::Other, true)).collect());
        let view = shown(&model, "14:12");
        assert_eq!((view.items.len(), view.page, view.pages), (10, 1, 3));
        model.turn_page(false);
        assert_eq!(shown(&model, "14:12").page, 1);
        for _ in 0..3 {
            model.turn_page(true);
        }
        let view = shown(&model, "14:12");
        assert_eq!((titles(&view), view.page), (vec!["Change r20", "Change r21", "Change r22"], 3));
    }

    #[test]
    fn pages_aShorterList_keepsThePageInRange() {
        let mut model = loaded((0..23).map(|i| row(&format!("r{i}"), Tab::Other, true)).collect());
        model.turn_page(true);
        model.turn_page(true);
        model.fetched(list_of((0..11).map(|i| row(&format!("r{i}"), Tab::Other, true)).collect()), at("14:13"));
        let view = shown(&model, "14:13");
        assert_eq!((titles(&view), view.page, view.pages), (vec!["Change r10"], 2, 2));
    }

    #[test]
    fn offline_keepsTheListUnderABannerWithTheLastAnswersTime_untilRadoAnswersAgain() {
        let mut model = loaded(vec![row("a", Tab::Other, true)]);
        model.fetched(Fetched::NotModified, at("14:20"));
        model.failed(Failure::Unreachable);
        let view = shown(&model, "14:31");
        assert_eq!(view.banners, ["Offline, list from 14:20"]);
        assert_eq!(titles(&view), ["Change a"]);

        model.failed(Failure::Answer("Rado answered 404".into()));
        assert_eq!(shown(&model, "14:31").banners, ["Rado answered 404 · list from 14:20"], "a bad answer is not offline");

        model.fetched(Fetched::NotModified, at("14:32"));
        assert!(shown(&model, "14:32").banners.is_empty());
    }

    #[test]
    fn offline_timeIsTheDevicesLocalTime() {
        let mut model = Model::new(Ok(config()));
        model.fetched(list_of(vec![]), DateTime::parse_from_rfc3339("2026-11-25T12:20:00Z").unwrap());
        model.failed(Failure::Unreachable);
        assert_eq!(shown(&model, "14:31").banners, ["Offline, list from 14:20"]);
    }

    #[test]
    fn sourceError_isABannerUnderTheOfflineOne() {
        let mut model = Model::new(Ok(config()));
        let Fetched::Changed { mut list, etag } = list_of(vec![]) else { unreachable!() };
        list.error = Some("Azure DevOps: 503; next try 14:15".into());
        list.last_refreshed = None;
        model.fetched(Fetched::Changed { list, etag }, at("14:12"));
        model.failed(Failure::Unreachable);
        let view = shown(&model, "14:13");
        assert_eq!(view.banners, ["Offline, list from 14:12", "Azure DevOps: 503; next try 14:15"]);
        assert_eq!(view.updated, "not updated yet");
    }

    #[test]
    fn beforeTheFirstList_saysWhatItIsDoing() {
        let mut model = Model::new(Ok(config()));
        assert_eq!(model.view(at("14:12")), View::Waiting { text: "Connecting to Rado at 192.168.1.14:7878…".into() });
        model.failed(Failure::Unreachable);
        assert_eq!(model.view(at("14:12")), View::Waiting { text: "Can't reach Rado at 192.168.1.14:7878. Trying again.".into() });
        model.failed(Failure::Answer("Rado answered 404".into()));
        assert_eq!(model.view(at("14:12")), View::Waiting { text: "Rado at 192.168.1.14:7878 answered: Rado answered 404".into() });
    }

    #[test]
    fn setup_forABadConfig() {
        let model = Model::new(Err("There is no rado.cfg.".into()));
        assert_eq!(model.view(at("14:12")), View::Setup { problem: "There is no rado.cfg.".into() });
    }

    #[test]
    fn rejectedToken_isSetup_untilRadoAnswersWithAList() {
        let mut model = loaded(vec![row("a", Tab::Other, true)]);
        model.fetched(Fetched::Unauthorized, at("14:13"));
        assert!(matches!(model.view(at("14:13")), View::Setup { problem } if problem.contains("rejected the token")));
        model.failed(Failure::Unreachable);
        model.failed(Failure::Answer("Rado answered 500".into()));
        assert!(matches!(model.view(at("14:13")), View::Setup { .. }), "rejected until Rado sends a list again");
        model.fetched(Fetched::NotModified, at("14:14"));
        assert_eq!(titles(&shown(&model, "14:14")), ["Change a"]);
    }

    #[test]
    fn age_readsLikeTheListWindow() {
        let mut fresh = row("a", Tab::Other, true);
        fresh.age_hours = 0;
        fresh.draft = Some(Draft {});
        let mut old = row("b", Tab::Other, true);
        old.age_hours = 50;
        let view = shown(&loaded(vec![fresh, old]), "14:12");
        assert_eq!(view.items.iter().map(|i| i.age.as_str()).collect::<Vec<_>>(), ["<1h", "2d"]);
        assert!(view.items[0].draft);
    }
}
