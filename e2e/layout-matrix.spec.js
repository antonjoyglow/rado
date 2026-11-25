import { test, expect } from "@playwright/test";
import { open } from "./harness.js";
import { snapshot, item, member, worstCase } from "./fixtures.js";

// Every theme × text size, swept over list widths. Each case loads once and resizes, so the sweep stays cheap.
const THEMES = [
  { theme: "system", colorScheme: "light" },
  { theme: "system", colorScheme: "dark" },
  { theme: "light" },
  { theme: "dark" },
  { theme: "highContrast" },
  { theme: "custom" },
];
const SCALES = [0.8, 1, 1.25, 1.5];
// At 360 px and 150% text the dot and the times column leave the title just over 80 px.
const TITLE_MIN = 80;

/** Both edges, the header's 400 px query and either side of the rows' 55em query at every text size, so a breakpoint
 *  that ignores the text size still lands between two samples (the body's border takes 2 px). */
const WIDTHS = [...new Set([360, 401, 402, 520, 1135, ...SCALES.flatMap((s) => [-1, 1].map((d) => Math.round(55 * 13 * s) + 2 + d))])]
  .filter((w) => w >= 360 && w <= 1135)
  .sort((a, b) => a - b);

/** The Review next row, every fact, many chips beside a short title, and a long title. */
function rows() {
  const chips = Array.from({ length: 6 }, (_, i) => member({ id: 60 + i, number: 4400 + i, repo: `PaymentsService${i}` }));
  return [
    item({ id: 50, key: "first", title: "Retry the token refresh once" }),
    worstCase(),
    item({ members: chips, id: 60, key: "many", ticket: "ENG-5281", title: "Fix it" }),
    item({ id: 70, key: "long", title: "A live layer gets only its own filters, so the sub-filter no longer breaks the live session for anyone" }),
  ];
}

const PAIRS = [
  ...["--ground", "--card", "--card-hover", "--tag", "--tint-accent", "--tint-normal", "--next-bg"].map((f) => ["--fg", f]),
  ...["--ground", "--card", "--card-hover", "--next-bg"].map((f) => ["--muted", f]),
  ["--on-accent", "--accent"],
];

