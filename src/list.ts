import { api, watchSnapshot, type Browser, type CustomTheme, type ReviewItem, type Snapshot, type Theme, type Tier } from "./api";
import { applyTheme, parseHex, unreadable } from "./theme";
import {
  ageColumn, ago, datesTooltip, LIST_HEIGHT_PERCENT, LIST_WIDTH_PERCENT, newestFirst, nextReview, parseWhole, parseMinutes, prLabel, stepScale, TEXT_CHOICES, TEXT_SCALE, TEXT_STEP,
} from "./format";
import { h } from "./dom";
import { counterPanel } from "./counter";
import { serialSaver } from "./save";
import { activeWorkspace, isTyping, stepWorkspace, switchWorkspace, trackerName, workspaceChip, workspaceStep, workspacesPanel } from "./workspaces";
import {
  defaultListTab, inSection, restoreTab, SECTIONS, SETTINGS_TABS, stepTab, type SectionKey, type SettingsTab,
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
  const newerText = h("span");
  const newerLine = h("div", { class: "newer", hidden: true }, newerText, h("button", { type: "button", class: "link", onclick: () => void api.openNewerRelease() }, "Download"));
  const status = h("div", { class: "status" }, statusText, newerLine);
  const body = h("div", { class: "body", id: "list-panel", role: "tabpanel" });
  const settingsPanel = h("div", { class: "settings", hidden: true });
  const heading = h("h1", {}, "Reviews");
  const bird = h("span", { class: "bird", ariaHidden: "true" });
  const refreshButton = h("button", { class: "icon", type: "button", title: "Refresh now", onclick: () => void api.refresh() }, "↻");
  const settingsButton = h("button", { class: "icon", type: "button", title: "Settings", onclick: () => showSettings(settingsPanel.hidden !== false) }, "⚙");
  const closeButton = h("button", { class: "icon", type: "button", title: "Close", onclick: () => void api.hideList() }, "✕");
  const workspace = workspaceChip(() => showSettings(true, "workspaces"));

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
    h("header", {}, bird, heading, workspace.chip, refreshButton, settingsButton, closeButton, workspace.picker),
    status,
    strip.list,
    settingsPanel,
    body,
  );
  strip.select(tab ?? "needsReview");

  // Settings replace the list rather than squeezing it; hiding the list keeps its rows and scroll position.
  let listScroll = 0;
  let settingsTab: SettingsTab = restoreTab(recall(SETTINGS_TAB_KEY), SETTINGS_TABS.map((t) => t.key)) ?? "workspaces";
  let settingsView: { update: (s: Snapshot) => void; select: (key: SettingsTab) => void } | null = null;
  const showSettings = (on: boolean, startTab: SettingsTab = settingsTab) => {
    if (on && !settingsPanel.hidden) settingsView?.select(startTab);
    if (on === !settingsPanel.hidden) return;
    if (on) listScroll = body.scrollTop;
    settingsPanel.hidden = !on;
    body.hidden = on;
    strip.list.hidden = on;
    heading.textContent = on ? "Settings" : "Reviews";
    settingsButton.textContent = on ? "←" : "⚙";
    settingsButton.title = on ? "Back to reviews" : "Settings";
    settingsView = on && snapshot
      ? renderSettings(settingsPanel, snapshot, startTab, (key) => {
          settingsTab = key;
          remember(SETTINGS_TAB_KEY, key);
        })
      : null;
    if (!on) {
      body.scrollTop = listScroll;
      revealTab();
    }
  };
  // Larger text can push the selected tab past the strip's edge; a hidden strip can't scroll, so this also runs on return from settings.
  const revealTab = () => strip.list.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest", inline: "nearest" });

  let snapshot: Snapshot | null = null;
  let pollingSince: number | null = null;

  const renderStatus = () => {
    if (!snapshot) return;
    statusText.replaceChildren();
    status.dataset.kind = snapshot.polling ? "busy" : snapshot.error ? "error" : "ok";
    if (snapshot.polling) {
      const seconds = pollingSince ? Math.round((Date.now() - pollingSince) / 1000) : 0;
      statusText.append(h("span", { class: "spinner" }), `Refreshing… usually 1–3 s (${seconds} s)`);
    } else if (snapshot.error) {
      statusText.append(snapshot.error);
    } else {
      statusText.append(`Refreshed ${ago(snapshot.lastRefreshed, new Date())} · every ${snapshot.settings.pollMinutes} min`);
    }
    if (snapshot.warning) statusText.append(h("div", { class: "hint" }, snapshot.warning));
  };

  const renderBody = () => {
    if (!snapshot) return;
    body.replaceChildren();
    if (!snapshot.hasPat && !snapshot.hasGithubToken) {
      if (settingsPanel.hidden) showSettings(true, "workspaces");
      return;
    }
    if (!snapshot.lastRefreshed) {
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
    if (items.length === 0) body.append(h("p", { class: "empty" }, "Nothing here."));
    const tracker = trackerName(activeWorkspace(snapshot.settings));
    for (const item of items) body.append(row(item, tracker, item.key === next?.key));
  };

  void watchSnapshot((s) => {
    if (s.polling && !snapshot?.polling) pollingSince = Date.now();
    const rescaled = s.settings.textScale !== snapshot?.settings.textScale;
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
  });
  setInterval(renderStatus, 1000);
  // Rows opened since the last render change which review is next.
  window.addEventListener("focus", renderBody);
  window.addEventListener("keydown", (e) => {
    const step = workspaceStep(e);
    if (step !== null && snapshot && !isTyping(e.target)) {
      e.preventDefault();
      const to = stepWorkspace(snapshot.settings.workspaces, snapshot.settings.activeWorkspace, step);
      if (to) switchWorkspace(to);
      return;
    }
    if (e.key !== "Escape") return;
    if (workspace.close()) workspace.chip.focus();
    else if (settingsPanel.hidden) void api.hideList();
    else showSettings(false);
  });
}

