import { api, type Saved, type Settings, type Snapshot, type Tracker, type Workspace } from "./api";
import { h, whenBlurred } from "./dom";
import { lastingSaver, secretSaver, type LastingSaver } from "./save";

/** One save queue per workspace for the window's lifetime: an editor reopened while a save runs queues behind it. */
const workspaceSavers = new Map<string, LastingSaver<Workspace>>();

/** The save queue for `workspace`'s fields, made on first use from the workspace as given. */
export function workspaceSaver(workspace: Workspace): LastingSaver<Workspace> {
  let saver = workspaceSavers.get(workspace.id);
  if (!saver) workspaceSavers.set(workspace.id, (saver = lastingSaver(workspace, api.saveWorkspace)));
  return saver;
}

export const activeWorkspace = (settings: Settings): Workspace | undefined =>
  settings.workspaces.find((w) => w.id === settings.activeWorkspace);

type TrackerKind = Tracker["kind"];

/** The chooser in the workspace editor: each tracker's name and its one field. */
export const TRACKERS: readonly { kind: TrackerKind; name: string; field: string; placeholder: string }[] = [
  { kind: "none", name: "None", field: "", placeholder: "" },
  { kind: "jira", name: "Jira", field: "Jira Cloud site", placeholder: "yourcompany.atlassian.net" },
  { kind: "linear", name: "Linear", field: "Linear workspace", placeholder: "yourteam, as in linear.app/yourteam" },
];

/** The name ticket chips are titled with ("Open ABC-12 in Linear"); empty without a tracker. */
export const trackerName = (workspace: Workspace | undefined): string => {
  const kind = workspace?.tracker.kind ?? "none";
  return kind === "none" ? "" : TRACKERS.find((t) => t.kind === kind)?.name ?? "";
};

/** The tracker's one field: the Jira site or the Linear workspace. */
export const trackerValue = (tracker: Tracker): string =>
  tracker.kind === "jira" ? tracker.site : tracker.kind === "linear" ? tracker.workspace : "";

export const trackerKeys = (tracker: Tracker): string[] => (tracker.kind === "none" ? [] : tracker.keys);

/** A tracker of `kind` with `value` as its field and `keys` as its team keys; an empty field is no tracker. */
export function trackerFor(kind: TrackerKind, value: string, keys: string[]): Tracker {
  const v = value.trim();
  if (!v || kind === "none") return { kind: "none" };
  return kind === "jira" ? { kind, site: v, keys } : { kind, workspace: v, keys };
}

export const parseList = (text: string): string[] => text.split(",").map((t) => t.trim()).filter(Boolean);

/** Ctrl+← and Ctrl+→ step through workspaces; any other key, or another modifier with them, is not a step. */
export function workspaceStep(e: Pick<KeyboardEvent, "key" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey">): -1 | 1 | null {
  if (!e.ctrlKey || e.altKey || e.shiftKey || e.metaKey) return null;
  return e.key === "ArrowLeft" ? -1 : e.key === "ArrowRight" ? 1 : null;
}

/** The workspace `delta` places from the active one, or null at either end: like desktops, the order doesn't wrap. */
export function stepWorkspace(workspaces: readonly Workspace[], active: string, delta: -1 | 1): string | null {
  const at = workspaces.findIndex((w) => w.id === active);
  const to = at + delta;
  return at < 0 || to < 0 || to >= workspaces.length ? null : workspaces[to].id;
}

/** Where Ctrl+arrow means "move by word" and must stay that. */
export function isTyping(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName));
}

const KEYS_HINT = "Ctrl+← / Ctrl+→ switch workspaces";

