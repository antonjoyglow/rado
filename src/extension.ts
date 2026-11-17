import type { ExtensionStatus } from "./api";

/** What Settings says about the tab groups extension; `steps` is set when Set up is needed. Set up copies the page address. */
export function extensionMessage(s: ExtensionStatus, browser: "edge" | "chrome"): { text: string; steps: string | null } {
  const page = `Set up copies ${browser}://extensions: paste it into the new tab and press Enter.`;
  const load = `Switch on Developer mode, click Load unpacked and pick ${s.folder} (selected in Explorer).`;
  switch (s.state) {
    case "loaded":
      return { text: s.from === null ? "Tab groups extension: loaded" : `Tab groups extension: loaded from ${s.from}`, steps: null };
    case "off":
      return { text: "Tab groups extension: switched off", steps: `${page} Switch it on there.` };
    case "gone":
      return { text: `Tab groups extension: loaded from ${s.path}, which no longer exists`, steps: `${page} Remove that copy. ${load}` };
    case "notLoaded":
      return { text: "Tab groups extension: not loaded, so tabs open without a tab group", steps: `${page} ${load}` };
  }
}
