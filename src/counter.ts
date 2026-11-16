import { api, type CounterStyle, type Settings } from "./api";
import { h } from "./dom";
import { heat, parseScale, parseWhole, SCALE_MAX, SCALE_MIN, stepScale } from "./format";
import { parseHex } from "./theme";

/** The look before the counter could be customised; matches `CounterStyle::default` in `store.rs`. */
export const COUNTER_DEFAULT: Readonly<CounterStyle> = { reviews: null, mine: null, braces: "#ffffff", restOpacity: 55, hoverOpacity: 8 };
export const DEFAULT_SCALE = 100;

/** Opacity limits in percent, matching `COUNTER_*_RANGE` in `store.rs`; below 20% at rest the counter is too faint to find. */
export const REST_RANGE: readonly [number, number] = [20, 100];
export const HOVER_RANGE: readonly [number, number] = [0, 100];
const REST_CHOICES = [30, 55, 80, 100];
const HOVER_CHOICES = [0, 8, 25, 50];
const SWATCHES = ["#ffffff", "#ffd60a", "#ff9f43", "#f26b6b", "#c792ea", "#6d9ef6", "#4cc07f"];

/** The preview's numbers: heat colours them green and blue, as on a typical day. */
const SAMPLE = { reviews: 5, mine: 1 };

/** A number's colour: the picked one, or heat for its count. */
export function numberColour(fixed: string | null, count: number): string {
  return fixed ?? heat(count);
}

/** The CSS variables `.pill` reads. */
export function counterVars(style: CounterStyle, scalePercent: number): Record<string, string> {
  return {
    "--pill-scale": String(scalePercent / 100),
    "--pill-braces": style.braces,
    "--pill-rest": String(style.restOpacity / 100),
    "--pill-hover": String(style.hoverOpacity / 100),
  };
}

export interface CounterParts {
  pill: HTMLElement;
  reviews: HTMLElement;
  bar: HTMLElement;
  mine: HTMLElement;
}

/** `{ reviews | mine }`; the real counter is a button, the settings preview a plain element. */
export function counterElement(tag: "button" | "div"): CounterParts {
  const reviews = h("span", { class: "count" });
  const bar = h("span", { class: "br" }, "|");
  const mine = h("span", { class: "count" });
  const pill = h(tag, { class: "pill" }, h("span", { class: "br" }, "{"), reviews, bar, mine, h("span", { class: "br" }, "}"));
  if (pill instanceof HTMLButtonElement) pill.type = "button";
  return { pill, reviews, bar, mine };
}

/** Fills in the numbers and colours. `problem` shows a bare `!` in the braces' colour (no token, or the first load failed). */
export function paintCounter(parts: CounterParts, counts: { reviews: number; mine: number }, style: CounterStyle, scalePercent: number, problem = false): void {
  for (const [name, value] of Object.entries(counterVars(style, scalePercent))) parts.pill.style.setProperty(name, value);
  parts.reviews.textContent = problem ? "!" : String(counts.reviews);
  parts.reviews.style.color = problem ? "" : numberColour(style.reviews, counts.reviews);
  parts.mine.textContent = String(counts.mine);
  parts.mine.style.color = numberColour(style.mine, counts.mine);
  parts.bar.hidden = parts.mine.hidden = problem;
}

interface Ui {
  note: () => HTMLElement;
  caption: (text: string, note: HTMLElement) => HTMLElement;
  ok: (note: HTMLElement) => void;
  fail: (note: HTMLElement, e: unknown) => void;
}