/** The header chip naming the active workspace, and the picker it opens. */
export function workspaceChip(onManage: () => void, switchTo: (id: string) => void): { chip: HTMLButtonElement; picker: HTMLElement; render: (s: Snapshot) => void; close: () => boolean } {
  const label = h("span", { class: "ws-name" });
  const chip = h("button", { class: "ws-chip", type: "button", ariaHasPopup: "menu", ariaExpanded: "false" }, label, h("span", { ariaHidden: "true" }, "▾"));
  const picker = h("div", { class: "ws-picker", role: "menu", hidden: true });
  let snapshot: Snapshot | null = null;

  const items = () => [...picker.querySelectorAll<HTMLButtonElement>("button")];
  const open = () => {
    if (!snapshot) return;
    const { workspaces, activeWorkspace: active } = snapshot.settings;
    picker.replaceChildren(
      ...workspaces.map((w) => h("button", {
        type: "button",
        role: "menuitemradio",
        ariaChecked: String(w.id === active),
        data: { id: w.id },
        onclick: () => {
          close();
          if (w.id !== snapshot?.settings.activeWorkspace) switchTo(w.id);
        },
      }, w.name)),
      h("button", { type: "button", role: "menuitem", class: "manage", onclick: () => { close(); onManage(); } }, "Manage workspaces…"),
    );
    picker.hidden = false;
    chip.ariaExpanded = "true";
    (items().find((b) => b.ariaChecked === "true") ?? items()[0]).focus();
  };
  const close = () => {
    if (picker.hidden) return false;
    picker.hidden = true;
    chip.ariaExpanded = "false";
    return true;
  };
  chip.onclick = () => (picker.hidden ? open() : close());
  picker.addEventListener("keydown", (e) => {
    const list = items();
    const at = list.indexOf(document.activeElement as HTMLButtonElement);
    const to = e.key === "ArrowDown" ? (at + 1) % list.length : e.key === "ArrowUp" ? (at - 1 + list.length) % list.length : null;
    if (to === null) return;
    e.preventDefault();
    list[to].focus();
  });
  document.addEventListener("mousedown", (e) => {
    if (!picker.hidden && !picker.contains(e.target as Node) && !chip.contains(e.target as Node)) close();
  });

  const render = (s: Snapshot) => {
    snapshot = s;
    const name = activeWorkspace(s.settings)?.name ?? "";
    label.textContent = name;
    chip.title = `Workspace: ${name} · ${KEYS_HINT}`;
    // An open picker keeps its items and focus; only the check mark follows a switch made with the keys.
    for (const b of items()) if (b.dataset.id) b.ariaChecked = String(b.dataset.id === s.settings.activeWorkspace);
  };
  return { chip, picker, render, close };
}

/** The field notes `renderSettings` owns, so saves report the same way on every tab. */
export interface Notes {
  note: () => HTMLElement;
  ok: (n: HTMLElement, text?: string) => void;
  fail: (n: HTMLElement, e: unknown) => void;
  caption: (text: string, n: HTMLElement) => HTMLElement;
}

const SAVED = "•••••••••••••••• saved";

