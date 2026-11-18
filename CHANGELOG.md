# Changelog

All notable changes to Rado. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow [Semantic Versioning](https://semver.org/). Each section is generated from the `feat` and `fix` pull request titles of its release (`npm run release`), then edited by hand; it becomes that version's GitHub release notes.

## [0.10.0] - 2026-11-18

### Features

- mcp: review agents get your GitHub PRs too, each marked when a bot opened it or it carries an agent label; the queue names a source whose last poll failed (#114)
- list: a roomier header and tabs, scrollbars in your theme, a status line that shows only when something needs saying, and a header bird that glows on refresh and yawns at new reviews (#116)
- settings: Remove… beside a saved PAT or GitHub token turns that source off (#118)
- extension: the tab groups extension ships with Rado; Settings shows whether your browser has it loaded and helps set it up (#34)
- extension: the Rado logo as the extension's icon (#36)

### Fixes

- list: quieter rows: Copy and Not mine appear on hover or keyboard focus, details wrap as whole phrases, and Review next is clearer (#121)
- list: when one source fails, the list and the counter show that the data is old instead of looking current (#119)
- polling: a hidden PR stays hidden while its source fails or has no token, and a token Windows can't read shows as an error, not as missing (#115)
- settings: a failed save keeps your focus and typing, and Settings reopened during a save no longer saves an older value over it (#117)
- settings: Quit left General; it is on the tray icon's menu (#33)
- release: the extension's version now follows Rado's (#120)

## [0.9.1] - 2026-11-16

### Features

- updates: the list links to a newer release's installer when GitHub has one (#32)
- list: the list opens against the screen edge, and the counter hides while it is open (#28)
- counter: the logo sits between the two counts instead of braces (#27)

## [0.9.0] - 2026-11-16

### Features

- branding: a green shoebill logo, app icon and tray icon; green is the default theme accent (#22)
- settings: a Counter tab to customize the floating `{ reviews | mine }` counter: colours, opacity at rest and near the cursor, and size, with a live preview and Reset (#23)

### Fixes

- settings: a failed save no longer leaves another tab's unsaved value to be written later (#24)
- release: rerunning `npm run release` keeps an edited changelog section and lists the PRs it lacks (#20)

### Documentation

- README: quick setup and a screenshot walkthrough (#21)

## [0.8.0] - 2026-11-16

### Features

- workspaces: switchable workspaces, each with its own sources and tracker; switch with the header chip or Ctrl+← / Ctrl+→, and only the active workspace is polled, counted and notified (#11)
- workspaces: Linear tracker and ticket chips on GitHub rows (#13)
- workspaces: watched GitHub repos count every open PR as a review (#17)
- settings: theme presets (System, Dark, Light, High contrast) and a custom theme (#10)
- settings: text size for the list window (#12)

### Fixes

- workspaces: match ticket keys by the tracker's team keys, so `hoo-342-343-…` links HOO-342 and `sprint-3` gets no chip (#14)
- installer: an update replaces a running `rado-mcp.exe` instead of failing (#15, #16)
- update: `update.cmd` installs into your real Rado even when started from a Claude desktop session (#18)

### Upgrading

- On first start, your settings become one workspace. The old file is kept as `settings.pre-workspaces.json`, and the Azure DevOps PAT and GitHub token move to per-workspace Credential Manager entries (`ado-pat:<id>`, `github-token:<id>` under `review-bracket`). Rado 0.7.0 and earlier can no longer read them.

## [0.7.0] - 2026-11-14

### Features

- list: header tabs for the review list and settings (#8)

## [0.6.2] - 2026-11-13

### Fixes

- list: keyboard-operable PR chips and the release target of grouped rows (#7)

## [0.6.1] - 2026-11-13

### Features

- list: finish the list polish nits (#6)

## [0.6.0] - 2026-11-13

### Features

- list: show each review once and make links and saves obvious (#5)

## [0.5.0] - 2026-11-13

### Features

- list: roomier review list with no truncated text (#4)

## [0.4.3] - 2026-11-13

### Fixes

- installer: recreate a missing Start menu shortcut (#3)

## [0.4.2] - 2026-11-12

### Fixes

- scroll the settings panel (#2)

## [0.4.1] - 2026-11-11

### Changed

- Dependency updates (#1)

## [0.4.0] - 2026-11-11

First release in this repository's history. Earlier builds (0.1.0, 0.2.0) predate it and have no notes.
