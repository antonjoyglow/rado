# Issue sessions for a release

One session owns one GitHub issue of antonjoyglow/rado and ships it as one PR into `<release branch>` (for example `release/0.12.0`). An orchestrator session starts the issue sessions and runs the release-level steps at the end.

Keep a good pace: no tests after every edit, only the full check (step 4). Don't spend time on anything that doesn't change the result.

## Steps

1. **Plan.** Read the issue: `gh issue view <n> -R antonjoyglow/rado --comments`. Read the code (CodeGraph: `codegraph explore "<names>"`). Write a short plan:
   - the smallest design that meets the issue (when the issue offers options, pick one and say why);
   - the files you'll change;
   - the tests you'll add;
   - the behaviour that changes for existing users' data.
2. **Grill the plan.** Give the plan and the issue to one blind reviewer subagent (model opus, effort high). Ask what is wrong or missing, what is simpler, and which existing test or doc contradicts it. Fix the plan. Ask the owner only when a decision has no sensible default.
3. **Implement**, with regression tests in the existing suites (cargo `mod tests`, vitest). UI changes also add a case to `e2e/` and run `npm run e2e`; behaviour that only WebView2 shows (focus, window size, native dialogs) also runs `npm run e2e:webview2`. Code rules:
   - comments only for constraints the code can't show;
   - no dead code;
   - English only;
   - test names in the file's existing style.
4. **Check once at the end:** `npm ci` (once), then `npm run check` (tsc, vitest, Rust tests, clippy `--all-targets -D warnings`).
   - All sessions share one Rust target dir: set `CARGO_TARGET_DIR=C:\Users\AntonisCharalampidis\dev\repos\personal\rado\src-tauri\target` (sessions mostly run one at a time).
   - The first build takes minutes: run it in the background, with the output in a file you grep afterwards for `test result`, `error` and `warning`.
   - `scripts\msvc.cmd` passes the command's exit code through, so a failing step fails `npm run check`.
   - Commit the ts-rs bindings under `src/bindings` if they change.
5. **Review the code.** Give your diff (`git diff origin/<release branch>`) to a blind reviewer subagent (opus, effort high). Ask for bugs and for what to delete or simplify, with the code, not your conclusions. Fix what is valid, then run a second fresh review. **After the second round, only a real bug or a medium/high finding starts another round**; wording and style don't. Re-run step 4 if code changed.
6. **PR.** One commit or a few, Conventional Commits.
   - The PR title becomes the squash subject and the changelog line, so write it for a user: `fix(area): …` or `feat(area): …`.
   - End commit messages with the `Co-Authored-By` line the session gives you.
   - Push as GitHub account `antonjoyglow` (the active gh account).
   - Open the PR into `<release branch>`. Body: what changed, how it was checked (test counts), what is not verified, a `Closes #n` line per issue (or `Part of #n` when the issue keeps open parts), and last the generated-with line. Merging closes the `Closes`/`Fixes` issues (`.github/workflows/close-issues-on-release-merge.yml`); don't close them by hand.
   - Merge with `gh pr merge <pr> -R antonjoyglow/rado --auto --squash` and stop; don't watch CI yourself. If CI fails or the base moved, the orchestrator tells you: merge `origin/<release branch>` into your branch (never rebase or force push), re-test, push.
7. **Report** in your last message, under 150 words: PR link, merge state, test counts, review rounds and what they found, and anything not verified (UI you didn't see in `e2e/` or the running app).

## Several issues in one session

Stay in your worktree and cut one branch per PR with `git switch -c <branch> origin/<release branch>`. Keep scratch files inside your worktree: paths in sibling worktrees are blocked.

## Never

- install Rado or run an installer (a Claude desktop session installs into the app's private AppData; see README, "Commands");
- enter or read real tokens;
- touch the real `%APPDATA%\dev.antonis.rado`;
- change system settings;
- push to `main`, tag or release;
- edit files outside your own worktree.

Windows: Git Bash mangles backslashes in heredocs and inline scripts, so write scripts with the Write tool. Parallel shells share one working directory, so start commands with `cd <worktree> && `. A force push is refused.

## Orchestrator

- Start one session per issue with this file as its brief, the release branch filled in, and any default the owner already chose.
- A status message that isn't done, blocked or asking the owner gets a one-line reply and no tool calls.
- A product decision taken on the owner's behalf goes on its issue as a comment (what was decided and why), so the owner can overturn it there.

## Release-level steps

1. After the last issue PR merges and before QA: one blind Opus review (effort high) of `git diff main...<release branch>`, asking specifically for interactions between PRs: shared state, load paths, migrations, settings and data-file formats. Per-PR reviews don't see these.
2. QA, then release as in README, "Releasing".
3. At release end, remove the worktrees of merged branches (`git worktree list`, `git worktree remove <path>`), then `git worktree prune`.