const TIER_LABEL: Record<Tier, string> = { urgent: "Urgent", normal: "Normal", low: "Low" };

function row(item: ReviewItem, tracker: string, isNext: boolean): HTMLElement {
  const aggregate = item.members.length > 1;
  // Plain text, so the outlined chips are exactly the things that open something.
  const facts = [
    h("span", {}, aggregate ? `${item.members.length} PRs · ${item.branch} · ${item.author}` : item.author),
    item.required && h("span", { class: "required" }, "required"),
    item.approvals > 0 && h("span", { class: "approved", title: `${item.approvals} approved` }, `✓ ${item.approvals}`),
    item.objections > 0 && h("span", { class: "objected", title: `${item.objections} rejected or waiting for author` }, `✗ ${item.objections}`),
    item.comments && item.comments.total > 0 && h("span", {}, `${item.comments.total} comment${item.comments.total === 1 ? "" : "s"}${item.comments.openThreads > 0 ? ` (${item.comments.openThreads} open)` : ""}`),
    item.draft && h("span", {}, "draft"),
    item.releaseTarget && h("span", { title: `Merges into ${item.target}` }, `→ ${item.target}`),
  ].filter((f): f is HTMLSpanElement => Boolean(f));
  const meta = facts.flatMap((f, i) => (i === 0 ? [f] : [" · ", f]));
  const members = item.members.map((m) => h("button", {
      class: "member",
      type: "button",
      title: m.state ? `Open ${prLabel(m)}: ${m.title}` : `Open ${prLabel(m)} (same branch, not assigned to you): ${m.title}`,
      data: { assigned: String(m.state !== null) },
      onclick: (e: MouseEvent) => {
        e.stopPropagation();
        opened(item.key);
        void api.openPr(m.id);
      },
    }, prLabel(m)));
  const ticket = item.ticket && h("button", {
    class: "member ticket",
    type: "button",
    title: `Open ${item.ticket} in ${tracker}`,
    onclick: (e: MouseEvent) => {
      e.stopPropagation();
      void api.openTicket(item.key);
    },
  }, item.ticket);
  const copy: HTMLButtonElement = h("button", {
    class: "action",
    type: "button",
    title: aggregate ? "Copy all links, one 'repo: link' per line" : "Copy the link",
    onclick: () => api.copyRow(item.key).then(() => flash(copy, "Copied"), () => flash(copy, "Failed")),
  }, "Copy");
  const tier = item.hidden ? "low" : item.tier;
  // The title button is the keyboard target for opening the row: its click bubbles to this div, as does a click
  // anywhere else on the row. The chips are buttons of their own beside it and stop their clicks here.
  return h("div", { class: "row", data: { tier, next: String(isNext) } },
    h("div", {
      class: "open",
      onclick: () => {
        opened(item.key);
        void api.openRow(item.key);
      },
    },
      h("span", { class: "dot", role: "img", title: `Priority: ${TIER_LABEL[tier]}`, ariaLabel: `Priority: ${TIER_LABEL[tier]}` }),
      h("div", { class: "main" },
        isNext && h("span", { class: "next-label" }, "Review next"),
        h("button", {
          class: "title",
          type: "button",
          title: aggregate ? `Open all ${item.members.length} PRs` : `Open ${prLabel(item)}`,
        }, item.title),
        h("span", { class: "meta" }, ...meta),
        h("span", { class: "members" }, ticket, ...members),
      ),
      ageCell(item),
    ),
    h("span", { class: "actions" },
      copy,
      h("button", {
        class: "action",
        type: "button",
        title: item.hidden ? "Show again" : item.state === "mine" ? "Hide until it closes" : "Not yours: leave the count until it closes",
        onclick: () => void api.setHidden(item.key, !item.hidden),
      }, item.hidden ? "Show" : item.state === "mine" ? "Hide" : "Not mine"),
    ),
  );
}

