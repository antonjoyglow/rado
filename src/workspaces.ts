import { api, type Saved, type Settings, type Snapshot, type Tracker, type Workspace } from "./api";
import { h } from "./dom";

export const activeWorkspace = (settings: Settings): Workspace | undefined =>
  settings.workspaces.find((w) => w.id === settings.activeWorkspace);

/** The Jira host ticket chips link to; empty hides them. */
export const jiraSite = (workspace: Workspace | undefined): string =>
  workspace?.tracker.kind === "jira" ? workspace.tracker.site : "";

export const trackerFor = (site: string): Tracker => (site.trim() ? { kind: "jira", site: site.trim() } : { kind: "none" });

export const parseTeams = (text: string): string[] => text.split(",").map((t) => t.trim()).filter(Boolean);

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
export function workspaceChip(onManage: () => void): { chip: HTMLButtonElement; picker: HTMLElement; render: (s: Snapshot) => void; close: () => boolean } {
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
          void api.switchWorkspace(w.id);
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
      : h("button", { type: "button", onclick: () => void api.switchWorkspace(workspace.id) }, "Make active"));
  };

  const buildEditor = (original: Workspace, saved: Saved) => {
    const workspace: Workspace = structuredClone(original);
    const n = { name: note(), pat: note(), org: note(), teams: note(), jira: note(), token: note() };
    const save = (target: HTMLElement) => api.saveWorkspace(workspace).then(() => ok(target), (e) => fail(target, e));

    const name = h("input", {
      type: "text", ariaLabel: "Workspace name", value: workspace.name,
      onchange: () => {
        if (!name.value.trim()) return fail(n.name, "A workspace needs a name.");
        workspace.name = name.value.trim();
        void save(n.name);
      },
    });
    const secret = (label: string, placeholder: string, isSaved: boolean, target: HTMLElement, store: (value: string) => Promise<void>) => {
      const input = h("input", { type: "password", autocomplete: "off", ariaLabel: label, placeholder: isSaved ? SAVED : placeholder });
      const button = h("button", {
        type: "button",
        onclick: () => store(input.value).then(() => {
          input.value = "";
          input.placeholder = SAVED;
          ok(target, "Saved to Windows Credential Manager");
        }, (e) => fail(target, e)),
      }, "Save");
      return h("span", { class: "inline" }, input, button);
    };
    const org = h("input", {
      type: "text", ariaLabel: "Organization", value: workspace.azureDevOps.organization, placeholder: "yourorg, as in dev.azure.com/yourorg",
      onchange: () => { workspace.azureDevOps.organization = org.value.trim(); void save(n.org); },
    });
    const teams = h("input", {
      type: "text", ariaLabel: "My teams", value: workspace.azureDevOps.teams.join(", "), placeholder: "TeamAlpha",
      onchange: () => { workspace.azureDevOps.teams = parseTeams(teams.value); void save(n.teams); },
    });
    const jira = h("input", {
      type: "text", ariaLabel: "Jira Cloud site", value: jiraSite(workspace), placeholder: "yourcompany.atlassian.net",
      onchange: () => { workspace.tracker = trackerFor(jira.value); void save(n.jira); },
    });

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
      h("label", {}, caption("PAT", n.pat), secret("PAT", "Paste a PAT (Code: Read)", saved.pat, n.pat, (v) => api.savePat(workspace.id, v))),
      h("label", {}, caption("Organization", n.org), org),
      h("label", {}, caption("My teams (comma-separated)", n.teams), teams),
      h("h2", {}, "GitHub"),
      h("label", {}, caption("Token", n.token), secret("GitHub token", "Paste a token (repo, read:org)", saved.githubToken, n.token, (v) => api.saveGithubToken(workspace.id, v))),
      h("h2", {}, "Tracker"),
      h("label", {}, caption("Jira Cloud site, for ticket links from branch names", n.jira), jira),
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
