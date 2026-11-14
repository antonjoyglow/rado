# Rado

A floating `{ reviews | mine }` that counts the Azure DevOps and GitHub pull requests waiting on your review, ranks them, and notifies you when a new one arrives. Tauri 2: a Rust backend with a vanilla TypeScript UI.

- **{ reviews | mine }:** two counts in white pixel braces (Silkscreen, bundled under `src/fonts/` with its OFL licence), with no background or border. The left number is the reviews waiting on you; the right is the rows of your own PRs that need work (a rejection, a waiting-for-author vote or an open thread), not all your open PRs. Each number has a heat colour: blue at 0, then cyan, green, yellow and orange, up to red at 10 or more. No token at all (neither an Azure DevOps PAT nor a GitHub token), or a failed first refresh, shows `{!}`. It is always on top, draggable, and remembers its position. Click it to open the list.
- **Size:** *Counter size* in settings goes from 75% to 200% of the original 16 px. The − / + buttons step by 25%, and you can type any whole percent in that range. The counter grows and shrinks around its right edge and stays on screen. Sizes other than 100% and 200% are slightly soft, since the pixel font is sharp only at whole multiples.
- **Opacity:** same rules as Glance. It sits at 0.55 and fades to 0.08 when the cursor comes within 15% of its bounds, so it never hides what is underneath.
- **Review next:** the top-ranked review waiting on you carries a *Review next* label and an accent bar, and its tab shows a dot. Rows you opened since Rado started move behind the ones you haven't, so opening the marked row moves the label to the next one; once you have opened them all, it marks the one you opened longest ago.
- **List:** one tab per section under the header: *Review* (needs your review), *Re-review* (new pushes since your vote), *Mine* (your PRs), *Author* (waiting on author), *Approved* (by you or your team), *Quiet* (for 3 weeks) and *Hidden* (not mine or hidden). Each tab shows its count, and only the selected tab's rows are listed. Hover a tab for its full name. All seven are always there, so they never move; an empty one is muted. Arrow keys, Home and End move between tabs. The list reopens on the tab you last chose; until you choose one, it opens the first of *Review*, *Re-review* and *Mine* that had rows when Rado started. Each tab keeps its scroll position. Click a row to open the PR. The counts are rows that need you, not PRs: reviews to do on the left, your own PRs that have a rejection or waiting-for-author vote or an unresolved comment thread on the right. *Mine* lists your PRs, grouped by branch, newest first: a row sorts by its most recently opened PR. You get a notification when one of your PRs starts needing you.
- **Settings:** ⚙ in the header, with tabs for *Azure DevOps* (PAT, organization, teams, Jira site), *GitHub* (token) and *General* (polling, counter and list size, browser, notifications, start with Windows, Quit). They reopen on the tab you last chose; with no token saved at all, they open on *Azure DevOps*. Esc goes back to the list, and Esc on the list hides it.
- **Aggregates:** PRs with the same source branch in different repos form one row.
  - The row shows *N PRs · branch* and one chip per PR. A row with a single PR shows its one chip too. Click a chip to open just that PR; a dashed chip is a PR on that branch that you aren't asked to review.
  - Clicking the row opens every PR in its own browser tab.
  - The row takes its title and section from its highest-ranked PR. Its approval and comment counts are the sums across the PRs.
