# Glossary

**Workspace**: a named set of sources (an Azure DevOps organization and teams, watched GitHub repos) with its own secrets, tracker, hidden PRs and vote memory. One is active at a time.

**Source**: where PRs come from: Azure DevOps or GitHub. Each is on when its workspace has a token for it.

**Source poll**: one source's fresh answer in the shape every source shares (`src-tauri/src/source.rs`): my PRs, their links and threads, related PRs on my branches, what the MCP queue needs, and whether anything may be missing. Each source's adapter builds it with `digest`; the session files it without knowing the source.

**Related PR**: an open PR on the same source branch as one of mine, where I am not a reviewer. Shown in that branch's row; only Azure DevOps lists them today.

**Workspace session**: the module (`src-tauri/src/session.rs`) that owns the settings, every workspace's state and the store, and decides when a poll is stale, when a baseline resets and when the poller must wake. The app's commands call it and render what it reports.

**Poll generation**: a counter in the session. A switch moves it on, and so does a saved or removed secret or a changed source on the active workspace (not on a parked one), which makes the poll in flight stale: its result is dropped instead of being filed.

**Baseline**: per workspace and source, the PRs already waiting on me at that source's last good poll. A poll announces only PRs not in the baseline. A source with no baseline yet (a new token, a changed source, a first visit) records its PRs without announcing them.
