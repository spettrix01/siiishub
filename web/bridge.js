// SIIISHUB in the browser: stands in for the Tauri runtime the app's
// interface expects (window.__TAURI__). The server injects this script into
// index.html before the app's own scripts, which read __TAURI__ as they load.
//
// - core.invoke(command, args): POST /api/invoke/<command>, which runs the
//   same backend code as the app's Tauri command; a failure rejects with the
//   error string, as Tauri does. Commands about the window, the remote
//   control or the desktop are answered here.
// - event.listen(name, handler): backend events arrive over one WebSocket
//   (/api/events), reconnecting when it drops.
// - window.getCurrentWindow(): no window to move or close in a browser; full
//   screen is the page's.
(() => {
  'use strict';

  window.__SIIISHUB_WEB__ = true;

  // ---------- Events ----------
  const listeners = new Map();
  let retries = 0;

  function dispatchEvent(name, payload) {
    const handlers = listeners.get(name);
    if (!handlers) return;
    for (const handler of [...handlers]) {
      try {
        handler({ event: name, payload, id: 0 });
      } catch (err) {
        console.error(`[bridge] listener of ${name} failed`, err);
      }
    }
  }

  function connect() {
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = new WebSocket(`${proto}//${location.host}/api/events`);
    socket.addEventListener('open', () => { retries = 0; });
    socket.addEventListener('message', (e) => {
      let msg;
      try { msg = JSON.parse(e.data); } catch { return; }
      if (msg && typeof msg.event === 'string') dispatchEvent(msg.event, msg.payload);
    });
    socket.addEventListener('close', () => {
      const delay = Math.min(10000, 500 * 2 ** retries++);
      setTimeout(connect, delay);
    });
  }

  async function listen(name, handler) {
    if (!listeners.has(name)) listeners.set(name, new Set());
    listeners.get(name).add(handler);
    return () => listeners.get(name)?.delete(handler);
  }

  // ---------- Page-side commands ----------
  function openUrl(url) {
    if (!/^https?:\/\//i.test(String(url))) throw new Error('unsupported address');
    window.open(url, '_blank', 'noopener,noreferrer');
  }

  async function setFullscreen(on) {
    const el = document.documentElement;
    if (on && !document.fullscreenElement) await el.requestFullscreen?.();
    else if (!on && document.fullscreenElement) await document.exitFullscreen?.();
    return null;
  }

  // Things of the app on a PC or a phone that have no meaning here.
  const NOT_IN_BROWSER = JSON.stringify({ code: 'error.web.notInBrowser' });
  const pageCommands = {
    remote_info: () => ({ running: false, port: 0, interfaces: [], clients: 0, devices: [] }),
    remote_push_state: () => null,
    remote_set_approval: () => null,
    remote_remember_device: () => null,
    remote_forget_device: () => null,
    player_mode: () => null,
    window_set_fullscreen: (args) => setFullscreen(!!args.fullscreen),
    open_url: (args) => { openUrl(args.url); return null; },
    open_download_dir: () => { throw NOT_IN_BROWSER; },
    download_open_folder: () => { throw NOT_IN_BROWSER; },
    download_add_local: () => { throw NOT_IN_BROWSER; },
  };

  async function invoke(command, args) {
    args = args || {};
    if (Object.prototype.hasOwnProperty.call(pageCommands, command)) {
      return pageCommands[command](args);
    }
    // The embedded mpv of the app: the browser's player takes these once it
    // is loaded (web/player.js). Without it, playback stops before the
    // server resolves the stream, which would start a torrent for nothing.
    const player = window.__SIIISHUB_PLAYER__;
    if (command.startsWith('mpv_')) {
      if (player) return player.command(command, args);
      throw JSON.stringify({ code: 'error.web.noPlayer' });
    }
    if (!player && (command === 'media_resolve' || command === 'download_play')) {
      throw JSON.stringify({ code: 'error.web.noPlayer' });
    }
    let res;
    try {
      res = await fetch(`/api/invoke/${encodeURIComponent(command)}`, {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(args),
      });
    } catch (err) {
      throw `Server unreachable: ${err.message || err}`;
    }
    if (res.status === 401) {
      location.href = '/login';
      throw 'unauthorized';
    }
    const body = await res.json().catch(() => null);
    if (!res.ok) throw (body && typeof body.error === 'string') ? body.error : `HTTP ${res.status}`;
    return body;
  }

  // ---------- Window ----------
  const currentWindow = {
    minimize: async () => {},
    maximize: async () => {},
    unmaximize: async () => {},
    toggleMaximize: async () => {},
    isMaximized: async () => false,
    close: async () => {},
    isFullscreen: async () => !!document.fullscreenElement,
    setFullscreen: (on) => setFullscreen(!!on),
  };

  window.__TAURI__ = {
    core: { invoke },
    event: {
      listen,
      emit: async (name, payload) => dispatchEvent(name, payload),
    },
    window: { getCurrentWindow: () => currentWindow },
    shell: { open: async (url) => openUrl(url) },
    // For the browser's player: events the embedded mpv would send.
    __emitLocal: dispatchEvent,
  };

  connect();
})();
