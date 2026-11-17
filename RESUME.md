# Resume: list chrome (scrollbar, bird motion, header, tabs)

Paused 2026-11-17 at the user's request. Delete this file before the PR.

## Left to do
1. **Rerun the blind review.** An Opus 5.5 high-effort reviewer was started on `git diff f500b4b..HEAD` and stopped before it reported. Nothing from it is known. Rerun it, then fix what it validly finds.
2. Rebase onto `origin/release/0.9.2` if it moved (the parallel "List row problems" work touches `src/list.ts` rows and `src/styles.css`: expect a small conflict).
3. Open one PR into `release/0.9.2`, squash merge, title `feat(list): …`, no version bump. Push as `antonjoyglow`.
4. Wait for CI, merge when green with no open comments.

## Branches (all pushed or merged)
| Branch | Head | State |
|---|---|---|
| `antonchar/list-chrome-61d6dc` (integration) | see the commit that adds this file | all four requests merged |
| `worktree-agent-aac1d50c721f905fd` (#1 scrollbar) | `62dc112` | merged into integration |
| `worktree-agent-a9c41c1ef8a8f0d72` (#2 bird motion) | `99b1a9b` | merged into integration |
| `worktree-agent-a2873affab62fc77c` (#3 header) | `026b4cb` | merged into integration |
| `worktree-agent-abf6d441461b1554d` (#4 tabs) | `a04a360` | merged into integration |

The subagent branches are local only (fully contained in the integration branch). Base commit for all four: `013fc15` (inline bird SVG with a separate lower jaw, `src/bird.ts`).

## Integrated
- **#1 Scrollbar:** `--scroll-thumb` at an **80%** mix (spec said 70%; the orchestrator changed it to reach 3:1). `scrollbar-width: thin; scrollbar-color` on `.body`, `.settings .tabpanel`, `.tabs`.
- **#2 Bird motion:** idle status text removed; `.status` hidden unless busy > 5 s, error, warning, newer release or stale ("Last checked N min ago"); ↻ spins while polling; refresh title "Refresh now · checked … · every N min"; glow on successful refresh (`glows()` in `src/status.ts`); yawn on the Rust `fresh` event (emitted in `apply`, `src-tauri/src/lib.rs`), held until focus; `role="status"` announcer; reduced-motion block and 1.5 s accent fade. The browser-fallback message has no own channel: it arrives as `ws.error`, so it shows as the error line.
- **#3 Header:** padding/gap/min-height, bird 28 px × min(scale, 1.25), h1 18 px bold ellipsis, `.icon` 32×32 with focus ring, `.ws-chip` 28 px, container query hides h1 below 400 px (adds `margin-left: auto` on the chip there, beyond the spec, so the chip and icons stay right).
- **#4 Tabs:** `#app > .tabs .tab` grows; tabular numbers; selected count in accent; no weight change for empty tabs; scrolls below about 520 px (measured, spec said ~525).

## Verified (this session)
- `npx tsc --noEmit` clean; `npm test` 208/208; `npm run test:rust` 156 + 18 passed.
- Real dev build (WebView2, throwaway identifier `dev.antonis.rado.listchrome`, real data): screenshots light/dark at default width and 360 px, yawn frozen at 450 ms, glow at 120 ms. They are in `C:\dev\tmp\shots\` and were sent to the user.
- Yawn path end to end: `fresh` emitted while unfocused → held → played on focus, both jaws animated, announcer "2 new reviews".
- Reduced motion (emulated): bird fades to the accent, no jaw animation, ↻ spin off.
- Scrollbar thumb contrast against `--bg` at 80%: light `#898d94` 3.33:1, dark `#81878f` 4.45:1, high contrast `#a6a6a6` 8.63:1.

## Not verified
- The blind review (see above).
- Custom-theme thumb contrast (depends on the user's colours).
- At 360 px the tab strip's thin scrollbar shows arrow buttons in WebView2; that predates this change.
