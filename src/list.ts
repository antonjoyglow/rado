import { api, watchFresh, watchSnapshot, type Browser, type CustomTheme, type InkStatus, type Member, type ReviewItem, type Settings, type Snapshot, type Theme, type Tier } from "./api";
import { applyTheme, parseHex, unreadable } from "./theme";
import {
  ageColumn, datesTooltip, draftMarker, hideButton, LIST_HEIGHT_PERCENT, LIST_WIDTH_PERCENT, newestFirst, nextReview, parseWhole, parseMinutes, prLabel, rowFacts, stepScale, TEXT_CHOICES, TEXT_SCALE, TEXT_STEP,
} from "./format";
import { POLL_MINUTES } from "./bindings/limits";
import { h, whenBlurred } from "./dom";
import { bird, glow, yawn } from "./bird";
import { glows, listActions, newReviewsText, onFocus, onFresh, onSnapshot, refreshTitle, showStatus, statusLine, type Yawn } from "./status";
import { counterPanel } from "./counter";
import { DROPPED, lastingSaver, type LastingSaver } from "./save";
import { extensionMessage } from "./extension";
import { activeWorkspace, isTyping, stepWorkspace, trackerName, workspaceChip, workspaceStep, workspacesPanel } from "./workspaces";
import {
  defaultListTab, inSection, noReviews, restoreTab, SECTIONS, SETTINGS_TABS, stepTab, type SectionKey, type SettingsTab,
} from "./tabs";

/** When you last opened each row, by row key; "Review next" walks past the ones you just opened. */
const openedAt = new Map<string, number>();
const opened = (key: string) => openedAt.set(key, Date.now());

const LIST_TAB_KEY = "rado.listTab";
const SETTINGS_TAB_KEY = "rado.settingsTab";

// Storage can be missing or throw (private mode, blocked site data); the tabs then just aren't remembered.
function recall(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function remember(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Not remembered; see recall.
  }
}

/**
 * A tablist over buttons the caller made once, so focus survives re-renders. Arrows, Home and End move focus
 * and selection. `onSelect` owns the state and calls `select` to update the ARIA attributes.
 */
function tabList<K extends string>(
  label: string,
  tabs: readonly { key: K; tab: HTMLButtonElement }[],
  panelOf: (key: K) => HTMLElement,
  onSelect: (key: K) => void,
): { list: HTMLElement; select: (key: K) => void } {
  for (const { key, tab } of tabs) {
    tab.type = "button";
    tab.role = "tab";
    tab.setAttribute("aria-controls", panelOf(key).id);
    tab.onclick = () => {
      tab.scrollIntoView({ block: "nearest", inline: "nearest" });
      onSelect(key);
    };
  }
  const list = h("div", { class: "tabs", role: "tablist", ariaLabel: label }, ...tabs.map((t) => t.tab));
  list.addEventListener("keydown", (e) => {
    // Ctrl+arrows switch workspaces, not tabs.
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    const from = tabs.findIndex((t) => t.tab === document.activeElement);
    const to = from < 0 ? null : stepTab(from, e.key, tabs.length);
    if (to === null) return;
    e.preventDefault();
    tabs[to].tab.focus();
    tabs[to].tab.click();
  });
  const select = (key: K) => {
    for (const t of tabs) {
      t.tab.ariaSelected = String(t.key === key);
      t.tab.tabIndex = t.key === key ? 0 : -1;
    }
    panelOf(key).setAttribute("aria-labelledby", tabs.find((t) => t.key === key)!.tab.id);
  };
  return { list, select };
}

const POLL_CHOICES = [1, 3, 5, 10];
const BROWSERS: [Browser, string][] = [["system", "System default"], ["edge", "Edge"], ["chrome", "Chrome"]];
const THEMES: [Theme, string][] = [["system", "System"], ["dark", "Dark"], ["light", "Light"], ["highContrast", "High contrast"], ["custom", "Custom"]];
const CUSTOM_COLOURS: [keyof CustomTheme, string][] = [["background", "Background"], ["text", "Text"], ["accent", "Accent"]];
const LIST_WIDTHS: [string, number][] = [["25%", 25], ["33%", 33], ["Half", 50], ["Full", 100]];
const LIST_HEIGHTS: [string, number][] = [["Half", 50], ["Full", 100]];