/** The Counter settings tab: a live preview, then colours, opacity and size, each saved as it is picked. */
export function counterPanel(settings: Settings, ui: Ui): HTMLElement {
  // A copy, so the snapshot keeps the saved look until the save lands. Every change replaces the object.
  settings.counter = { ...settings.counter };
  const shows: (() => void)[] = [];

  const rest = counterElement("div");
  const near = counterElement("div");
  near.pill.dataset.hover = "true";
  const restLabel = h("span", { class: "sample-label" });
  const nearLabel = h("span", { class: "sample-label" });
  /** Paints the preview with `style`: the saved look, or a value still being picked or typed. */
  const paint = (style: CounterStyle = settings.counter, scale = settings.pillScale) => {
    for (const p of [rest, near]) paintCounter(p, SAMPLE, style, scale);
    restLabel.textContent = `At rest · ${style.restOpacity}%`;
    nearLabel.textContent = `Near the cursor · ${style.hoverOpacity}%`;
  };
  const preview = h("div", { class: "counter-preview", ariaLabel: "Preview" },
    h("div", { class: "sample" }, rest.pill, restLabel),
    h("div", { class: "sample" }, near.pill, nearLabel),
  );
  /** Every control and the preview back to the values in `settings`. */
  const redraw = () => {
    shows.forEach((show) => show());
    paint();
  };

  /** What the last successful save wrote. */
  let saved = { counter: settings.counter, scale: settings.pillScale };
  /**
   * Applies and saves a change. When the newest change fails to save, everything goes back to `saved`,
   * so a later save elsewhere can't carry an unsaved value, even when overlapping saves both failed.
   */
  const commit = (n: HTMLElement, change: Partial<CounterStyle>, scale = settings.pillScale) => {
    const sent = { counter: { ...settings.counter, ...change }, scale };
    settings.counter = sent.counter;
    settings.pillScale = scale;
    redraw();
    api.saveSettings(settings).then(() => {
      saved = sent;
      ui.ok(n);
    }, (e) => {
      if (settings.counter === sent.counter) {
        settings.counter = saved.counter;
        settings.pillScale = saved.scale;
        redraw();
      }
      ui.fail(n, e);
    });
  };
  /** A typed value that doesn't parse: say why, and show the saved value again. */
  const reject = (n: HTMLElement, why: string) => {
    ui.fail(n, why);
    redraw();
  };

  const colourField = (key: "reviews" | "mine" | "braces", label: string) => {
    const n = ui.note();
    const heatChip = key === "braces" ? null : h("button", { type: "button", class: "chip", title: "Blue at 0, through green and yellow, to red at 10 or more", onclick: () => commit(n, { [key]: null }) }, "Heat");
    const swatches = SWATCHES.map((c) => {
      const swatch = h("button", { type: "button", class: "swatch", title: c, ariaLabel: `${label}: ${c}`, onclick: () => commit(n, { [key]: c }) });
      swatch.style.background = c;
      return swatch;
    });
    const picker = h("input", { type: "color", ariaLabel: `${label} colour` });
    const typed = h("input", { type: "text", class: "hex", spellcheck: false, ariaLabel: `${label} colour, as hex` });
    shows.push(() => {
      const value = settings.counter[key];
      if (heatChip) heatChip.dataset.on = String(value === null);
      swatches.forEach((s, i) => (s.dataset.on = String(SWATCHES[i] === value)));
      // A colour input can't be empty; while on heat it starts from grey.
      picker.value = value ?? "#808080";
      typed.value = value ?? "";
      typed.placeholder = value === null ? "Heat" : "";
    });
    picker.oninput = () => paint({ ...settings.counter, [key]: picker.value });
    picker.onchange = () => commit(n, { [key]: picker.value });
    typed.oninput = () => {
      const value = parseHex(typed.value);
      if (value) paint({ ...settings.counter, [key]: value });
    };
    typed.onchange = () => {
      const value = parseHex(typed.value);
      if (value === null) reject(n, "Use a hex colour such as #ffd60a.");
      else commit(n, { [key]: value });
    };
    return h("div", { class: "field" }, ui.caption(label, n), h("span", { class: "inline" }, heatChip, ...swatches, picker, typed));
  };

  const opacityField = (key: "restOpacity" | "hoverOpacity", label: string, range: readonly [number, number], choices: number[]) => {
    const n = ui.note();
    const input = h("input", { type: "number", min: String(range[0]), max: String(range[1]), class: "scale", ariaLabel: `${label} in percent` });
    const chips = choices.map((p) => h("button", { type: "button", class: "chip", onclick: () => commit(n, { [key]: p }) }, `${p}%`));
    shows.push(() => {
      input.value = String(settings.counter[key]);
      chips.forEach((c, i) => (c.dataset.on = String(choices[i] === settings.counter[key])));
    });
    input.oninput = () => {
      const percent = parseWhole(input.value, range);
      if (percent !== null) paint({ ...settings.counter, [key]: percent });
    };
    input.onchange = () => {
      const percent = parseWhole(input.value, range);
      if (percent === null) reject(n, `Use a whole number from ${range[0]} to ${range[1]}.`);
      else commit(n, { [key]: percent });
    };
    return h("div", { class: "field" }, ui.caption(`${label} (${range[0]}–${range[1]}%)`, n), h("span", { class: "inline" }, ...chips, input, "%"));
  };

  const sizeField = () => {
    const n = ui.note();
    const input = h("input", { type: "number", min: String(SCALE_MIN), max: String(SCALE_MAX), class: "scale", ariaLabel: "Counter size in percent" });
    const smaller = h("button", { type: "button", title: "Smaller", onclick: () => commit(n, {}, stepScale(settings.pillScale, -1)) }, "−");
    const bigger = h("button", { type: "button", title: "Bigger", onclick: () => commit(n, {}, stepScale(settings.pillScale, 1)) }, "+");
    shows.push(() => {
      input.value = String(settings.pillScale);
      smaller.disabled = settings.pillScale <= SCALE_MIN;
      bigger.disabled = settings.pillScale >= SCALE_MAX;
    });
    input.oninput = () => {
      const percent = parseScale(input.value);
      if (percent !== null) paint(settings.counter, percent);
    };
    input.onchange = () => {
      const percent = parseScale(input.value);
      if (percent === null) reject(n, `Use a whole number from ${SCALE_MIN} to ${SCALE_MAX}.`);
      else commit(n, {}, percent);
    };
    // Not <label>: a label forwards clicks on its caption to its first button.
    return h("div", { class: "field" }, ui.caption(`Size (${SCALE_MIN}–${SCALE_MAX}%)`, n), h("span", { class: "inline" }, smaller, input, "%", bigger));
  };

  const resetNote = ui.note();
  const reset = h("button", { type: "button", onclick: () => commit(resetNote, COUNTER_DEFAULT, DEFAULT_SCALE) }, "Reset to default");

  const panel = h("div", { class: "counter-settings" },
    preview,
    colourField("reviews", "Reviews number"),
    colourField("mine", "Mine number"),
    colourField("braces", "Braces and bar"),
    opacityField("restOpacity", "Opacity at rest", REST_RANGE, REST_CHOICES),
    opacityField("hoverOpacity", "Opacity near the cursor", HOVER_RANGE, HOVER_CHOICES),
    sizeField(),
    h("div", { class: "end" }, resetNote, reset),
  );
  redraw();
  return panel;
}