- **Quiet for 3 weeks:** a review with no activity for 21 days (no push, vote, comment, reviewer or status change, or nothing since it opened) leaves the count, the notifications and the MCP queue, and moves to the *Quiet* tab. Any new activity brings it back, with a notification. Your own PRs never go quiet. A PR whose threads failed to load stays where it was.
- **Jira:** when an Azure DevOps branch name holds an issue key (`ABC-12`, as in `feature/ABC-12-login`), the row shows it as a chip that opens the issue on the **Jira site** set in settings (for example `yourcompany.atlassian.net`). With no Jira site set, there are no chips. No Jira token is needed; Rado only builds the link.
- **Copy:** copies the PR link to the clipboard. For an aggregate, it copies one `repo: link` line per PR.
- **Each row shows:**
  - `✓ n`: reviewers who approved
  - `✗ n`: reviewers who rejected or are waiting for the author
  - `💬 n · m open`: written comments and unresolved threads (system threads such as votes and pushes aren't counted)
  - `5d · 2h` on the right: how long ago it was opened, then (muted) its last activity: the latest push, vote, comment, reviewer or status change. An aggregate shows when its earliest PR (of those assigned to you or opened by you) was opened, and its latest activity across those PRs, a later PR's opening included. Activity in the first five minutes, such as Azure DevOps adding the reviewers, counts as part of opening. Hover for each PR's exact times. The activity part is missing when nothing happened after opening, or when the PR's threads didn't load.
- **Ranking:** one score per PR.
  - Needs review counts for 50 and re-review for 45.
  - Required adds 25. A target of `master`/`main`/`qa`/`release` adds 15. Age adds 1 point per 4 h, up to 30.
  - A team-only assignment subtracts 10, and a draft subtracts 40.
  - A score of 85 or more is urgent.
- **Sources:** Azure DevOps, GitHub (github.com), or both, polled at the same time. A source is on once its token is saved in settings. If one source fails on a poll, its PRs from the last good poll stay listed and the status line names the source. A source's first good poll (a new token, or after failing since start) records its reviews without a notification; a poll that was already running when you saved a token or changed the organization or teams is discarded. A GitHub search with more than 100 results lists the first 100 and says so in the status line.
- **Data (Azure DevOps):**
  - Every active PR in the **Organization** set in settings comes from one paged call.
  - It keeps the PRs where you are a reviewer, or where one of the teams under **My teams** is. A team is matched by name, so `TeamAlpha` matches the reviewer `[contoso]\TeamAlpha`.
  - On a team-only PR, the team's vote (cast by a teammate) decides the section.
  - Your own open PRs go under *Mine*, with their approvals and comments. PRs you declined are skipped.
  - One extra call per PR in your list fetches its comment threads. The same call gives its last activity. If that call fails, the PR just shows no comment count and no activity.
- **Data (GitHub):** one GraphQL call per poll runs three searches, across every repo the token can see: reviews requested from you or your teams, open PRs you reviewed, and your own open PRs.
  - Requested from you, or from a team only (shown as *team*): *Review*.
  - A push after your approval or change request: *Re-review*, using the commit your review was on. Your vote is your last approval or change request in the PR's review history, so a later comment doesn't replace it; if that vote was dismissed (for example by a repo that dismisses stale approvals on push), the PR counts as changed since your vote.
  - Your latest review approved: *Approved*; requested changes: *Author*. A PR you only commented on, and aren't requested on, isn't listed.
  - `✓` / `✗` count each reviewer's latest approval or change request (comments after it don't change it); `💬` counts comments and review-thread comments, with unresolved threads as *open*. Last activity is the PR's `updatedAt`.
  - GitHub has no "required reviewer" flag in its API, so GitHub PRs never get the *required* bonus.
  - Rows group by branch within a source: a GitHub and an Azure DevOps PR on the same branch name are two rows. Rows show `repo #n` for GitHub and `repo !n` for Azure DevOps.
  - GitHub PRs aren't in the MCP review queue yet.
- **Storage:**
  - The PAT and the GitHub token are kept in Windows Credential Manager (`review-bracket` / `ado-pat` and `github-token`).
  - Everything else is in `%APPDATA%\dev.antonis.rado\`:
    - `settings.json`: your settings
    - `votes.json`: the source commit at your last vote, per PR
    - `hidden.json`: the PRs you hid; a PR leaves this list once it closes
    - `.window-state.json`: the pill's position
    - `queue.json`: the review queue for the MCP server; `agent-drafts.json`: the PRs it has drafted, with their commits
- **Not mine / Hide:** click **Not mine** on a review row (**Hide** on one of your own PRs) to drop that row (every PR of yours in it) from the count and from notifications until it closes (on GitHub, until it drops out of Rado's GitHub list, for example once a teammate covers a team request). Hidden PRs are only forgotten after a poll in which every source answered in full. Hiding stays on this computer; Azure DevOps or GitHub still lists you as a reviewer. To bring it back, use **Show** on the *Hidden* tab.
- **Placement:** on first run, the counter starts at the bottom right of the primary screen, 12 px from the edges as in Glance. After you drag it, it reopens where you left it, moved back inside the screen if it would stick out.

## Browser and tab groups

**Open links in** (settings): *System default*, *Edge* or *Chrome*. Every link Rado opens follows it: PRs, *Review next* and Jira. If the chosen browser can't be started, the link opens in the default browser and the status line says why.

Every PR link Rado opens ends in `#rado=<branch>`; links from an aggregate also end in `&group`. The extension in `browser-extension/` works in Edge and Chrome and does two things with that tag:
- **Tab title:** it names the tab `repo:branch`, for example `WebApp:ABC-4827`. Azure DevOps and GitHub keep rewriting the title, so the extension puts it back each time. The name survives a reload, and the tab returns to its usual title once you leave that PR.
- **Tab group:** it puts the tabs of an aggregate into one tab group named after the branch. If that group is already open in the window, the tabs join it.

Without the extension, the links open as ordinary tabs.

To install it once, in each browser you use:
1. Open `edge://extensions` or `chrome://extensions` and switch on **Developer mode**.
2. Click **Load unpacked** and pick the `browser-extension` folder.

