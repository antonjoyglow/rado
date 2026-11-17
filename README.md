<p align="center"><img src="docs/logo.svg" width="128" height="128" alt="Rado's logo: a punk shoebill"></p>

# Rado

Rado is a Windows tray app that shows the pull requests waiting for your review, across Azure DevOps and GitHub. A small counter on your desktop shows how many reviews you have and how many of your own PRs need work. Click it for a ranked list. Workspaces keep separate setups apart, for example your company's Azure DevOps and Jira and a side project's GitHub and Linear.

## Quick setup

1. **Install.** Download `Rado_<version>_x64-setup.exe` from the [latest release](https://github.com/antonjoyglow/rado/releases/latest) and run it. It installs for your user only. The installer isn't code-signed, so Windows SmartScreen may warn you; choose **More info → Run anyway** (not verified on a fresh machine).
2. **Create a token** for each source you use:
   - **Azure DevOps PAT:** **Code (Read)** is the only scope it needs.
   - **GitHub token:** a classic token with `repo` (to read private repos) and `read:org` (to see review requests to your teams). A fine-grained token with *Pull requests: Read* and *Metadata: Read* on your orgs should also work, but some orgs must approve it first (not tested yet).
3. **Fill in your workspace.** The counter shows `!` beside Rado's bird until a token is saved. Click it (or the tray icon), and the list opens on **Settings → Workspaces**, where a workspace named *Default* is ready. Give it a name, then:
   - Azure DevOps: paste the PAT and click **Save**, then fill in **Organization** (`yourorg`, as in `dev.azure.com/yourorg`) and, if your teams get review requests, **My teams** (comma-separated).
   - GitHub: paste the token and click **Save**. Optionally, add **Watched repos** (`owner/name`) to count every open PR in them as a review.

   Tokens go to Windows Credential Manager. Other fields save when you leave them.
4. **Optional: link tickets.** Under **Tracker**, pick **Jira** and enter your site (`yourcompany.atlassian.net`), or **Linear** and enter your workspace (`yourteam`, as in `linear.app/yourteam`). Set **Team keys** (`ENG, OPS`) so only those keys become ticket chips.
5. **Optional: more workspaces.** **+ Add workspace** adds another; switch with the chip in the list's header or **Ctrl+←** / **Ctrl+→**. Only the active workspace is polled and notifies.
6. **Optional: the MCP server** for Claude review agents (Azure DevOps PRs only):

   ```
   claude mcp add --scope user rado -- "%LOCALAPPDATA%\Rado\rado-mcp.exe"
   ```

   That is for cmd; in PowerShell, write the path as `"$env:LOCALAPPDATA\Rado\rado-mcp.exe"`.

## Walkthrough

<img src="docs/screenshots/counter.png" width="70" alt="The counter showing 5, Rado's bird, then 1">

**The counter.** Left of the bird: reviews waiting on you. Right: your own PRs that need work. Click it to open the list; drag it anywhere.

<img src="docs/screenshots/review-list.png" width="640" alt="The review list">

**The list.** One tab per section; the top-ranked review is marked *Review next*. A row whose branch names a ticket gets a ticket chip (`ENG-42`), and each PR gets its own chip; a dashed chip is a PR on the same branch that you aren't asked to review (Azure DevOps only). Click a row to open every PR in it, or a chip to open one.

<img src="docs/screenshots/workspace-picker.png" width="640" alt="The workspace picker open under the header chip">

**Switching workspaces.** Click the workspace chip, or press **Ctrl+←** / **Ctrl+→** in the list.

<img src="docs/screenshots/second-workspace.png" width="640" alt="The list for a GitHub workspace">

**Another workspace.** Its own list, here a GitHub PR (`#88`) linked to a Linear ticket.

<img src="docs/screenshots/settings-workspaces.png" width="640" alt="Settings, Workspaces tab">

**Settings → Workspaces** (⚙ in the header). Sources, watched repos, tracker and team keys for each workspace.

<img src="docs/screenshots/settings-general.png" width="640" alt="Settings, General tab in the light theme">

**Settings → General.** Check every (polling interval), text size, list width and height, theme (shown: *Light*), browser, notifications and start with Windows.

<img src="docs/screenshots/settings-counter.png" width="640" alt="Settings, Counter tab with a live preview">

**Settings → Counter.** A live preview, a colour for each number (heat by default) and for the logo, opacity at rest and near the cursor, and size. **Reset to default** brings back the default look.

<img src="src-tauri/icons/32x32.png" width="32" alt="Rado's tray icon">

**The tray icon.** Left-click opens the list. Right-click for **Show reviews**, **Refresh now** and **Quit**.

## Reference

### Counter

- `reviews`, Rado's bird, `mine`; the bird is white by default. *Reviews* are the review rows waiting on you; *mine* are the rows of your own PRs with a rejection, a waiting-for-author vote or an open thread, not all your open PRs.
- By default each number is coloured by count (*Heat*): blue at 0, then cyan, green, yellow and orange, up to red at 10 or more.
- `!` and the bird, with no mine number: the active workspace has no token, or the first refresh failed. The tooltip starts with the active workspace's name.
- Always on top. By default it sits at 55% opacity and fades to 8% when the cursor comes within 15% of its bounds. It starts at the bottom right of the primary screen, 12 px from the edges, and reopens where you left it, moved back on screen if it would stick out.
- **Settings → Counter** changes its look; each change saves as you pick it.
  - *Reviews number* and *Mine number*: **Heat**, a swatch, the colour picker or a typed hex colour. *Logo* (the bird and the `!`): the same without Heat. Settings saved by 0.9.0 keep their braces colour as the logo colour.
  - *Opacity at rest*: 20–100% (chips for 30, 55, 80 and 100%). *Opacity near the cursor*: 0–100% (chips for 0, 8, 25 and 50%).
  - *Size*: 75% to 200%; − / + step by 25%, or type a whole percent. It grows around its right edge.
  - **Reset to default**: heat numbers, white logo, 55% / 8% opacity, 100% size.

### List

- It opens against the screen edge on the counter's side (left or right), level with the counter and kept on screen, on the counter's screen. The counter is hidden while the list is open and comes back when it closes: ✕, **Esc**, or a click anywhere else.
- *List width* (20–100% of the screen, chips for 25%, 33%, Half and Full; never under 360 px) and *List height* (25–100%, chips for Half and Full) are in **Settings → General**. The default is 33% wide and half the screen high. Pixel sizes saved by 0.9.0 are dropped for these defaults.
- **New version:** Rado asks GitHub for its latest release at start and every 6 hours. When it is newer, the status line under the title reads *Rado x.y.z is available · Download*, and **Download** opens the release's installer in your default browser. Run it to update; Rado itself downloads and installs nothing. Offline or rate-limited checks stay silent.
- Tabs: *Review* (needs your review), *Re-review* (new pushes since your vote), *Mine* (your PRs), *Author* (waiting on author), *Approved* (by you or your team), *Quiet* (no activity for 3 weeks) and *Hidden* (Not mine or hidden). All seven always show, each with its count; empty ones are muted. Arrow keys, Home and End move between tabs.
- The list reopens on the tab you last chose. Before you choose one, it opens the first of *Review*, *Re-review* and *Mine* that had rows at start. Each tab keeps its scroll position.
- *Mine* is grouped by branch, newest first. You get a notification when one of your PRs starts needing you.
- **Review next:** the top-ranked review gets the label, an accent bar and a dot on its tab. Rows you opened since Rado started move behind the others; once you've opened them all, it marks the one you opened longest ago.
- **Rows:** PRs with the same source branch in different repos form one row (*N PRs · branch*), titled and placed by their highest-ranked PR, with summed approvals and comments. GitHub and Azure DevOps PRs never share a row. PR chips read `repo !n` (Azure DevOps) or `repo #n` (GitHub).
- **Row details:** `✓ n` approved, `✗ n` rejected or waiting for author, `n comments (m open)` (system threads such as votes and pushes don't count), *required*, *draft*, and `→ <target>` when it merges into `master`, `main`, `qa` or `release`. On the right: *opened* and *active* (the latest push, vote, comment, reviewer or status change; activity in the first five minutes counts as opening). An aggregate uses its earliest PR's opening and its latest activity. Hover for each PR's exact times.
- **Copy** copies the PR link; for an aggregate, one `repo: link` line per PR.
- **Not mine** (on a review) or **Hide** (on your own PR) drops the row from the count and notifications until it closes. On GitHub that's until it leaves Rado's GitHub results, for example once a teammate covers a team request. Hidden PRs are forgotten only after a poll in which every source answered in full. **Show** on the *Hidden* tab brings one back. Hiding is local; Azure DevOps or GitHub still lists you as a reviewer.
- **Quiet:** a review with no activity for 21 days leaves the count, notifications and the MCP queue, and moves to *Quiet*. Any activity brings it back, with a notification. Your own PRs never go quiet; a PR whose threads failed to load stays where it was.
- **Esc** closes the workspace picker, then leaves settings, then hides the list.

### Ranking

One score per PR. Needs review: 50; re-review: 45. Required: +25. Target `master`, `main`, `qa` or `release`: +15. Age: +1 per 4 hours, up to 30. Team-only assignment: −10. Draft: −40. 85 or more is urgent (red dot), 40 or more normal (orange); the rest are grey.

### Workspaces

- A workspace is a named set of sources (an Azure DevOps organization with its PAT and teams, a GitHub token, or both) and a tracker.
- The order doesn't wrap: Ctrl+→ on the last workspace does nothing. The keys don't switch while you type in a field.
- A switch drops any poll still running for the old workspace and refreshes the new one at once. Each workspace keeps its own list, votes, hidden PRs and notification baseline, so switching back shows its last list and only reviews that are new since its last poll notify. A workspace's first poll since Rado started records its reviews without notifying.
- Deleting a workspace (after a confirm) removes its tokens and data. The last workspace can't be deleted.
- Upgrading from before 0.8.0 turns your settings into one workspace named after your organization (or *GitHub* with only a GitHub token, else *Default*). The old file is kept as `settings.pre-workspaces.json`, and the PAT and token move to the workspace's own keyring entries; the old entries are deleted once the new ones read back.

### Trackers

- A ticket key in a branch name (`ABC-12`, as in `feature/ABC-12-login`) becomes a chip that opens the issue in the workspace's Jira site or Linear workspace. Rado only builds the link; no tracker token is needed. With no tracker set, there are no chips. Pasting a Jira or Linear link into the field works too.
- Without team keys, upper-case keys count anywhere in the branch. With Linear, lower-case keys also count in Linear's branch shape (`anton/eng-123-add-login`). `release-2026`, `renovate/node-20`, `UTF-8`, `ISO-8601`, `SHA-256` and `CVE-2024-…` never make chips, but with Linear `chore/node-18-upgrade` and `claude/sprint-3-onboarding` do (NODE-18, SPRINT-3).
- With **team keys** set, only those keys count, in any case, at the start of the branch or after `/`, `-` or `_`. `hoo-312-ranks` gives HOO-312 and `claude/hoo-342-343-panels` gives HOO-342; `sprint-3` gives nothing.

### Sources

- Azure DevOps and GitHub (github.com) are polled at the same time. A source is on once its token is saved. If one fails, its PRs from the last good poll stay listed and the status line names it.
- A source's first good poll (new token, or after failing since start) records its reviews without notifying. A poll that was running when you switched workspaces, saved a token or changed the organization or teams is discarded.
- **Azure DevOps:** every active PR in the organization comes from one paged call. Rado keeps those where you, or one of **My teams**, is a reviewer. Teams match by name (`TeamAlpha` matches `[contoso]\TeamAlpha`). On a team-only PR, the team's vote decides the section. Your own open PRs go under *Mine*; PRs you declined are skipped. One extra call per listed PR fetches its threads and last activity; if it fails, the row shows no comments or activity.
- **GitHub:** one GraphQL call per poll searches every repo the token can see: reviews requested from you or your teams, open PRs you reviewed, and your own open PRs.
  - A push after your approval or change request moves it to *Re-review*. Your vote is your last approval or change request; a later comment doesn't replace it, and a dismissed vote counts as changed.
  - Your latest review approved: *Approved*; requested changes: *Author*. A PR you only commented on, and aren't requested on, isn't listed.
  - **Watched repos** count every open PR there as a review, except drafts and your own (bots' PRs included). They score like a team request and then follow your vote. Pasting a repo link works too. Changing the list re-polls without a notification per PR.
  - `✓` / `✗` count each reviewer's latest approval or change request; comments include review-thread comments.
  - GitHub has no "required reviewer" flag, so GitHub PRs never get that bonus. Last activity is the PR's `updatedAt`. A search with more than 100 results lists the first 100 and says so.

### Browser and tab groups

**Open links in** (settings): *System default*, *Edge* or *Chrome*, for every link Rado opens: PRs, *Review next* and tickets. If the chosen browser can't start, the link opens in the default one and the status line says why.

Every PR link Rado opens ends in `#rado=<branch>` (plus `&group` from an aggregate). The extension in `browser-extension/`, for Edge and Chrome, uses that tag to:
- name the tab `repo:branch` (`WebApp:ABC-4827`), and keep that name while you stay on the PR;
- put an aggregate's tabs into one tab group named after the branch, joining an open group of that name.

Without it, links open as ordinary tabs. To install, open `edge://extensions` or `chrome://extensions`, switch on **Developer mode**, click **Load unpacked** and pick `browser-extension`. It asks for `tabs` and `tabGroups` and runs on `dev.azure.com` and `github.com` only. After an update, click **Reload** on its card. If you loaded the old `edge-extension` folder, remove it and load `browser-extension` instead.

### MCP server

`rado-mcp.exe` is a stdio MCP server that lets a scheduled Claude agent review your queue. It reads `queue.json`, which Rado writes after each good poll, from the active workspace (looked up on every call, so it follows a switch). It never reads your PAT. The agent reviews through the Azure DevOps MCP server and only drafts; it never posts or votes. GitHub PRs aren't in the queue yet.

- `list_review_queue`: reviews waiting on you, best-ranked first, grouped by branch. Each PR has `project`, `repoId`, `prId` and `sourceCommit`, plus `lastVotedCommit` on a re-review. It leaves out hidden, quiet and your own PRs, and PRs already drafted at their current commit. It refuses when the queue is older than three polls (at least 30 minutes), meaning Rado isn't running or can't reach Azure DevOps. Not mine, Hide and Show take effect at the next poll.
- `mark_drafted(prId, commit)`: records the draft in `agent-drafts.json` so later runs skip the PR until a new push. `commit` must be the PR's current `sourceCommit`, so a PR id from another workspace's organization isn't marked by mistake. Nothing changes in Azure DevOps or in Rado's list.

Register the installed server with the command in [Quick setup](#quick-setup), or a source build with `npm run mcp:build` and `claude mcp add --scope user rado -- "<repo>\src-tauri\target\release\rado-mcp.exe"`. A Claude session holding the server locks the exe; close it before you rebuild.

### Storage

- Tokens: Windows Credential Manager, service `review-bracket`, users `ado-pat:<workspace id>` and `github-token:<workspace id>`. The id never changes, so renaming keeps the tokens. `RADO_KEYRING_SERVICE` overrides the service name, for a dev build that must not touch your real tokens.
- Everything else is under `%APPDATA%\dev.antonis.rado\`:
  - `settings.json`: settings and workspaces, with the active one
  - `.window-state.json`: the counter's position
  - `workspaces\<workspace id>\`: `votes.json` (the source commit at your last vote), `hidden.json` (hidden PRs, dropped once closed), `queue.json` (the MCP queue) and `agent-drafts.json` (PRs the agent drafted, with their commits)

### Settings

⚙ in the list's header. *Workspaces* holds each workspace's sources and tracker, **Make active** and **Delete workspace…**. *General* holds what all workspaces share: **Check every** (polling), text size (80–150%, chips for 90, 100, 115 and 130%, − / + by 5%; larger text makes the tab strip scroll sideways), list width and height, theme, browser, notifications and start with Windows; **Quit** is on the tray icon's right-click menu. *Counter* holds the counter's colours, opacity and size (see [Counter](#counter)). Settings reopen on the tab you last chose, or on *Workspaces* when the active workspace has no token.

**Theme:** *System* (default; follows Windows' light or dark mode), *Dark*, *Light*, *High contrast* or *Custom* (a background, text and accent colour, picked or typed as hex such as `#1f2125`; it warns when text or accent reads below 4.5:1). *System*, *Dark* and *Light* use shades of the logo's green as the accent, and the same green marks approvals and OK states. The counter has its own colours, under *Counter*.

## Commands

```
npm install
npm test             # TypeScript and browser extension unit tests
npm run test:rust    # Rust unit tests
npm run app:dev      # run with hot reload
npm run app:build    # release exe + NSIS installer under src-tauri/target/release/bundle/nsis
npm run app:update   # build, then update the installed Rado in place and restart it
```

The installer updates an existing install in place: it skips Tauri's "uninstall before installing" page and the folder page, keeps your shortcuts and settings, and closes a running Rado first. This comes from `src-tauri/windows/installer.nsi`, Tauri CLI 2.12.4's template with Rado's changes (search it for `Rado:`). After upgrading `@tauri-apps/cli`, re-extract the template and reapply them.

The installer and uninstaller also stop every `rado-mcp.exe` running from the install folder (one that won't stop is renamed to `rado-mcp.exe.old` and deleted next time). Claude sessions using it lose the `rado` tools until they restart; new sessions start the new one.

`app:update` runs the installer with `/P /UPDATE /R`: it keeps the existing install, closes a running Rado and starts it again. It builds under `src-tauri/target/app`, so Claude sessions holding `target/release/rado-mcp.exe` don't block it. It starts the installer through WMI (`scripts/run-outside.ps1`): run directly from a Claude desktop session, it would install into the Claude app's private copy of `%LOCALAPPDATA%` instead of the real one. To update by hand, run the setup exe with the same flags.

The `app:*` and `test:rust` scripts go through `scripts/msvc.cmd`, which loads the VS Build Tools environment. Without it, Rust can pick a Visual Studio install that has no C++ libraries.

## Releasing

Features collect on a `release/x.y.z` branch and ship together; `main` always equals the last release. Pull requests into `main` and `release/**` run `.github/workflows/ci.yml` (npm test, cargo test, clippy).

1. Cut the release branch from main: `git fetch origin && git switch -c release/0.8.0 origin/main && git push -u origin release/0.8.0`.
2. Feature PRs target it (`gh pr create --base release/0.8.0`), are squash-merged, have conventional titles (`feat(scope): …`, `fix: …`) and bump no versions. Only `feat` and `fix` squash subjects reach the changelog. For a single-commit PR, GitHub uses the commit's subject instead of the PR title, so give the commit the same conventional title.
3. To release, on the release branch run `npm run release -- 0.8.0` (`--dry-run` prints the section first). It sets the version in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` and `src-tauri/tauri.conf.json`, and writes a dated `## [0.8.0]` section into `CHANGELOG.md` from the commits since the last `v*` tag. Rerunning keeps an existing section as edited and prints the `feat` and `fix` PRs it doesn't mention yet; `--force` regenerates it, dropping hand edits.
4. Edit the section, commit, and open the release PR from `release/0.8.0` into `main`. Merge it with a merge commit, not squash, so the feature commits stay visible.
5. Tag the merge on main and push the tag: `git fetch origin && git tag v0.8.0 origin/main && git push origin v0.8.0`. `.github/workflows/release.yml` builds the installer and publishes it with the changelog section as the release notes; it fails if the section is missing.
6. Cut the next release branch from main after tagging (`release/0.9.0`). A patch release also starts from main (`release/0.8.1`), never from the old release branch: that branch never sees its own tag, so the script would list the whole previous release again.

## License

MIT, see [LICENSE](LICENSE). The Nunito font in `src/fonts/` is under the SIL Open Font License (`src/fonts/OFL.txt`).