/** The Workspaces settings tab: the list with add and set active, and an editor for the selected workspace. */
export function workspacesPanel(first: Snapshot, notes: Notes): { element: HTMLElement; update: (s: Snapshot) => void } {
  const { note, ok, fail, caption } = notes;
  let snapshot = first;
  let selected = first.settings.activeWorkspace;
  let editing: string | null = null;

  const listNote = note();
  const list = h("div", { class: "ws-list" });
  const add = h("button", {
    type: "button",
    onclick: () => api.addWorkspace("").then((id) => {
      selected = id;
      return refresh().then(() => editor.querySelector<HTMLInputElement>("input")?.select());
    }, (e) => fail(listNote, e)),
  }, "+ Add workspace");
  const editor = h("div", { class: "ws-editor" });
  const element = h("div", { class: "ws-panel" },
    h("div", { class: "field" }, caption("Workspaces", listNote), list, h("span", { class: "inline" }, add)),
    editor,
  );
  const refresh = () => api.snapshot().then(update);

  const renderList = () => {
    const active = snapshot.settings.activeWorkspace;
    list.replaceChildren(...snapshot.settings.workspaces.map((w) => h("button", {
      type: "button",
      class: "ws-item",
      ariaPressed: String(w.id === selected),
      onclick: () => {
        selected = w.id;
        update(snapshot);
      },
    }, h("span", {}, w.name), w.id === active && h("span", { class: "ws-active" }, "active"))));
  };

  let activeSlot: HTMLElement | null = null;
  const renderActive = (workspace: Workspace) => {
    if (!activeSlot) return;
    const isActive = workspace.id === snapshot.settings.activeWorkspace;
    activeSlot.replaceChildren(isActive
      ? h("span", { class: "ws-active" }, "Active: polled and counted now")
      : h("button", { type: "button", onclick: () => api.switchWorkspace(workspace.id).catch((e) => fail(listNote, e)) }, "Make active"));
  };

  const buildEditor = (original: Workspace, saved: Saved) => {
    const saver = workspaceSaver(original);
    // The saver is the only writer of these fields, and the snapshot can lag a save just made.
    const workspace: Workspace = structuredClone(saver.latest);
    const n = { name: note(), pat: note(), org: note(), teams: note(), tracker: note(), keys: note(), token: note(), repos: note() };
    const save = (target: HTMLElement, done?: string) => saver.save(workspace).then(() => ok(target, done), (e) => fail(target, e));

    const name = h("input", {
      type: "text", ariaLabel: "Workspace name", value: workspace.name,
      onchange: () => {
        if (!name.value.trim()) return fail(n.name, "A workspace needs a name.");
        workspace.name = name.value.trim();
        void save(n.name);
      },
    });
    const secret = (
      label: string, placeholder: string, isSaved: boolean, target: HTMLElement,
      store: (value: string) => Promise<void>, remove: () => Promise<void>,
    ) => {
      const input = h("input", { type: "password", autocomplete: "off", ariaLabel: label, placeholder: isSaved ? SAVED : placeholder });
      const removeSlot = h("span", { class: "inline" });
      const askRemove = (saved: boolean) => removeSlot.replaceChildren(...(saved
        ? [h("button", { type: "button", onclick: confirmRemove }, "Remove…")]
        : []));
      const confirmRemove = () => removeSlot.replaceChildren(
        `Remove the saved ${label}?`,
        h("button", {
          type: "button",
          class: "danger",
          onclick: () => remove().then(() => {
            input.placeholder = placeholder;
            askRemove(false);
            ok(target, "Removed from Windows Credential Manager");
          }, (e) => fail(target, e)),
        }, "Remove"),
        h("button", { type: "button", onclick: () => askRemove(true) }, "Cancel"),
      );
      askRemove(isSaved);
      const save = secretSaver(store);
      const commit = () => {
        const value = input.value;
        save(value).then((saved) => {
          if (!saved) return;
          if (input.value === value) input.value = "";
          input.placeholder = SAVED;
          askRemove(true);
          ok(target, "Saved to Windows Credential Manager");
        }, (e) => fail(target, e));
      };
      // Pressing Save leaves the field, which may have saved and emptied it before the click.
      const commitOrAsk = () => {
        if (input.value.trim()) commit();
        else if (input.placeholder !== SAVED) fail(target, `Paste the ${label} first`);
      };
      input.onchange = commit;
      // Change fires only for a new value, so Enter after a failed save retries here.
      input.onkeydown = (e) => {
        if (e.key === "Enter") commitOrAsk();
      };
      const button = h("button", { type: "button", onclick: commitOrAsk }, "Save");
      return h("span", { class: "inline" }, input, button, removeSlot);
    };
    const org = h("input", {
      type: "text", ariaLabel: "Organization", value: workspace.azureDevOps.organization, placeholder: "yourorg, as in dev.azure.com/yourorg",
      onchange: () => { workspace.azureDevOps.organization = org.value.trim(); void save(n.org); },
    });
    const repos = h("input", {
      type: "text", ariaLabel: "Watched repos", value: workspace.github.repos.join(", "), placeholder: "owner/name, as in github.com/owner/name",
      onchange: () => { workspace.github = { repos: parseList(repos.value) }; void save(n.repos); },
    });
    const teams = h("input", {
      type: "text", ariaLabel: "My teams", value: workspace.azureDevOps.teams.join(", "), placeholder: "TeamAlpha",
      onchange: () => { workspace.azureDevOps.teams = parseList(teams.value); void save(n.teams); },
    });
    // The chosen kind stays on screen while its field is still empty, though until then the saved tracker is none.
    let kind = workspace.tracker.kind;
    let keys = trackerKeys(workspace.tracker);
    const chosen = () => TRACKERS.find((t) => t.kind === kind) ?? TRACKERS[0];
    const saveTracker = (target: HTMLElement) => {
      workspace.tracker = trackerFor(kind, trackerField.value, keys);
      const held = workspace.tracker.kind === "none" && keys.length > 0;
      void save(target, held ? `Not kept until the ${chosen().field} is set` : undefined);
    };
    const trackerField = h("input", { type: "text", value: trackerValue(workspace.tracker), onchange: () => saveTracker(n.tracker) });
    const keysField = h("input", {
      type: "text", ariaLabel: "Team keys", value: keys.join(", "), placeholder: "HOO, ENG",
      onchange: () => { keys = parseList(keysField.value); saveTracker(n.keys); },
    });
    const keysLabel = h("label", {}, caption("Team keys (comma-separated)", n.keys), keysField,
      h("span", { class: "hint" }, "Set these to stop chips on branches like sprint-3"));
    const trackerChips = TRACKERS.map((t) => h("button", {
      type: "button",
      class: "chip",
      onclick: () => {
        if (t.kind === kind) return;
        kind = t.kind;
        trackerField.value = "";
        showTracker();
        workspace.tracker = { kind: "none" };
        void save(n.tracker);
        if (kind !== "none") trackerField.focus();
      },
    }, t.name));
    const showTracker = () => {
      trackerChips.forEach((c, i) => (c.dataset.on = String(TRACKERS[i].kind === kind)));
      trackerField.hidden = kind === "none";
      keysLabel.hidden = kind === "none";
      trackerField.ariaLabel = chosen().field;
      trackerField.placeholder = chosen().placeholder;
    };
    showTracker();
    // A failed save puts every field back to the last saved values; a text field with focus once it loses it.
    saver.revert = (last) => {
      const trackerBefore = JSON.stringify(workspace.tracker);
      Object.assign(workspace, last);
      whenBlurred(name, () => (name.value = workspace.name));
      whenBlurred(org, () => (org.value = workspace.azureDevOps.organization));
      whenBlurred(teams, () => (teams.value = workspace.azureDevOps.teams.join(", ")));
      whenBlurred(repos, () => (repos.value = workspace.github.repos.join(", ")));
      // The tracker goes back only when it changed, so a chosen kind with its field still empty survives another
      // field's failure. It goes back at once and whole, even mid-typing: any part left over would be sent again
      // with the next tracker save, or mixed with the restored kind.
      if (JSON.stringify(workspace.tracker) === trackerBefore) return;
      kind = workspace.tracker.kind;
      keys = trackerKeys(workspace.tracker);
      trackerField.value = trackerValue(workspace.tracker);
      keysField.value = keys.join(", ");
      showTracker();
    };

    const removeSlot = h("span", { class: "inline" });
    const askRemove = () => removeSlot.replaceChildren(h("button", { type: "button", onclick: confirmRemove }, "Delete workspace…"));
    const confirmRemove = () => removeSlot.replaceChildren(
      `Delete “${workspace.name}” and its saved tokens?`,
      h("button", {
        type: "button",
        class: "danger",
        // The editor is rebuilt when the workspace goes, so the outcome is reported beside the list.
        onclick: () => api.deleteWorkspace(workspace.id).then(
          () => ok(listNote, `Deleted “${workspace.name}”`),
          (e) => fail(listNote, e),
        ).then(refresh),
      }, "Delete"),
      h("button", { type: "button", onclick: askRemove }, "Cancel"),
    );
    askRemove();

    activeSlot = h("span", { class: "inline" });
    editor.replaceChildren(
      h("label", {}, caption("Name", n.name), name),
      activeSlot,
      h("h2", {}, "Azure DevOps"),
      h("label", {}, caption("PAT", n.pat), secret("PAT", "Paste a PAT (Code: Read)", saved.pat, n.pat,
        (v) => api.savePat(workspace.id, v), () => api.removePat(workspace.id))),
      h("label", {}, caption("Organization", n.org), org),
      h("label", {}, caption("My teams (comma-separated)", n.teams), teams),
      h("h2", {}, "GitHub"),
      h("label", {}, caption("Token", n.token), secret("GitHub token", "Paste a token (repo, read:org)", saved.githubToken, n.token,
        (v) => api.saveGithubToken(workspace.id, v), () => api.removeGithubToken(workspace.id))),
      h("label", {}, caption("Watched repos (comma-separated)", n.repos), repos,
        h("span", { class: "hint" }, "Every open PR there counts as waiting on your review, except drafts and yours")),
      h("h2", {}, "Tracker"),
      // Not <label>: a label forwards clicks on its caption to its first button.
      h("div", { class: "field" }, caption("Links ticket keys in branch names to their issues", n.tracker), h("span", { class: "inline" }, ...trackerChips, trackerField)),
      keysLabel,
      removeSlot,
    );
    renderActive(workspace);
  };

  // Rebuilt only when the selection changes, so a switch or a save elsewhere keeps a pasted but unsaved token.
  function update(s: Snapshot) {
    snapshot = s;
    const { workspaces, activeWorkspace: active } = s.settings;
    if (!workspaces.some((w) => w.id === selected)) selected = active;
    renderList();
    const workspace = workspaces.find((w) => w.id === selected);
    if (!workspace) return;
    if (editing !== selected) {
      editing = selected;
      buildEditor(workspace, s.saved[workspace.id] ?? { pat: false, githubToken: false });
    } else {
      renderActive(workspace);
    }
  }
  update(first);
  return { element, update };
}