The extension asks for the `tabs` and `tabGroups` permissions, and it runs on `dev.azure.com` and `github.com` pages only. After you update it, click **Reload** on its card. If you loaded the old `edge-extension` folder, remove that card and load `browser-extension` instead.

## MCP server for review agents

`rado-mcp.exe` lets a scheduled Claude agent review your queue.

**What it does**
- It is a stdio MCP server. It reads the queue that Rado writes after each successful poll: `queue.json` in the data folder.
- It never reads the PAT.
- The review itself goes through the Azure DevOps MCP server. The agent drafts its comments and never posts or votes.

**Tools**
- `list_review_queue`: the reviews waiting on you, best-ranked first and grouped by branch.
  - Each PR has `project`, `repoId`, `prId` and `sourceCommit`.
  - A re-review also has `lastVotedCommit`.
  - It leaves out hidden and quiet PRs, your own PRs, and PRs already drafted at their current commit.
  - It refuses when the queue is older than three polls (at least 30 minutes), which means Rado isn't running or can't reach Azure DevOps.
  - A Not mine, Hide or Show takes effect at the next poll.
- `mark_drafted(prId, commit)`: records the draft in `agent-drafts.json`, so later runs skip the PR until someone pushes again. Nothing changes in Azure DevOps or in Rado's list.

**Build and register it**

```
npm run mcp:build
claude mcp add --scope user rado -- "<repo>\src-tauri\target\release\rado-mcp.exe"
```

A Claude session holding the server keeps the exe locked. Close that session before you rebuild.

## Token scopes

- **Azure DevOps PAT:** **Code (Read)** is all it needs.
- **GitHub token:** a classic token with `repo` (to read private repos) and `read:org` (to see review requests to your teams). A fine-grained token with *Pull requests: Read* and *Metadata: Read* on your orgs should also work, but some orgs must approve it first (not tested yet).

## Commands

```
npm install
npm test             # TypeScript and Edge extension unit tests
npm run test:rust    # Rust unit tests
npm run app:dev      # run with hot reload
npm run app:build    # release exe + NSIS installer under src-tauri/target/release/bundle/nsis
npm run app:update   # build, then update the installed Rado in place and restart it
```

The installer updates an existing install in place: it skips Tauri's "uninstall before installing" page and the folder page, keeps your shortcuts and settings, and closes a running Rado first. This comes from `src-tauri/windows/installer.nsi`, Tauri CLI 2.12.4's template with that one change (search it for `Rado:`). After upgrading `@tauri-apps/cli`, re-extract the template and reapply the change.

`app:update` runs the installer with `/P /UPDATE /R`: it keeps the existing install instead of uninstalling it first, closes a running Rado, and starts it again. It builds under `src-tauri/target/app`, so Claude sessions holding `target/release/rado-mcp.exe` don't block it. To update by hand, run the setup exe with the same flags.

The `app:*` and `test:rust` scripts go through `scripts/msvc.cmd`, which loads the VS Build Tools environment. On this machine, Rust would otherwise pick a Visual Studio install that has no C++ libraries.

## Releasing

Features collect on a `release/x.y.z` branch and ship together; `main` always equals the last release. Pull requests into `main` and `release/**` run `.github/workflows/ci.yml` (npm test, cargo test, clippy).

1. Cut the release branch from main: `git fetch origin && git switch -c release/0.8.0 origin/main && git push -u origin release/0.8.0`.
2. Feature PRs target it (`gh pr create --base release/0.8.0`), are squash-merged, have conventional titles (`feat(scope): …`, `fix: …`) and bump no versions. Only `feat` and `fix` squash subjects reach the changelog. For a PR with a single commit, GitHub uses that commit's subject instead of the PR title, so give the commit the same conventional title.
3. To release, on the release branch run `npm run release -- 0.8.0` (`--dry-run` prints the section first). It sets the version in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` and `src-tauri/tauri.conf.json`, and writes a dated `## [0.8.0]` section into `CHANGELOG.md` from the commits since the last `v*` tag. Rerunning it replaces that section, hand edits included.
4. Edit the section, commit, and open the release PR from `release/0.8.0` into `main`. Merge it with a merge commit, not squash, so the feature commits stay visible.
5. Tag the merge on main and push the tag: `git fetch origin && git tag v0.8.0 origin/main && git push origin v0.8.0`. `.github/workflows/release.yml` builds the installer and publishes it with the `0.8.0` changelog section as the release notes; it fails if the section is missing.
6. Cut the next release branch from main after tagging (`release/0.9.0`). A patch release also starts from main (`release/0.8.1`), never from the old release branch: that branch never sees its own tag, so the script would list the whole previous release again.

## License

MIT, see [LICENSE](LICENSE). The Silkscreen font in `src/fonts/` is under the SIL Open Font License (`src/fonts/OFL.txt`).
