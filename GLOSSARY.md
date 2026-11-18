# Glossary

**Workspace**: a named set of sources (an Azure DevOps organization and teams, watched GitHub repos) with its own secrets, tracker, hidden PRs and vote memory. One is active at a time.

**Source**: where PRs come from: Azure DevOps or GitHub. Each is on when its workspace has a token for it.

**Workspace session**: the module (`src-tauri/src/session.rs`) that owns the settings, every workspace's state and the store, and decides when a poll is stale, when a baseline resets and when the poller must wake. The app's commands call it and render what it reports.

**Poll generation**: a counter in the session. A switch moves it on, and so does a saved or removed secret or a changed source on the active workspace (not on a parked one), which makes the poll in flight stale: its result is dropped instead of being filed.

**Baseline**: per workspace and source, the PRs already waiting on me at that source's last good poll. A poll announces only PRs not in the baseline. A source with no baseline yet (a new token, a changed source, a first visit) records its PRs without announcing them.
