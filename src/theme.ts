import type { CustomTheme, Theme } from "./api";

type Rgb = [number, number, number];

/** WCAG AA for body text. */
export const READABLE = 4.5;

/** A typed colour as lowercase `#rrggbb`: `#RRGGBB` or `rrggbb`, trimmed. Null for anything else. */
export function parseHex(typed: string): string | null {
  const digits = typed.trim().replace(/^#/, "");
  return /^[0-9a-f]{6}$/i.test(digits) ? `#${digits.toLowerCase()}` : null;
}

const rgb = (hex: string): Rgb => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as Rgb;
const hex = (c: Rgb) => `#${c.map((v) => Math.round(v).toString(16).padStart(2, "0")).join("")}`;
/** `share` of `a` blended into `b`. */
const mix = (a: string, b: string, share: number) => {
  const [x, y] = [rgb(a), rgb(b)];
  return hex(x.map((v, i) => v * share + y[i] * (1 - share)) as Rgb);
};

function luminance(color: string): number {
  const [r, g, b] = rgb(color).map((v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio, 1 to 21. */
export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

// Starting hues for status text, from the light and dark palettes in styles.css.
const STATUS_ON_LIGHT = { "--urgent": "#c22d2d", "--normal": "#a35f08", "--clear": "#1d7a44" };
const STATUS_ON_DARK = { "--urgent": "#f26b6b", "--normal": "#d98a1c", "--clear": "#4cc07f" };

/** Every variable a custom theme sets; `--low` is only a dot, so the stylesheet's grey stays. */
const VARIABLES = ["--bg", "--fg", "--muted", "--line", "--hover", "--accent", "--on-accent", "--urgent", "--normal", "--clear"];

/** Every list colour variable, derived from the three picked colours. */
export function customPalette({ background: bg, text: fg, accent }: CustomTheme): Record<string, string> {
  // A tint toward the text colour, unless that tint costs the text its 4.5:1; then a tint the other way.
  const away = luminance(fg) > luminance(bg) ? "#000000" : "#ffffff";
  const hover = [mix(fg, bg, 0.07), mix(away, bg, 0.07)].find((c) => contrast(fg, c) >= READABLE) ?? mix(fg, bg, 0.07);
  // `color` blended toward the text colour just enough to read at 4.5:1 on the background and hover colours.
  const readable = (color: string) =>
    [0, 0.2, 0.4, 0.6, 0.8].map((share) => mix(fg, color, share)).find((c) => contrast(c, bg) >= READABLE && contrast(c, hover) >= READABLE) ?? fg;
  const status = luminance(bg) < 0.18 ? STATUS_ON_DARK : STATUS_ON_LIGHT;
  return {
    "--bg": bg,
    "--fg": fg,
    // As faint as still reads.
    "--muted": readable(mix(fg, bg, 0.6)),
    "--line": mix(fg, bg, 0.18),
    "--hover": hover,
    "--accent": accent,
    "--on-accent": contrast("#000000", accent) >= contrast("#ffffff", accent) ? "#000000" : "#ffffff",
    ...Object.fromEntries(Object.entries(status).map(([name, color]) => [name, readable(color)])),
  };
}

/** The picks that read below 4.5:1 on the background, with their ratio. */
export function unreadable({ background, text, accent }: CustomTheme): string[] {
  return ([["Text", text], ["Accent", accent]] as const)
    .map(([name, color]) => [name, contrast(color, background)] as const)
    .filter(([, ratio]) => ratio < READABLE)
    .map(([name, ratio]) => `${name} ${ratio.toFixed(1)}:1`);
}

/** Sets the list's theme on `root`; a custom theme's colours go inline so they beat every stylesheet rule. */
export function applyTheme(root: HTMLElement, theme: Theme, custom: CustomTheme): void {
  root.dataset.theme = theme;
  if (theme === "custom") {
    for (const [name, value] of Object.entries(customPalette(custom))) root.style.setProperty(name, value);
  } else {
    for (const name of VARIABLES) root.style.removeProperty(name);
  }
}