export function mountList(root: HTMLElement): void {
  // Until the first snapshot brings the saved theme.
  document.documentElement.dataset.theme = "system";
  // The text is redrawn every second for the "ago" times; the Download line only on a snapshot, so a click or
  // keyboard focus on its button survives.
  const statusText = h("div");
  const actionText = h("div", { class: "failed" });
  const newerText = h("span");
  const newerLine = h("div", { class: "newer", hidden: true }, newerText, h("button", { type: "button", class: "link", onclick: () => void act(api.openNewerRelease()) }, "Download"));
  const status = h("div", { class: "status", hidden: true }, statusText, actionText, newerLine);
  const announcer = h("div", { class: "visually-hidden", role: "status" });
  const body = h("div", { class: "body", id: "list-panel", role: "tabpanel" });
  const settingsPanel = h("div", { class: "settings", hidden: true });
  const heading = h("h1", {}, "Reviews");
  const logo = bird();
  // The banner's own lines aren't a live region: it is hidden until it has something to say.
  const announce = (text: string) => {
    // Cleared first so a screen reader announces a repeat of the same text.
    announcer.textContent = "";
    requestAnimationFrame(() => { announcer.textContent = text; });
  };
  const showFailure = (error: string | null) => {
    actionText.textContent = error ?? "";
    if (error) announce(error);
    renderStatus();
  };
  const act = listActions((error) => {
    // Opening a link can hide the list before a later link of the same row fails; that error belongs to the last visit.
    if (error && document.hidden) return;
    showFailure(error);
  });
  const switchTo = (id: string) => void act(api.switchWorkspace(id));
  const refreshButton = h("button", { class: "icon refresh", type: "button", ariaLabel: "Refresh now", title: "Refresh now", onclick: () => void act(api.refresh()) }, h("span", { class: "refresh-glyph" }, "↻"));
  const settingsButton = h("button", {
    class: "icon",
    type: "button",
    title: "Settings",
    // Keeps focus in a Settings field until the click, so ← commits it the way Esc does.
    onmousedown: (e: MouseEvent) => e.preventDefault(),
    onclick: () => showSettings(settingsPanel.hidden !== false),
  }, "⚙");
  const closeButton = h("button", { class: "icon", type: "button", title: "Close", onclick: () => void api.hideList() }, "✕");
  const workspace = workspaceChip(() => showSettings(true, "workspaces"), switchTo);

  // One tab per section, always all seven so they never move. Created once; renderBody updates them in place.
  const tabs = SECTIONS.map((s) => {
    const count = h("span", { class: "n" });
    const tab = h("button", { class: "tab", id: `tab-${s.key}`, title: s.title }, s.label, count, h("span", { class: "next-dot", ariaHidden: "true" }));
    return { key: s.key, count, tab };
  });
  let tab: SectionKey | null = restoreTab(recall(LIST_TAB_KEY), SECTIONS.map((s) => s.key));
  const scrolls = new Map<SectionKey, number>();
  const strip = tabList("Review sections", tabs, () => body, (key) => {
    if (tab) scrolls.set(tab, body.scrollTop);
    tab = key;
    strip.select(key);
    remember(LIST_TAB_KEY, key);
    renderBody();
    body.scrollTop = scrolls.get(key) ?? 0;
  });

  root.append(
    h("header", {}, logo, heading, workspace.chip, refreshButton, settingsButton, closeButton, workspace.picker),
    status,
    strip.list,
    settingsPanel,
    body,
    announcer,
  );
  strip.select(tab ?? "needsReview");

  // Settings replace the list rather than squeezing it; hiding the list keeps its rows and scroll position.
  let listScroll = 0;
  let settingsTab: SettingsTab = restoreTab(recall(SETTINGS_TAB_KEY), SETTINGS_TABS.map((t) => t.key)) ?? "workspaces";
  let settingsView: { update: (s: Snapshot) => void; select: (key: SettingsTab) => void } | null = null;
  const showSettings = (on: boolean, startTab: SettingsTab = settingsTab) => {
    if (on && !settingsPanel.hidden) settingsView?.select(startTab);
    if (on === !settingsPanel.hidden) return;
    if (on) {
      listScroll = body.scrollTop;
      // Whatever failed on the way out is now in front of you to fix.
      showFailure(null);
    }
    const editing = !on && document.activeElement instanceof HTMLElement && settingsPanel.contains(document.activeElement) ? document.activeElement : null;
    settingsPanel.hidden = !on;
    // The focused field commits now rather than whenever Chromium moves focus off the hidden field, and once hidden,
    // so its save, or the check that refuses it, reports to the status line instead of a hidden note.
    editing?.blur();
    body.hidden = on;
    strip.list.hidden = on;
    heading.textContent = on ? "Settings" : "Reviews";
    settingsButton.textContent = on ? "←" : "⚙";
    settingsButton.title = on ? "Back to reviews" : "Settings";
    settingsView = on && snapshot
      ? renderSettings(settingsPanel, snapshot, startTab, (key) => {
          settingsTab = key;
          remember(SETTINGS_TAB_KEY, key);
        }, showFailure)
      : null;
    if (!on) {
      body.scrollTop = listScroll;
      revealTab();
    }
  };
  // Larger text can push the selected tab past the strip's edge; a hidden strip can't scroll, so this also runs on return from settings.
  const revealTab = () => strip.list.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest", inline: "nearest" });
  // Settings opens on the active workspace, whose editor holds the field.
  const showWatchedRepos = () => {
    showSettings(true, "workspaces");
    settingsPanel.querySelector<HTMLInputElement>('input[aria-label="Watched repos"]')?.focus();
  };

  let snapshot: Snapshot | null = null;
  let pollingSince: number | null = null;

  const renderStatus = () => {
    if (!snapshot || document.hidden) return;
    const now = new Date();
    const line = statusLine(snapshot, pollingSince, now);
    statusText.replaceChildren();
    status.dataset.kind = line?.kind ?? "ok";
    if (line?.kind === "busy") statusText.append(h("span", { class: "spinner" }), line.text);
    else if (line) statusText.append(line.text);
    if (snapshot.warning) statusText.append(h("div", { class: "hint" }, snapshot.warning));
    status.hidden = !showStatus(line, snapshot.warning, actionText.textContent || null, snapshot.newerRelease);
    refreshButton.dataset.busy = String(snapshot.polling);
    refreshButton.title = refreshTitle(snapshot.lastRefreshed, snapshot.settings.pollMinutes, now);
  };

  // A hidden list can't have focus (it hides when it loses it), so a yawn waits for the next focus.
  let heldYawn = 0;
  let yawning: Animation | null = null;
  const settleYawn = ({ held, play }: Yawn) => {
    heldYawn = held;
    if (play === 0) return;
    announce(newReviewsText(play));
    if (yawning?.playState === "running") return;
    yawning = yawn(logo);
  };
  void watchFresh((count) => settleYawn(onFresh(heldYawn, count, document.hasFocus())));

  const renderBody = () => {
    if (!snapshot) return;
    body.replaceChildren();
    if (!snapshot.hasPat && !snapshot.hasGithubToken) {
      if (settingsPanel.hidden) showSettings(true, "workspaces");
      return;
    }
    // One source can have rows while another has a token but never answered, which leaves lastRefreshed unset.
    if (!snapshot.lastRefreshed && snapshot.items.length === 0) {
      body.append(h("p", { class: "empty" }, snapshot.error ? "Nothing loaded yet." : "Loading your reviews…"));
      return;
    }
    const all = snapshot.items;
    // Chosen once, on the first loaded snapshot, so the tab doesn't jump as counts change.
    tab ??= defaultListTab(all);
    // The next review is never hidden, so its tab is its state.
    const next = nextReview(all, openedAt);
    for (const t of tabs) {
      const n = all.filter((i) => inSection(i, t.key)).length;
      t.count.textContent = String(n);
      t.tab.dataset.empty = String(n === 0);
      t.tab.dataset.next = String(t.key === next?.state);
    }
    strip.select(tab);
    const current = tab;
    const inThis = all.filter((i) => inSection(i, current));
    const items = current === "mine" ? newestFirst(inThis) : inThis;
    // Only once every source has answered: until then an empty Review tab may just mean a source hasn't.
    const why = current === "needsReview" && snapshot.lastRefreshed !== null && noReviews(all);
    const watch = why && snapshot.hasGithubToken;
    if (items.length === 0) body.append(h("p", { class: "empty" },
      "Nothing here.",
      why && " Rado lists PRs where you or your teams are reviewers.",
      watch && " To also see others' open PRs in a GitHub repo, add it to ",
      watch && h("button", { type: "button", class: "link", onclick: showWatchedRepos }, "Watched repos"),
      watch && ".",
    ));
    const tracker = trackerName(activeWorkspace(snapshot.settings));
    for (const item of items) body.append(row(item, tracker, item.key === next?.key, act));
  };

  void watchSnapshot((s) => {
    if (s.polling && !snapshot?.polling) pollingSince = Date.now();
    const rescaled = s.settings.textScale !== snapshot?.settings.textScale;
    const refreshed = glows(snapshot, s);
    heldYawn = onSnapshot(heldYawn, snapshot?.settings.activeWorkspace ?? null, s.settings.activeWorkspace);
    snapshot = s;
    applyTheme(document.documentElement, s.settings.theme, s.settings.customTheme);
    document.body.style.setProperty("--text-scale", String(s.settings.textScale / 100));
    workspace.render(s);
    settingsView?.update(s);
    newerLine.hidden = !s.newerRelease;
    newerText.textContent = s.newerRelease ? `Rado ${s.newerRelease.version} is available · ` : "";
    renderStatus();
    renderBody();
    if (rescaled) revealTab();
    if (refreshed) glow(logo);
  });
  setInterval(renderStatus, 1000);
  document.addEventListener("visibilitychange", () => {
    // A failed action's error is about that visit; the list hides when it loses focus.
    if (document.hidden) actionText.textContent = "";
    renderStatus();
  });
  window.addEventListener("focus", () => {
    // Rows opened since the last render change which review is next.
    renderBody();
    settleYawn(onFocus(heldYawn));
  });
  window.addEventListener("keydown", (e) => {
    const step = workspaceStep(e);
    if (step !== null && snapshot && !isTyping(e.target)) {
      e.preventDefault();
      const to = stepWorkspace(snapshot.settings.workspaces, snapshot.settings.activeWorkspace, step);
      if (to) switchTo(to);
      return;
    }
    if (e.key !== "Escape") return;
    if (workspace.close()) workspace.chip.focus();
    else if (settingsPanel.hidden) void api.hideList();
    else showSettings(false);
  });
}

