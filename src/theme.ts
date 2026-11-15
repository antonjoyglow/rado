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

// The status colours of the light and dark palettes in styles.css.
const STATUS_ON_LIGHT = { "--urgent": "#c22d2d", "--normal": "#a35f08", "--low": "#8a9099", "--clear": "#1d7a44" };
const STATUS_ON_DARK = { "--urgent": "#f26b6b", "--normal": "#d98a1c", "--low": "#8a9099", "--clear": "#4cc07f" };

/** Every list colour variable, derived from the three picked colours. */
export function customPalette({ background: bg, text: fg, accent }: CustomTheme): Record<string, string> {
  const hover = mix(fg, bg, 0.07);
  // As faint as still reads at 4.5:1 on the hover colour, else the text colour itself.
  const muted = [0.6, 0.7, 0.8, 0.9].map((share) => mix(fg, bg, share)).find((c) => contrast(c, hover) >= READABLE) ?? fg;
  const onAccent = contrast("#000000", accent) >= contrast("#ffffff", accent) ? "#000000" : "#ffffff";
  return {
    "--bg": bg,
    "--fg": fg,
    "--muted": muted,
    "--line": mix(fg, bg, 0.18),
    "--hover": hover,
    "--accent": accent,
    "--on-accent": onAccent,
    ...(luminance(bg) < 0.18 ? STATUS_ON_DARK : STATUS_ON_LIGHT),
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
  const palette = customPalette(custom);
  for (const [name, value] of Object.entries(palette)) {
    if (theme === "custom") root.style.setProperty(name, value);
    else root.style.removeProperty(name);
  }
}
