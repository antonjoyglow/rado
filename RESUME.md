# Resume: review-list row fixes (not started, plan unapproved)

Branch `antonchar/list-rows-f4b1f6`, based on `origin/release/0.9.2` at f500b4b. The PR goes into `release/0.9.2` as `fix(list): …`, squash-merged, with no version bump. Delete this file before opening the PR.

Status: the plan below has not been approved by the user. No code is written.

## Open questions for the user

1. **Where the dates tooltip goes.** Hiding `.age` on hover (problem 1) makes its tooltip with each PR's exact times (README:86) unreachable. Recommendation: move the tooltip to the `.meta` details line.
2. **Quitting Rado for the screenshots.** The dev build shares the installed Rado's identifier, and `tauri-plugin-single-instance` (src-tauri/src/lib.rs:1010) hands off to the running instance, then exits. The user has to quit Rado while the screenshots are taken.

## Changes to the brief's numbers (measured)

- **Border:** at 60% muted, `--border-strong` is 2.36:1 on the light background, which fails 3:1. At 85% it is at least 3.36:1 in light (bg, hover, next tint), 4.14 in dark and 7.06 in high contrast. Over the 9 custom palettes used in `theme.test.ts`, the minimum is 3.24.
- **Next-row tint:** at 8% accent, muted text drops to 4.45:1 in light, under the 4.5 rule (styles.css:7). At 6% it is 4.59. In custom themes, even 6% drops muted and status text below 4.5 on 4 of 9 palettes (#2e7d32/#fff, #3a3a3a/#c0c0c0, #d38b86/#383211, #000/#767676). Fix: `customPalette` sets `--next-bg` and makes `readable()` also check against it.

## Plan

**1. Copy and Not mine replace the times on hover and keyboard focus** (src/list.ts:241-313)
- `.row` becomes a grid: dot | main | side. Remove the `.open` wrapper and move its onclick onto `.row`.
- `.age` and `.actions` share the side cell, which is as wide as the wider of the two, so titles don't re-wrap on hover.
- Copy and Not mine get `e.stopPropagation()`, as the chips already have.
- Hidden: `opacity: 0; pointer-events: none`, so the buttons stay in the Tab order. They appear on `.row:hover` and `.row:has(:focus-visible)`. Use `:focus-visible`, not `:focus-within`, which stays on after a mouse click on a chip.
- `@media (hover: none)`: the actions get their own column and stay visible. A touchscreen laptop with a mouse reports `hover: hover`, so that case is not covered.
- README: update the row description, the Copy line and the Not mine line (85-88).

**2. Row details break mid-phrase**
- The first fact is one span, `N PRs · branch · author` (list.ts:245). Split it into three facts.
- Move fact building to `rowFacts(item)` in `src/format.ts`, with unit tests.
- CSS: `.row .meta > span { display: inline-block; max-width: 100% }`. A phrase wraps whole, and only a phrase wider than the row breaks inside.

**3. Borders and what green means**
- Add to `:root`: `--border-strong: color-mix(in srgb, var(--muted) 85%, var(--bg))`. It follows every theme, including custom ones, without changes to theme.ts.
- Use it on `.row .member` (ticket, PR and dashed chips), `.row .action`, and the settings inputs and buttons (styles.css:208, 216, 218).
- Leave `.ws-chip` and `.spinner` on `--line`: the header session owns them.
- Green, recommended: PR chips become neutral like the ticket chip (`--border-strong` border, `--fg` text, accent fill on hover). Green then means: selected tab, approved ✓, and the Review next row.
- Alternative: keep PR chips green, meaning "opens something".

**4. Review next**
- The label stays as text but moves to the start of the details line, bold, in `--fg`, not green: `Review next · author · required`. The dot then lines up with the title.
- Add a row tint `--next-bg: color-mix(in srgb, var(--accent) 6%, var(--bg))`, and keep the 3 px bar.
- Custom themes: `customPalette` sets `--next-bg`, and `readable()` checks muted and status text on it as well. Add a test for that.
- README:84 changes too.

**5. "required"**
- `--fg`, weight 600, instead of `--urgent` (styles.css:195).

## Process after approval

- Tests: `npm test`, and `npm run test:rust` (through scripts/msvc.cmd).
- Screenshots of the real app, light and dark, at 360 px and at default width, with a row on hover and a row on keyboard focus. Force the hover and focus states with a temporary CSS rule that isn't committed: on Windows, the real cursor doesn't trigger hover in an unfocused window. Capture with PrintWindow.
- A blind Opus 5.5 high-effort reviewer on the diff; fix what it validly finds.
- Rebase onto `release/0.9.2`. Expect a conflict with the header/tabs/scrollbar session over `--scroll-thumb` and the reduced-motion block in styles.css.
- PR, then green CI, then a squash-merge. Push as `antonjoyglow`.
- Shared note for the header session: `side-session-notes/local_34c922da-…/list-rows.md`.