const TIER_LABEL: Record<Tier, string> = { urgent: "Urgent", normal: "Normal", low: "Low" };

function row(item: ReviewItem, tracker: string, isNext: boolean, act: ReturnType<typeof listActions>): HTMLElement {
  const aggregate = item.members.length > 1;
  // Plain text, so the outlined chips are exactly the things that open something. The separating dots are
  // drawn by the stylesheet inside each phrase, so a line never starts with one.
  const facts = rowFacts(item).map(({ text, kind, title }) => {
    const fact = h("span", { class: kind }, text);
    if (title) fact.title = title;
    return fact;
  });
  if (isNext) facts.unshift(h("span", { class: "next-label" }, "Review next"));
  const meta = facts.flatMap((f, i) => (i === 0 ? [f] : [" ", f]));
  const members = item.members.flatMap((m) => [h("button", {
      class: "member",
      type: "button",
      title: m.state ? `Open ${prLabel(m)}: ${m.title}` : `Open ${prLabel(m)} (same branch, not assigned to you): ${m.title}`,
      data: { assigned: String(m.state !== null) },
      onclick: (e: MouseEvent) => {
        e.stopPropagation();
        void act(api.openPr(m.id).then(() => opened(item.key)));
      },
    }, prLabel(m)), draftChip(m, item.key)]);
  const ticket = item.ticket && h("button", {
    class: "member",
    type: "button",
    title: `Open ${item.ticket} in ${tracker}`,
    onclick: (e: MouseEvent) => {
      e.stopPropagation();
      void act(api.openTicket(item.key));
    },
  }, item.ticket);
  const copy: HTMLButtonElement = h("button", {
    class: "action",
    type: "button",
    title: aggregate ? "Copy all links, one 'repo: link' per line" : "Copy the link",
    onclick: () => api.copyRow(item.key).then(() => flash(copy, "Copied"), () => flash(copy, "Failed")),
  }, "Copy");
  const tier = item.hidden ? "low" : item.tier;
  // The title button is the keyboard target for opening the row: its click bubbles to the row, as does a click
  // anywhere else on it. The chips stop their clicks, and the actions cell stops its own.
  return h("div", {
    class: "row",
    data: { tier, next: String(isNext) },
    // Marked first: a row of several PRs can fail on a later link after the first one opened.
    onclick: () => {
      opened(item.key);
      void act(api.openRow(item.key));
    },
  },
    h("span", { class: "dot", role: "img", title: `Priority: ${TIER_LABEL[tier]}`, ariaLabel: `Priority: ${TIER_LABEL[tier]}` }),
    h("div", { class: "main" },
      h("button", {
        class: "title",
        type: "button",
        title: aggregate ? `Open all ${item.members.length} PRs` : `Open ${prLabel(item)}`,
      }, item.title),
      h("span", { class: "meta", title: datesTooltip(item.members, (iso) => DATE_FORMAT.format(new Date(iso))) }, ...meta),
      h("span", { class: "members" }, ticket, ...members),
    ),
    h("div", { class: "side" },
      ageCell(item),
      // A click between or beside the buttons must not open the row.
      h("span", { class: "actions", onclick: (e: Event) => e.stopPropagation() },
        copy,
        h("button", {
          class: "action",
          type: "button",
          title: hideButton(item).title,
          onclick: () => void act(api.setHidden(item.key, !item.hidden)),
        }, hideButton(item).label),
      ),
    ),
  );
}

