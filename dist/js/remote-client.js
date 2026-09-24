// Android: the phone as the remote control of SIIISHUB on a PC. The QR code
// in the PC's settings (Remote) holds the address of the phone page the PC
// serves (remote.rs, remote_page.html). The app opens that page full screen
// in a frame, so approval, pairing and every command work as in the phone's
// browser and always match the version running on the PC. The overlay first
// shows the camera to frame the code, then the page; android-back.js closes
// it on Back.
import { $ } from './dom.js';
import { t } from './i18n.js';

const DEFAULT_PORT = 9871;
const PROBE_TIMEOUT_MS = 4000;
const SCAN_INTERVAL_MS = 200;

const overlay = $('#remoteOverlay');
const hostLabel = $('#remoteOverlayHost');
const scanVideo = $('#remoteScanVideo');
const scanHint = $('#remoteScanHint');
const frameSlot = $('#remoteFrameSlot');

/** `{ host, url }` for an address typed or scanned (`ip`, `ip:port` or the
 *  `http://ip:port/` of the QR code), null when it cannot be a SIIISHUB PC. */
export function parseRemoteAddress(raw) {
  let s = String(raw || '').trim();
  if (!s) return null;
  if (!/^[a-z][a-z\d+.-]*:\/\//i.test(s)) s = `http://${s}`;
  let u;
  try { u = new URL(s); } catch { return null; }
  if (u.protocol !== 'http:' || u.username || u.password) return null;
  if (u.pathname !== '/' || u.search || u.hash) return null;
  // Chromium percent-encodes what it cannot put in a host instead of
  // rejecting it: only a name, an IPv4 or a bracketed IPv6 address.
  if (!/^[a-z\d.-]+$/i.test(u.hostname) && !/^\[[\da-f:.]+\]$/i.test(u.hostname)) return null;
  const host = `${u.hostname}:${u.port || DEFAULT_PORT}`;
  return { host, url: `http://${host}/` };
}

let stream = null;
let scanTimer = 0;
let scanDone = null; // settles the pending scanRemoteQr()
let session = 0; // bumped on close: stale camera and probe results are dropped

function showOverlay(mode, host = '') {
  overlay.dataset.mode = mode;
  hostLabel.textContent = host;
  overlay.hidden = false;
}

function stopCamera() {
  clearTimeout(scanTimer);
  scanTimer = 0;
  stream?.getTracks().forEach(track => track.stop());
  stream = null;
  scanVideo.srcObject = null;
}

export function closeRemoteOverlay() {
  session++;
  stopCamera();
  frameSlot.replaceChildren(); // drops the page and its connection to the PC
  overlay.hidden = true;
  scanDone?.(null);
}

/** True when the camera and the WebView can read QR codes. */
export async function canScanQr() {
  if (!('BarcodeDetector' in window) || !navigator.mediaDevices?.getUserMedia) return false;
  try {
    return (await BarcodeDetector.getSupportedFormats()).includes('qr_code');
  } catch {
    return false;
  }
}

/** Shows the camera until a SIIISHUB QR code is framed. Resolves with its
 *  address, leaving the overlay open for connectRemote(), or with null when
 *  the user closes it; rejects when the camera cannot be used. */
export async function scanRemoteQr() {
  const mine = ++session;
  const detector = new BarcodeDetector({ formats: ['qr_code'] });
  scanHint.textContent = t('settings.remoteClient.scanHint');
  showOverlay('scan');
  let media;
  try {
    media = await navigator.mediaDevices.getUserMedia({
      video: { facingMode: { ideal: 'environment' }, width: { ideal: 1280 }, height: { ideal: 720 } },
      audio: false,
    });
  } catch (e) {
    if (mine === session) closeRemoteOverlay();
    throw e;
  }
  if (mine !== session) {
    // Closed while Android was asking for the camera permission.
    media.getTracks().forEach(track => track.stop());
    return null;
  }
  stream = media;
  scanVideo.srcObject = media;
  scanVideo.play().catch(() => {});
  return new Promise(resolve => {
    scanDone = entry => {
      scanDone = null;
      resolve(entry);
    };
    const tick = async () => {
      if (mine !== session || !stream) return;
      try {
        for (const code of await detector.detect(scanVideo)) {
          const entry = parseRemoteAddress(code.rawValue);
          if (entry && mine === session) {
            stopCamera();
            scanDone?.(entry);
            return;
          }
          scanHint.textContent = t('settings.remoteClient.notSiiishub');
        }
      } catch { /* no frame yet */ }
      if (mine === session && stream) scanTimer = setTimeout(tick, SCAN_INTERVAL_MS);
    };
    tick();
  });
}

/** True when something answers at the address: the PC's remote server. */
async function reachable(url) {
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), PROBE_TIMEOUT_MS);
  try {
    await fetch(url, { mode: 'no-cors', cache: 'no-store', signal: ctl.signal });
    return true;
  } catch {
    return false;
  } finally {
    clearTimeout(timer);
  }
}

/** Opens the remote of the PC once it answers: 'ok', 'unreachable' (the
 *  overlay is closed) or 'cancelled' when the user closed it meanwhile. */
export async function connectRemote(entry) {
  const mine = overlay.hidden ? ++session : session;
  if (!overlay.hidden) scanHint.textContent = t('settings.remoteClient.connecting', { host: entry.host });
  const ok = await reachable(entry.url);
  if (mine !== session) return 'cancelled';
  if (!ok) {
    closeRemoteOverlay();
    return 'unreachable';
  }
  const frame = document.createElement('iframe');
  frame.className = 'remote-frame';
  frame.title = t('settings.pane.remote');
  frame.src = entry.url;
  frameSlot.replaceChildren(frame);
  showOverlay('remote', entry.host);
  return 'ok';
}

overlay?.querySelector('[data-remote-close]')?.addEventListener('click', closeRemoteOverlay);

// Leaving the app while framing the code releases the camera.
document.addEventListener('visibilitychange', () => {
  if (document.hidden && stream) closeRemoteOverlay();
});