const DATE_FORMAT = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });

/** When the row's earliest PR opened, then its last activity. Hover shows each PR's exact times. */
function ageCell(item: ReviewItem): HTMLElement {
  const { opened, active } = ageColumn(item.createdAt, item.updatedAt, new Date());
  return h("span", { class: "age", title: datesTooltip(item.members, (iso) => DATE_FORMAT.format(new Date(iso))) },
    h("span", {}, opened),
    active && h("span", {}, active),
  );
}

function flash(button: HTMLButtonElement, text: string): void {
  const label = button.textContent;
  button.textContent = text;
  button.dataset.flash = "true";
  setTimeout(() => {
    button.textContent = label;
    delete button.dataset.flash;
  }, 1500);
}

function renderSettings(
  panel: HTMLElement,
  snapshot: Snapshot,
  startTab: SettingsTab,
  onSelect: (key: SettingsTab) => void,
): { update: (s: Snapshot) => void; select: (key: SettingsTab) => void } {
  const settings = { ...snapshot.settings };
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
  const fail = (n: HTMLElement, e: unknown) => show(n, "error", String(e));
  // General and Counter share `settings` and save it whole. A failed save rolls both tabs back to the last saved
  // values (the field notes outlive the rebuild, so the error stays shown).
  const queueSave = serialSaver(settings, api.saveSettings, (saved) => {
    Object.assign(settings, saved);
    panels.general.replaceChildren(...generalFields());
    counter.redraw();
  });
  const save = (n: HTMLElement) => queueSave(settings).then(() => ok(n), (e) => fail(n, e));
  const caption = (text: string, n: HTMLElement) => h("span", { class: "caption" }, text, n);
  const n = { minutes: note(), text: note(), width: note(), height: note(), theme: note(), custom: note(), browser: note(), notify: note(), autostart: note() };
  const workspaces = workspacesPanel(snapshot, { note, ok, fail, caption });

  // Not in `settings`, so a rollback's rebuild reuses it rather than reading it again.
  const autostart = h("input", { type: "checkbox", ariaLabel: "Start with Windows", onchange: () => api.setAutostart(autostart.checked).then(() => ok(n.autostart), (e) => fail(n.autostart, e)) });
  void api.autostart().then((on) => (autostart.checked = on));
  const startWithWindows = h("label", { class: "check" }, autostart, "Start with Windows", n.autostart);

  /** The General tab's fields, filled from `settings`. */
  const generalFields = (): HTMLElement[] => {
    const minutes = h("input", { type: "number", min: "1", max: "120", value: String(settings.pollMinutes), class: "minutes", ariaLabel: "Minutes between checks" });
    const chips = POLL_CHOICES.map((m) =>
      h("button", { type: "button", class: "chip", data: { on: String(m === settings.pollMinutes) }, onclick: () => setMinutes(m) }, `${m} min`),
    );
    const setMinutes = (m: number) => {
      settings.pollMinutes = m;
      minutes.value = String(m);
      chips.forEach((c, i) => (c.dataset.on = String(POLL_CHOICES[i] === m)));
      void save(n.minutes);
    };
    minutes.onchange = () => {
      const m = parseMinutes(minutes.value);
      if (m === null) fail(n.minutes, "Use a whole number from 1 to 120.");
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
      void save(n.browser);
    };

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
      const set = (value: string) => {
        fill(value);
        queueSave(settings).then(() => {
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

    const notify = h("input", { type: "checkbox", ariaLabel: "Notify me about new reviews", checked: settings.notify, onchange: () => { settings.notify = notify.checked; void save(n.notify); } });

    return [
      // Not <label>: a label forwards clicks on its caption to its first button (a chip, or −).
      h("div", { class: "field" }, caption("Check every", n.minutes), h("span", { class: "inline" }, ...chips, minutes, "min")),
      h("div", { class: "field" }, caption(`Text size (${TEXT_SCALE[0]}–${TEXT_SCALE[1]}%)`, n.text), h("span", { class: "inline" }, ...textChips, textSmaller, textScale, "%", textBigger)),
      listSide("listWidthPercent", "List width", LIST_WIDTH_PERCENT, LIST_WIDTHS, n.width),
      listSide("listHeightPercent", "List height", LIST_HEIGHT_PERCENT, LIST_HEIGHTS, n.height),
      h("div", { class: "field" }, caption("Theme", n.theme), h("span", { class: "inline" }, ...themes)),
      customColours,
      h("div", { class: "field" }, caption("Open links in", n.browser), h("span", { class: "inline" }, ...browsers)),
      h("label", { class: "check" }, notify, "Notify me about new reviews", n.notify),
      startWithWindows,
      h("div", { class: "end" }, h("button", { type: "button", onclick: () => void api.quit() }, "Quit")),
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