/** "Draft ready" beside the PR's chip: a button when the draft has a link, plain text when it has none. */
function draftChip(m: Member, rowKey: string): HTMLElement | null {
  const marker = draftMarker(m);
  if (!marker) return null;
  if (!marker.linked) return h("span", { class: "agent-draft", title: marker.title }, "Draft ready");
  const open: HTMLButtonElement = h("button", {
    class: "member agent-draft",
    type: "button",
    title: marker.title,
    onclick: (e: MouseEvent) => {
      e.stopPropagation();
      api.openDraft(m.id).then(() => opened(rowKey), (error: unknown) => {
        open.title = `${marker.title}\nCouldn't open it: ${String(error)}`;
        flash(open, "Failed");
      });
    },
  }, "Draft ready");
  return open;
}

const DATE_FORMAT = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });

/** When the row's earliest PR opened, then its last activity. */
function ageCell(item: ReviewItem): HTMLElement {
  const { opened, active } = ageColumn(item.createdAt, item.updatedAt, new Date());
  return h("span", { class: "age" },
    h("span", {}, opened),
    active && h("span", {}, active),
  );
}

function flash(button: HTMLButtonElement, text: string): void {
  // A second click during the note would save the note as the label and end the note early.
  if (button.dataset.flash) return;
  const label = button.textContent;
  button.textContent = text;
  button.dataset.flash = "true";
  setTimeout(() => {
    button.textContent = label;
    delete button.dataset.flash;
  }, 1500);
}

