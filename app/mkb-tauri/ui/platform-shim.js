// mkb platform shim — lets the ONE shared UI run either inside the Tauri desktop shell or in a
// plain browser (served by `mkb-web`), with no forked front-end.
//
// Under Tauri (`withGlobalTauri: true`), `window.__TAURI__` is already injected before this runs, so
// we detect it and do nothing. In a browser it's absent, so we install a minimal `window.__TAURI__`
// whose surface matches exactly what the UI uses:
//   - core.invoke(cmd, args)  -> POST /api/<cmd>  (with an X-MKB-Vault header for this tab's vault)
//   - core.convertFileSrc(p)  -> /assets?vault=<v>&path=<p>   (vault image → HTTP, server-scoped)
//   - event.listen(evt, cb)   -> one EventSource('/sse?vault=<v>')   (daemon push → SSE)
//
// Per-tab vaults: each tab remembers its active vault in sessionStorage (per-tab, survives reload,
// isolated from other tabs). `switch_vault` is handled *client-side* — it repoints THIS tab and
// reopens its SSE, without touching the server's global state or other tabs. The launch default is
// never changed by switching (that's `set_default_vault`).
//
// vault-changed de-dup: the server sends the current generation as the first SSE event on every
// (re)connect. We forward a refresh to the UI only when the generation actually differs from the
// last one we saw — so a plain reconnect (e.g. returning from a backgrounded phone tab) refreshes
// only if something changed while we were away, and the initial page load doesn't double-refresh.
//
// This file is bundled by the desktop app too (it's in ui/), where it is a deliberate no-op.
(function () {
  "use strict";

  // Real Tauri present → leave everything to it.
  if (
    window.__TAURI__ &&
    window.__TAURI__.core &&
    typeof window.__TAURI__.core.invoke === "function"
  ) {
    return;
  }

  var VAULT_KEY = "mkb.vault";

  function activeVault() {
    try {
      return sessionStorage.getItem(VAULT_KEY) || null;
    } catch (_) {
      return null;
    }
  }
  function setActiveVault(name) {
    try {
      if (name) sessionStorage.setItem(VAULT_KEY, name);
      else sessionStorage.removeItem(VAULT_KEY);
    } catch (_) {
      /* private mode / disabled storage: fall back to in-memory */
    }
    _memVault = name || null;
  }
  // Fallback when sessionStorage is unavailable (Safari private mode edge cases).
  var _memVault = null;
  function currentVault() {
    return activeVault() || _memVault;
  }

  function tryParse(text) {
    try {
      return JSON.parse(text);
    } catch (_) {
      return text;
    }
  }

  async function invoke(cmd, args) {
    // The OS folder picker has no browser equivalent; ask for a path so a local vault can still be
    // added by typing its path. Returning null mirrors the user cancelling the dialog.
    if (cmd === "pick_vault") {
      var p = window.prompt("Enter the full path to the vault folder:");
      return p && p.trim() ? p.trim() : null;
    }

    // Switching vaults is a per-tab, client-side operation: repoint this tab and reopen its live
    // stream. No server round-trip, no effect on the launch default or on other tabs. The UI's own
    // post-switch reload then re-fetches everything (now carrying the new vault header).
    if (cmd === "switch_vault") {
      var name = args && args.name;
      if (name) {
        setActiveVault(name);
        reopenEventSource();
      }
      return null;
    }

    var headers = { "content-type": "application/json" };
    var v = currentVault();
    if (v) headers["X-MKB-Vault"] = v;

    var res = await fetch("/api/" + encodeURIComponent(cmd), {
      method: "POST",
      headers: headers,
      body: JSON.stringify(args || {}),
    });

    // Tauri's invoke() REJECTS with the command's Err value; mirror that so existing try/catch works.
    if (!res.ok) {
      var errText = await res.text();
      throw tryParse(errText);
    }
    // A unit-returning command sends JSON `null`; invoke() resolving to null/undefined matches Tauri.
    var text = await res.text();
    return text ? tryParse(text) : undefined;
  }

  function convertFileSrc(path /*, protocol */) {
    var v = currentVault();
    var q = "path=" + encodeURIComponent(path);
    if (v) q = "vault=" + encodeURIComponent(v) + "&" + q;
    return "/assets?" + q;
  }

  // ----- live refresh (SSE) -----
  // A single EventSource per tab, reopened on vault switch. The UI registers one `vault-changed`
  // handler (via event.listen); we fire it on a real change AND on a reconnect. EventSource
  // auto-reconnects on its own after a drop (e.g. a phone returning from the background); on that
  // reconnect we refresh once to catch anything that changed while we were away. The server seeds
  // its watcher to the current generation, so a fresh connect never spuriously fires.
  var _es = null;
  var _listeners = {}; // event name -> [handlers]
  var _openedBefore = false; // has this tab's SSE connected at least once?

  function sseUrl() {
    var v = currentVault();
    return v ? "/sse?vault=" + encodeURIComponent(v) : "/sse";
  }

  function fire(evt, payload) {
    (_listeners[evt] || []).forEach(function (h) {
      try {
        h({ event: evt, payload: payload });
      } catch (_) {}
    });
  }

  function eventSource() {
    if (_es) return _es;
    _es = new EventSource(sseUrl());
    _es.addEventListener("vault-changed", function (e) {
      fire("vault-changed", tryParse(e.data));
    });
    _es.onopen = function () {
      // First open = initial load (already fresh). A later open = a reconnect → catch up.
      if (_openedBefore) fire("vault-changed", null);
      _openedBefore = true;
    };
    return _es;
  }

  // On a vault switch the UI reloads everything itself, so reset the "connected before" flag and
  // reconnect to the new vault's stream without triggering a redundant catch-up refresh.
  function reopenEventSource() {
    _openedBefore = false;
    if (_es) {
      try {
        _es.close();
      } catch (_) {}
      _es = null;
    }
    eventSource();
  }

  // Tauri's event.listen(evt, handler) resolves to an unlisten function; the handler receives
  // { event, payload }. Registers the handler and ensures the EventSource is running.
  function listen(evt, handler) {
    (_listeners[evt] = _listeners[evt] || []).push(handler);
    eventSource();
    return Promise.resolve(function () {
      var arr = _listeners[evt] || [];
      var i = arr.indexOf(handler);
      if (i >= 0) arr.splice(i, 1);
    });
  }

  window.__TAURI__ = {
    core: { invoke: invoke, convertFileSrc: convertFileSrc },
    event: { listen: listen },
  };
  // Marks this as the browser build (not the real Tauri desktop shell), so the UI can hide
  // desktop-only affordances (e.g. the Settings "Web UI" launcher) when running as a web page.
  window.__MKB_WEB__ = true;
})();
