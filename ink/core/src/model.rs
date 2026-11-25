//! What the device knows, and the view it draws from that. The clock comes in as local time, so every time on
//! screen is the device's.

use chrono::{DateTime, FixedOffset};

use crate::api::{Fetched, List, Row, Tab, Tier};
use crate::config::Config;

pub const ROWS_PER_PAGE: usize = 7;

/// The tabs in strip order, labelled as in the list window.
const TABS: [(Tab, &str); 4] = [(Tab::NeedsReview, "Review"), (Tab::ReReview, "Re-review"), (Tab::Mine, "Mine"), (Tab::Hidden, "Hidden")];

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
    /// Chosen once, on the first loaded list, so the tab doesn't jump as counts change.
    tab: Option<Tab>,
    page: usize,
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
    pub reviews: u32,
    pub mine: u32,
    /// "updated 14:12", Rado's last refresh in the device's time.
    pub updated: String,
    /// The offline banner, then Rado's source error.
    pub banners: Vec<String>,
    pub tabs: Vec<TabView>,
    pub rows: Vec<RowView>,
    /// The page shown, from 1, and how many the tab has.
    pub page: usize,
    pub pages: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabView {
    pub tab: Tab,
    pub label: &'static str,
    pub count: usize,
    pub selected: bool,
    /// The tab holds *Review next*.
    pub next: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowView {
    pub key: String,
    pub title: String,
    /// "api !101 · web #102".
    pub prs: String,
    /// Age, PR count, votes, comments and ticket, in that order.
    pub facts: Vec<String>,
    pub tier: Tier,
    pub draft_ready: bool,
    pub review_next: bool,
}

impl Model {
    pub fn new(config: Result<Config, String>) -> Self {
        Model { config, link: Link::Connecting, list: None, heard_at: None, tab: None, page: 0 }
    }

    pub fn fetched(&mut self, fetched: Fetched, now: DateTime<FixedOffset>) {
        match fetched {
            Fetched::Changed { list, .. } => {
                // Rado lists nothing until its sources first answer; an empty Review tab then means nothing yet.
                if list.last_refreshed.is_some() || !list.rows.is_empty() {
                    self.tab.get_or_insert_with(|| default_tab(&list.rows));
                }
                self.list = Some(*list);
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

    pub fn failed(&mut self, failure: Failure) {
        self.link = Link::Failing(failure);
    }

    pub fn select_tab(&mut self, tab: Tab) {
        if self.tab != Some(tab) {
            self.tab = Some(tab);
            self.page = 0;
        }
    }

    /// Turns a page forward or back, staying within the tab's pages.
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
        let tab = self.tab.unwrap_or(Tab::NeedsReview);
        let next = review_next(&list.rows);
        let next_key = next.map(|r| r.key.as_str());
        let in_tab = self.rows_in_tab();
        View::List(ListView {
            workspace: list.workspace.clone(),
            reviews: list.counts.reviews,
            mine: list.counts.mine,
            updated: list
                .last_refreshed
                .map_or("not updated yet".into(), |at| format!("updated {}", at.with_timezone(&now.timezone()).format("%H:%M"))),
            banners,
            tabs: TABS
                .iter()
                .map(|&(t, label)| TabView {
                    tab: t,
                    label,
                    count: list.rows.iter().filter(|r| r.tab == t).count(),
                    selected: t == tab,
                    next: next.is_some_and(|r| r.tab == t),
                })
                .collect(),
            rows: in_tab
                .iter()
                .skip(self.page * ROWS_PER_PAGE)
                .take(ROWS_PER_PAGE)
                .map(|row| row_view(row, next_key == Some(row.key.as_str())))
                .collect(),
            page: self.page + 1,
            pages: self.pages(),
        })
    }

    fn rows_in_tab(&self) -> Vec<&Row> {
        let tab = self.tab.unwrap_or(Tab::NeedsReview);
        self.list.iter().flat_map(|l| &l.rows).filter(|r| r.tab == tab).collect()
    }

    fn pages(&self) -> usize {
        self.rows_in_tab().len().div_ceil(ROWS_PER_PAGE).max(1)
    }
}

/// The list window's rule for rows nobody opened yet: the first row, in Rado's order, waiting on my review.
fn review_next(rows: &[Row]) -> Option<&Row> {
    rows.iter().find(|r| r.waiting && r.tab != Tab::Mine)
}

/// The first of Review, Re-review and Mine with rows, else Review.
fn default_tab(rows: &[Row]) -> Tab {
    [Tab::NeedsReview, Tab::ReReview, Tab::Mine]
        .into_iter()
        .find(|&t| rows.iter().any(|r| r.tab == t))
        .unwrap_or(Tab::NeedsReview)
}

fn age(hours: i64) -> String {
    match hours {
        ..1 => "new".into(),
        1..24 => format!("{hours}h"),
        _ => format!("{}d", hours / 24),
    }
}

fn plural(n: u32, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

fn row_view(row: &Row, review_next: bool) -> RowView {
    let prs = row
        .prs
        .iter()
        .map(|pr| format!("{} {}{}", pr.repo, if pr.provider == crate::api::Provider::Github { "#" } else { "!" }, pr.number))
        .collect::<Vec<_>>()
        .join(" · ");
    let mut facts = vec![age(row.age_hours)];
    if row.prs.len() > 1 {
        facts.push(format!("{} PRs", row.prs.len()));
    }
    if row.approvals > 0 {
        facts.push(format!("{} approved", row.approvals));
    }
    if row.objections > 0 {
        facts.push(plural(row.objections, "objection"));
    }
    if let Some(c) = row.comments.as_ref().filter(|c| c.total > 0) {
        let open = if c.open_threads > 0 { format!(" ({} open)", c.open_threads) } else { String::new() };
        facts.push(format!("{}{open}", plural(c.total, "comment")));
    }
    facts.extend(row.ticket.clone());
    RowView {
        key: row.key.clone(),
        title: row.title.clone(),
        prs,
        facts,
        tier: row.tier,
        draft_ready: row.draft.is_some(),
        review_next,
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::api::{Comments, Counts, Draft, Pr, Provider};

    fn at(hm: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(&format!("2026-11-25T{hm}:00+02:00")).unwrap()
    }

    fn config() -> Config {
        Config { host: "192.168.1.14".into(), port: 7878, token: "t".into() }
    }

    fn row(key: &str, tab: Tab, waiting: bool) -> Row {
        Row {
            key: key.into(),
            tab,
            tier: Tier::Normal,
            title: format!("Change {key}"),
            ticket: None,
            age_hours: 5,
            approvals: 0,
            objections: 0,
            comments: None,
            waiting,
            draft: None,
            prs: vec![Pr { repo: "api".into(), number: 1, provider: Provider::Ado }],
        }
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

    fn keys(view: &ListView) -> Vec<&str> {
        view.rows.iter().map(|r| r.key.as_str()).collect()
    }

    #[test]
    fn fixture_showsTheHeaderTabsAndRows() {
        let fixture = include_str!("../../fixtures/list.json");
        let mut model = Model::new(Ok(config()));
        model.fetched(Fetched::Changed { list: serde_json::from_str(fixture).unwrap(), etag: None }, at("14:12"));
        let view = shown(&model, "14:12");
        assert_eq!((view.workspace.as_str(), view.reviews, view.mine), ("Acme", 3, 1));
        assert_eq!(view.updated, "updated 11:30", "Rado's 09:30 UTC in the device's +02:00");
        let tabs: Vec<_> = view.tabs.iter().map(|t| (t.label, t.count, t.selected)).collect();
        assert_eq!(tabs, [("Review", 2, true), ("Re-review", 1, false), ("Mine", 2, false), ("Hidden", 1, false)]);
        assert_eq!(keys(&view), ["pr:1", "branch:GitHub:feature/shared"]);
        let first = &view.rows[0];
        assert_eq!(first.prs, "api !101");
        assert_eq!(first.facts, ["1d", "1 approved", "3 comments (1 open)", "ABC-12"]);
        assert_eq!((first.tier, first.draft_ready, first.review_next), (Tier::Urgent, true, true));
        assert_eq!(view.rows[1].prs, "web #102 · app #103");
        assert_eq!(view.rows[1].facts, ["2d", "2 PRs"]);
    }

    #[test]
    fn defaultTab_isTheFirstOfReviewReReviewMineWithRows_chosenOnce() {
        let model = loaded(vec![row("h", Tab::Hidden, false), row("m", Tab::Mine, true), row("r", Tab::ReReview, true)]);
        assert!(shown(&model, "14:12").tabs[1].selected);

        let mut model = loaded(vec![row("h", Tab::Hidden, false)]);
        assert!(shown(&model, "14:12").tabs[0].selected);
        model.fetched(list_of(vec![row("m", Tab::Mine, true)]), at("14:13"));
        assert!(shown(&model, "14:13").tabs[0].selected, "a later list doesn't move the tab");
    }

    #[test]
    fn defaultTab_waitsForRadosFirstPoll() {
        let mut model = Model::new(Ok(config()));
        let Fetched::Changed { mut list, etag } = list_of(vec![]) else { unreachable!() };
        list.last_refreshed = None;
        model.fetched(Fetched::Changed { list, etag }, at("14:12"));
        assert_eq!(shown(&model, "14:12").updated, "not updated yet");
        model.fetched(list_of(vec![row("m", Tab::Mine, true)]), at("14:13"));
        assert!(shown(&model, "14:13").tabs[2].selected);
    }

    #[test]
    fn reviewNext_isTheFirstRowWaitingOnMyReview_inRadosOrder() {
        let mut model = loaded(vec![
            row("mine", Tab::Mine, true),
            row("quiet", Tab::NeedsReview, false),
            row("b", Tab::ReReview, true),
            row("a", Tab::NeedsReview, true),
        ]);
        let marked = |model: &Model| shown(model, "14:20").rows.into_iter().filter(|r| r.review_next).map(|r| r.key).collect::<Vec<_>>();
        assert!(marked(&model).is_empty(), "the next review is on the Re-review tab");
        let tabs: Vec<_> = shown(&model, "14:20").tabs.iter().map(|t| t.next).collect();
        assert_eq!(tabs, [false, true, false, false]);
        model.select_tab(Tab::ReReview);
        assert_eq!(marked(&model), ["b"]);
    }

    #[test]
    fn pages_holdSevenRows_turnWithinTheTab_andResetOnATabChange() {
        let rows: Vec<Row> = (0..16).map(|i| row(&format!("r{i}"), Tab::NeedsReview, true)).collect();
        let mut model = loaded(rows);
        let view = shown(&model, "14:12");
        assert_eq!((view.rows.len(), view.page, view.pages), (7, 1, 3));
        model.turn_page(false);
        assert_eq!(shown(&model, "14:12").page, 1);
        model.turn_page(true);
        model.turn_page(true);
        model.turn_page(true);
        let view = shown(&model, "14:12");
        assert_eq!((keys(&view), view.page), (vec!["r14", "r15"], 3));

        model.select_tab(Tab::Hidden);
        let view = shown(&model, "14:12");
        assert_eq!((view.rows.len(), view.page, view.pages), (0, 1, 1));
        model.select_tab(Tab::NeedsReview);
        assert_eq!(shown(&model, "14:12").page, 1);
    }

    #[test]
    fn pages_aShorterList_keepsThePageInRange() {
        let mut model = loaded((0..16).map(|i| row(&format!("r{i}"), Tab::NeedsReview, true)).collect());
        model.turn_page(true);
        model.turn_page(true);
        model.fetched(list_of((0..8).map(|i| row(&format!("r{i}"), Tab::NeedsReview, true)).collect()), at("14:13"));
        let view = shown(&model, "14:13");
        assert_eq!((keys(&view), view.page, view.pages), (vec!["r7"], 2, 2));
    }

    #[test]
    fn offline_keepsTheListUnderABannerWithTheLastAnswersTime_untilRadoAnswersAgain() {
        let mut model = loaded(vec![row("a", Tab::NeedsReview, true)]);
        model.fetched(Fetched::NotModified, at("14:20"));
        model.failed(Failure::Unreachable);
        let view = shown(&model, "14:31");
        assert_eq!(view.banners, ["Offline, list from 14:20"]);
        assert_eq!(keys(&view), ["a"]);

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
        model.fetched(Fetched::Changed { list, etag }, at("14:12"));
        model.failed(Failure::Unreachable);
        assert_eq!(shown(&model, "14:13").banners, ["Offline, list from 14:12", "Azure DevOps: 503; next try 14:15"]);
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
        let mut model = loaded(vec![row("a", Tab::NeedsReview, true)]);
        model.fetched(Fetched::Unauthorized, at("14:13"));
        assert!(matches!(model.view(at("14:13")), View::Setup { problem } if problem.contains("rejected the token")));
        model.fetched(Fetched::NotModified, at("14:14"));
        assert_eq!(keys(&shown(&model, "14:14")), ["a"]);
    }

    #[test]
    fn rowFacts_countsAndAge() {
        let mut r = row("a", Tab::NeedsReview, true);
        r.age_hours = 0;
        r.objections = 2;
        r.comments = Some(Comments { total: 1, open_threads: 0 });
        r.draft = Some(Draft {});
        let view = row_view(&r, false);
        assert_eq!(view.facts, ["new", "2 objections", "1 comment"]);
        assert!(view.draft_ready);
    }
}