/** One save queue for the window's lifetime: Settings reopened while a save runs must queue behind it, not beside it. */
let settingsSaver: LastingSaver<Settings> | null = null;

function renderSettings(
  panel: HTMLElement,
  snapshot: Snapshot,
  startTab: SettingsTab,
  onSelect: (key: SettingsTab) => void,
  showFailure: (error: string) => void,
): { update: (s: Snapshot) => void; select: (key: SettingsTab) => void } {
  const saver = (settingsSaver ??= lastingSaver(snapshot.settings, api.saveSettings));
  // This saver is the only writer of these settings, and the snapshot can lag a save just made.
  const settings = { ...saver.latest };
  // Each field reports its own save next to its caption; errors stay until the next attempt. Notes sit inside
  // <label>s, so every input carries an explicit ariaLabel to keep the note out of its accessible name.
  const note = () => h("span", { class: "note", role: "status" });
  const timers = new Map<HTMLElement, number>();
  const show = (n: HTMLElement, kind: "ok" | "error", text: string) => {
    clearTimeout(timers.get(n));
    n.dataset.kind = kind;
    n.textContent = text;
    if (kind === "ok") timers.set(n, setTimeout(() => (n.textContent = ""), 2000));
  };
  const ok = (n: HTMLElement, text = "Saved") => show(n, "ok", text);
  const fail = (n: HTMLElement, e: unknown) => {
    show(n, "error", String(e));
    // A save made on the way out of Settings, or out of a workspace's editor, can fail after its note left the screen.
    // DROPPED would hide the error that caused it.
    if (n.offsetParent === null && e !== DROPPED) showFailure(String(e).startsWith("Not saved") ? String(e) : `Not saved: ${String(e)}`);
  };
  // General and Counter share `settings` and save it whole. A failed save puts both tabs back to the last saved
  // values in place; a General field that has focus is put back when it loses it.
  saver.revert = (saved) => {
    Object.assign(settings, saved);
    generalShows.forEach((show) => show());
    counter.redraw();
  };
  const save = (n: HTMLElement) => saver.save(settings).then(() => ok(n), (e) => fail(n, e));
  const generalShows: (() => void)[] = [];
  /** Re-shows `input` from `settings` after a rollback; while it has focus, once it loses it. */
  const shown = (input: HTMLInputElement, show: () => void) => generalShows.push(() => whenBlurred(input, show));
  const caption = (text: string, n: HTMLElement) => h("span", { class: "caption" }, text, n);
  const n = { minutes: note(), text: note(), width: note(), height: note(), theme: note(), custom: note(), browser: note(), notify: note(), autostart: note(), ink: note() };
  const workspaces = workspacesPanel(snapshot, { note, ok, fail, caption });

  // Not in `settings`, so a rollback leaves it as it is.
  const autostart = h("input", { type: "checkbox", ariaLabel: "Start with Windows", onchange: () => api.setAutostart(autostart.checked).then(() => ok(n.autostart), (e) => fail(n.autostart, e)) });
  void api.autostart().then((on) => (autostart.checked = on));
  const startWithWindows = h("label", { class: "check" }, autostart, "Start with Windows", n.autostart);

  // Chromium writes its profile files a few seconds after Load unpacked,
  // so after Set up the status is re-read on focus and every few seconds while it is on screen, until it is loaded.
  const extension = h("div", { class: "extension", role: "status" });
  let extensionRun = 0;
  let settingUp = false;
  const checkExtension = () => {
    const browser = settings.browser;
    const run = ++extensionRun;
    if (browser === "system") return extension.replaceChildren();
    api.extensionStatus(browser).then((s) => {
      if (run !== extensionRun) return;
      if (s.state === "loaded") settingUp = false;
      const { text, steps } = extensionMessage(s, browser);
      const setUp = h("button", { type: "button", onclick: () => {
        settingUp = true;
        api.setUpExtension(browser).catch((e) => fail(n.browser, e));
      } }, "Set up");
      extension.replaceChildren(h("span", { class: "inline" }, text, steps && setUp));
      if (steps) extension.append(h("span", { class: "steps" }, steps));
    }, (e) => {
      if (run === extensionRun) extension.replaceChildren(String(e));
    });
  };
  // The token stays in Rust: Settings shows dots, and Copy config puts the device's file on the clipboard.
  const ink = h("div", { class: "ink" });
  let inkShown = "";
  const showInk = (s: InkStatus) => {
    // Re-drawn only on a change, so a re-read during a click keeps its button.
    if (JSON.stringify(s) === inkShown) return;
    inkShown = JSON.stringify(s);
    const button = (text: string, onclick: () => void) => h("button", { type: "button", onclick }, text);
    const act = (call: () => Promise<InkStatus>, done: string) => call().then((next) => {
      showInk(next);
      ok(n.ink, done);
    }, (e) => fail(n.ink, e));
    ink.replaceChildren(
      s.paired
        ? h("span", { class: "inline" },
          h("span", { class: "token" }, "•••••••••••••••• saved"),
          button("Copy config", () => api.copyInkConfig().then(() => ok(n.ink, "Copied"), (e) => fail(n.ink, e))),
          button("Replace token", () => act(api.pairInk, "Replaced")),
          button("Remove", () => act(api.unpairInk, "Removed")))
        : h("span", { class: "inline" }, button("Pair a device", () => act(api.pairInk, "Paired"))),
      h("span", {}, s.address ? `Laptop address: ${s.address}` : "Laptop address: none, not on Wi-Fi"),
      h("span", { class: "problem", hidden: !s.problem }, s.problem),
    );
  };
  const checkInk = () => api.inkStatus().then(showInk, (e) => fail(n.ink, e));
  checkInk();

  const recheck = () => {
    if (!extension.isConnected) {
      window.removeEventListener("focus", recheck);
      clearInterval(extensionTimer);
      return;
    }
    if (settingUp && !document.hidden && extension.offsetParent !== null) checkExtension();
    if (!document.hidden && ink.offsetParent !== null) checkInk();
  };
  window.addEventListener("focus", recheck);
  const extensionTimer = setInterval(recheck, 3000);

  /** The General tab's fields, filled from `settings`; built once, and re-shown through `generalShows`. */
  const generalFields = (): HTMLElement[] => {
    const minutes = h("input", { type: "number", min: String(POLL_MINUTES[0]), max: String(POLL_MINUTES[1]), value: String(settings.pollMinutes), class: "minutes", ariaLabel: "Minutes between checks" });
    const chips = POLL_CHOICES.map((m) =>
      h("button", { type: "button", class: "chip", data: { on: String(m === settings.pollMinutes) }, onclick: () => setMinutes(m) }, `${m} min`),
    );
    const setMinutes = (m: number) => {
      settings.pollMinutes = m;
      minutes.value = String(m);
      chips.forEach((c, i) => (c.dataset.on = String(POLL_CHOICES[i] === m)));
      void save(n.minutes);
    };
    shown(minutes, () => {
      minutes.value = String(settings.pollMinutes);
      chips.forEach((c, i) => (c.dataset.on = String(POLL_CHOICES[i] === settings.pollMinutes)));
    });
    minutes.onchange = () => {
      const m = parseMinutes(minutes.value);
      if (m === null) fail(n.minutes, `Use a whole number from ${POLL_MINUTES[0]} to ${POLL_MINUTES[1]}.`);
      else setMinutes(m);
    };

    const textScale = h("input", { type: "number", min: String(TEXT_SCALE[0]), max: String(TEXT_SCALE[1]), value: String(settings.textScale), class: "scale", ariaLabel: "Text size in percent" });
    const textChips = TEXT_CHOICES.map((p) =>
      h("button", { type: "button", class: "chip", onclick: () => setTextScale(p) }, `${p}%`),
    );
    const textSmaller = h("button", { type: "button", title: "Smaller text", onclick: () => setTextScale(stepScale(settings.textScale, -1, TEXT_STEP, TEXT_SCALE)) }, "−");
    const textBigger = h("button", { type: "button", title: "Bigger text", onclick: () => setTextScale(stepScale(settings.textScale, 1, TEXT_STEP, TEXT_SCALE)) }, "+");
    const showTextScale = (percent: number) => {
      textScale.value = String(percent);
      textChips.forEach((c, i) => (c.dataset.on = String(TEXT_CHOICES[i] === percent)));
      textSmaller.disabled = percent <= TEXT_SCALE[0];
      textBigger.disabled = percent >= TEXT_SCALE[1];
    };
    const setTextScale = (percent: number) => {
      settings.textScale = percent;
      showTextScale(percent);
      void save(n.text);
    };
    showTextScale(settings.textScale);
    shown(textScale, () => showTextScale(settings.textScale));
    textScale.onchange = () => {
      const percent = parseWhole(textScale.value, TEXT_SCALE);
      if (percent === null) fail(n.text, `Use a whole number from ${TEXT_SCALE[0]} to ${TEXT_SCALE[1]}.`);
      else setTextScale(percent);
    };

    /** A list side as a percent of the screen: chips, or a typed whole number. */
    const listSide = (key: "listWidthPercent" | "listHeightPercent", label: string, range: readonly [number, number], choices: [string, number][], msg: HTMLElement) => {
      const input = h("input", { type: "number", min: String(range[0]), max: String(range[1]), value: String(settings[key]), class: "scale", ariaLabel: `${label} in percent of the screen` });
      const chips = choices.map(([text, p]) => h("button", { type: "button", class: "chip", title: `${p}% of the screen`, onclick: () => set(p) }, text));
      const showValue = (p: number) => {
        input.value = String(p);
        chips.forEach((c, i) => (c.dataset.on = String(choices[i][1] === p)));
      };
      const set = (p: number) => {
        settings[key] = p;
        showValue(p);
        void save(msg);
      };
      showValue(settings[key]);
      shown(input, () => showValue(settings[key]));
      input.onchange = () => {
        const p = parseWhole(input.value, range);
        if (p === null) fail(msg, `Use a whole number from ${range[0]} to ${range[1]}.`);
        else set(p);
      };
      return h("div", { class: "field" }, caption(`${label} (${range[0]}–${range[1]}% of the screen)`, msg), h("span", { class: "inline" }, ...chips, input, "%"));
    };

    const browsers = BROWSERS.map(([value, label]) =>
      h("button", { type: "button", class: "chip", data: { on: String(value === settings.browser) }, onclick: () => setBrowser(value) }, label),
    );
    const setBrowser = (value: Browser) => {
      settings.browser = value;
      browsers.forEach((c, i) => (c.dataset.on = String(BROWSERS[i][0] === value)));
      checkExtension();
      void save(n.browser);
    };
    checkExtension();
    generalShows.push(() => {
      browsers.forEach((c, i) => (c.dataset.on = String(BROWSERS[i][0] === settings.browser)));
      checkExtension();
    });

    const themes = THEMES.map(([value, label]) =>
      h("button", { type: "button", class: "chip", data: { on: String(value === settings.theme) }, onclick: () => setTheme(value) }, label),
    );
    const setTheme = (value: Theme) => {
      settings.theme = value;
      themes.forEach((c, i) => (c.dataset.on = String(THEMES[i][0] === value)));
      customColours.hidden = value !== "custom";
      void save(n.theme);
    };
    // A copy, so the snapshot keeps the saved colours. The saved snapshot repaints the list.
    settings.customTheme = { ...settings.customTheme };
    const colourRows = CUSTOM_COLOURS.map(([key, label]) => {
      const picker = h("input", { type: "color", value: settings.customTheme[key], ariaLabel: `${label} colour` });
      const typed = h("input", { type: "text", class: "hex", value: settings.customTheme[key], spellcheck: false, ariaLabel: `${label} colour, as hex` });
      const fill = (value: string) => (settings.customTheme[key] = picker.value = typed.value = value);
      shown(typed, () => (picker.value = typed.value = settings.customTheme[key]));
      const set = (value: string) => {
        fill(value);
        saver.save(settings).then(() => {
          const hard = unreadable(settings.customTheme);
          if (hard.length > 0) show(n.custom, "error", `Saved, but hard to read: ${hard.join(", ")}`);
          else ok(n.custom);
        }, (e) => fail(n.custom, e));
      };
      picker.onchange = () => set(picker.value);
      typed.onchange = () => {
        const value = parseHex(typed.value);
        if (value === null) fail(n.custom, "Use a hex colour such as #1f2125.");
        else set(value);
      };
      return h("span", { class: "inline" }, h("span", { class: "colour-name" }, label), picker, typed);
    });
    const customColours = h("div", { class: "field", hidden: settings.theme !== "custom" }, caption("Custom colours", n.custom), ...colourRows);
    generalShows.push(() => {
      themes.forEach((c, i) => (c.dataset.on = String(THEMES[i][0] === settings.theme)));
      customColours.hidden = settings.theme !== "custom";
    });

    const notify = h("input", { type: "checkbox", ariaLabel: "Notify me about new reviews", checked: settings.notify, onchange: () => { settings.notify = notify.checked; void save(n.notify); } });
    generalShows.push(() => (notify.checked = settings.notify));

    return [
      // Not <label>: a label forwards clicks on its caption to its first button (a chip, or −).
      h("div", { class: "field" }, caption("Check every", n.minutes), h("span", { class: "inline" }, ...chips, minutes, "min")),
      h("div", { class: "field" }, caption(`Text size (${TEXT_SCALE[0]}–${TEXT_SCALE[1]}%)`, n.text), h("span", { class: "inline" }, ...textChips, textSmaller, textScale, "%", textBigger)),
      listSide("listWidthPercent", "List width", LIST_WIDTH_PERCENT, LIST_WIDTHS, n.width),
      listSide("listHeightPercent", "List height", LIST_HEIGHT_PERCENT, LIST_HEIGHTS, n.height),
      h("div", { class: "field" }, caption("Theme", n.theme), h("span", { class: "inline" }, ...themes)),
      customColours,
      h("div", { class: "field" }, caption("Open links in", n.browser), h("span", { class: "inline" }, ...browsers), extension),
      h("label", { class: "check" }, notify, "Notify me about new reviews", n.notify),
      startWithWindows,
      h("div", { class: "field" },
        caption("PocketBook", n.ink),
        h("span", { class: "steps" }, "Shows this list on a PocketBook and acts on its rows. Pair, then save the copied config on the PocketBook as system/config/rado.cfg."),
        ink),
    ];
  };
  const counter = counterPanel(settings, { note, caption, fail, save });

  const tabPanel = (key: SettingsTab, ...children: HTMLElement[]) => h("div", { class: "tabpanel", id: `settings-${key}`, role: "tabpanel" }, ...children);
  // Every panel lives for the whole visit, so switching tabs keeps a pasted but unsaved token.
  const panels: Record<SettingsTab, HTMLElement> = {
    workspaces: tabPanel("workspaces", workspaces.element),
    counter: tabPanel("counter", counter.element),
    general: tabPanel("general", ...generalFields()),
  };
  const tabs = SETTINGS_TABS.map((t) => ({ key: t.key, tab: h("button", { class: "tab", id: `settings-tab-${t.key}` }, t.label) }));
  const showTab = (key: SettingsTab) => {
    strip.select(key);
    for (const t of SETTINGS_TABS) panels[t.key].hidden = t.key !== key;
  };
  const strip = tabList("Settings sections", tabs, (key) => panels[key], (key) => {
    showTab(key);
    onSelect(key);
  });
  showTab(startTab);
  panel.replaceChildren(strip.list, ...SETTINGS_TABS.map((t) => panels[t.key]));
  return {
    update: workspaces.update,
    select: (key) => {
      showTab(key);
      onSelect(key);
    },
  };
}