/** Installs window.__layout: colour parsing, WCAG contrast and the checks, all run inside the page. */
function installChecks() {
  const parse = (c) => {
    const n = c.match(/[\d.]+/g).map(Number);
    const srgb = c.startsWith("color(");
    const [r, g, b] = srgb ? n.slice(0, 3) : n.slice(0, 3).map((v) => v / 255);
    return { r, g, b, a: n.length > 3 ? n[3] : 1 };
  };
  const over = (top, under) => ({
    r: top.r * top.a + under.r * (1 - top.a), g: top.g * top.a + under.g * (1 - top.a), b: top.b * top.a + under.b * (1 - top.a), a: 1,
  });
  /** The solid colour behind `el`: its own and its ancestors' background colours, laid over each other. */
  const backdrop = (el) => {
    const layers = [];
    for (let e = el; e; e = e.parentElement) {
      const c = parse(getComputedStyle(e).backgroundColor);
      if (c.a > 0) layers.push(c);
      if (c.a >= 1) break;
    }
    return layers.reduceRight((under, top) => over(top, under), { r: 1, g: 1, b: 1, a: 1 });
  };
  const lum = ({ r, g, b }) => {
    const [R, G, B] = [r, g, b].map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
    return 0.2126 * R + 0.7152 * G + 0.0722 * B;
  };
  const contrast = (a, b) => {
    const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  };
  const name = (el) => el.tagName.toLowerCase() + (el.id ? `#${el.id}` : "") + (el.className && typeof el.className === "string" ? `.${el.className.trim().split(/\s+/).join(".")}` : "");

  window.__layout = {
    hidden: () => [...document.querySelectorAll("[hidden]")]
      .filter((el) => getComputedStyle(el).display !== "none")
      .map((el) => `${name(el)} is [hidden] but displays as ${getComputedStyle(el).display}`),

    list(titleMin) {
      const out = [];
      for (const el of [document.documentElement, document.body, document.querySelector("header"), document.querySelector(".body")]) {
        if (el.scrollWidth > el.clientWidth) out.push(`${name(el)} overflows sideways: ${el.scrollWidth} > ${el.clientWidth}`);
      }
      for (const row of document.querySelectorAll(".row")) {
        const r = row.getBoundingClientRect();
        const key = row.querySelector(".title").textContent.slice(0, 20);
        for (const el of row.querySelectorAll("*")) {
          const b = el.getBoundingClientRect();
          if (b.width && (b.left < r.left - 0.5 || b.right > r.right + 0.5)) out.push(`"${key}": ${name(el)} sticks out of its row (${Math.round(b.left)}–${Math.round(b.right)} outside ${Math.round(r.left)}–${Math.round(r.right)})`);
        }
        const title = row.querySelector(".title").getBoundingClientRect().width;
        if (title < titleMin) out.push(`"${key}": the title is ${Math.round(title)} px wide, under ${titleMin}`);
        const main = row.querySelector(".main").getBoundingClientRect().width;
        for (const chip of row.querySelectorAll(".member")) {
          const width = chip.getBoundingClientRect().width;
          chip.style.whiteSpace = "nowrap";
          const unbroken = chip.getBoundingClientRect().width;
          chip.style.whiteSpace = "";
          if (unbroken > width + 0.5 && unbroken <= main) out.push(`"${key}": chip "${chip.textContent}" breaks over lines though the row has room for it`);
        }
        for (const el of row.querySelectorAll("*")) {
          if (![...el.childNodes].some((n) => n.nodeType === Node.TEXT_NODE && n.textContent.trim())) continue;
          const ratio = contrast(parse(getComputedStyle(el).color), backdrop(el));
          if (ratio < 4.5) out.push(`"${key}": ${name(el)} "${el.textContent.trim()}" reads at ${ratio.toFixed(2)}:1`);
        }
      }
      return out;
    },

    /** The focused element's outline: drawn, and 3:1 against what it is drawn on. */
    focus() {
      const el = document.activeElement;
      const s = getComputedStyle(el);
      if (!el.matches(":focus-visible")) return [`${name(el)} is focused without :focus-visible`];
      if (s.outlineStyle === "none" || parseFloat(s.outlineWidth) === 0) return [`${name(el)} has no focus outline`];
      const under = parseFloat(s.outlineOffset) < 0 ? backdrop(el) : backdrop(el.parentElement);
      const ratio = contrast(parse(s.outlineColor), under);
      return ratio < 3 ? [`${name(el)}'s focus outline ${s.outlineColor} reads at ${ratio.toFixed(2)}:1 on what is under it`] : [];
    },

    pairs(pairs) {
      const colour = (n) => {
        const probe = document.body.appendChild(Object.assign(document.createElement("i"), { style: `color:var(${n})` }));
        const c = getComputedStyle(probe).color;
        probe.remove();
        return c;
      };
      return pairs.flatMap(([text, fill]) => {
        const ratio = contrast(parse(colour(text)), parse(colour(fill)));
        return ratio < 4.5 ? [`${text} on ${fill} reads at ${ratio.toFixed(2)}:1`] : [];
      });
    },
  };
}

const FOCUS_TARGETS = ['#app > .tabs .tab[aria-selected="true"]', '#app > .tabs .tab[aria-selected="false"]', ".row .title", ".row .member", "header .icon"];

for (const { theme, colorScheme } of THEMES) {
  for (const scale of SCALES) {
    const label = colorScheme ? `${theme} (${colorScheme})` : theme;
    test(`${label} at ${scale * 100}% text holds its layout from 360 to 1135 px`, async ({ page }) => {
      if (colorScheme) await page.emulateMedia({ colorScheme });
      await page.addInitScript(installChecks);
      await open(page, snapshot({ items: rows(), settings: { theme, textScale: scale * 100 } }));
      await expect(page.locator(".row")).toHaveCount(4);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

      await page.evaluate(() => document.fonts.ready);
      const problems = await page.evaluate((p) => window.__layout.pairs(p), PAIRS);
      for (const width of WIDTHS) {
        await page.setViewportSize({ width, height: 700 });
        const found = await page.evaluate((min) => [...window.__layout.list(min), ...window.__layout.hidden()], TITLE_MIN);
        problems.push(...found.map((p) => `${width} px: ${p}`));
      }
      for (const target of FOCUS_TARGETS) {
        // A key press first makes the next focus() count as keyboard focus, so :focus-visible applies.
        await page.keyboard.press("Shift");
        await page.locator(target).first().focus();
        problems.push(...(await page.evaluate(() => window.__layout.focus())));
      }
      await page.locator('header [title="Settings"]').click();
      await expect(page.locator(".settings")).toBeVisible();
      problems.push(...(await page.evaluate(() => window.__layout.hidden())).map((p) => `in Settings: ${p}`));

      expect(problems).toEqual([]);
    });
  }
}
