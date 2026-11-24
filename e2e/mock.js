// Injected before the app: stands in for Tauri's IPC, so the UI runs in a plain browser. Tests drive it through window.__e2e.
(() => {
  const S = (window.__e2e = { calls: [], settled: [], fail: {}, delay: {}, responses: {}, handlers: {}, listeners: {}, cb: {} });
  let next = 0;
  const view = new URLSearchParams(location.search).get("view") || "list";
  const answer = async (cmd, args) => {
    if (S.delay[cmd]) await new Promise((r) => setTimeout(r, S.delay[cmd]));
    if (S.fail[cmd] !== undefined) throw S.fail[cmd];
    if (S.handlers[cmd]) return S.handlers[cmd](args);
    if (cmd in S.responses) return structuredClone(S.responses[cmd]);
    return null;
  };
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: view }, currentWebview: { windowLabel: view, label: view } },
    transformCallback(cb, once) {
      const i = ++next;
      S.cb[i] = (p) => {
        if (once) delete S.cb[i];
        cb(p);
      };
      return i;
    },
    unregisterCallback(i) {
      delete S.cb[i];
    },
    convertFileSrc: (p) => p,
    async invoke(cmd, args) {
      S.calls.push({ cmd, args: args === undefined ? {} : JSON.parse(JSON.stringify(args)) });
      if (cmd === "plugin:event|listen") {
        (S.listeners[args.event] ??= []).push(args.handler);
        return args.handler;
      }
      if (cmd.startsWith("plugin:")) return null;
      try {
        return await answer(cmd, args);
      } finally {
        S.settled.push(cmd);
      }
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  S.emit = (event, payload) => (S.listeners[event] || []).forEach((h) => S.cb[h]?.({ event, id: 0, payload }));
  S.callsOf = (cmd) => S.calls.filter((c) => c.cmd === cmd);
})();
