# Changelog

All notable changes to Rado. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow [Semantic Versioning](https://semver.org/). Each section is generated from the `feat` and `fix` pull request titles of its release (`npm run release`), then edited by hand; it becomes that version's GitHub release notes.

## [0.9.1] - 2026-11-16

### Features

- updates: Rado installs new releases from GitHub by itself, while the list is closed (#29)
- list: the list opens against the screen edge, and the counter hides while it is open (#28)
- counter: the logo sits between the two counts instead of braces (#27)

### Upgrading

- Install 0.9.1 by hand once. 0.9.0 and earlier have no updater, so automatic updates start with 0.9.2.

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
