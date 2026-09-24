// Platform flags. The interface served by siiishub-server to a browser
// (web/bridge.js stands in for Tauri): no window, no remote control, the
// backend on the server.
export const IS_WEB = !!window.__SIIISHUB_WEB__;
// The Android build has the embedded player behind a transparent WebView and
// no window chrome: a few desktop-only controls are hidden through the
// `is-android` class on <html>. The phone APK has the mobile layout
// (`is-phone`); the TV APK (lib.rs sets __SIIISHUB_TV__) has the interface of
// Windows and Linux, driven with the remote's D-pad (`is-tv`, tv-nav.js), and
// so has the phone APK on a device without a touchscreen. The app, not a
// browser on Android: there the page is the web interface, the same as on
// Windows and Linux.
export const IS_ANDROID = !IS_WEB && /\bAndroid\b/i.test(navigator.userAgent);
export const IS_TV = IS_ANDROID && (!!window.__SIIISHUB_TV__ || navigator.maxTouchPoints === 0);
export const IS_PHONE = IS_ANDROID && !IS_TV;

if (IS_WEB) document.documentElement.classList.add('is-web');

if (IS_ANDROID) {
  document.documentElement.classList.add('is-android', IS_TV ? 'is-tv' : 'is-phone');
  // No pinch / double-tap zoom of the page: the layout is already sized for
  // the display. The Android WebView honours user-scalable=no (the plugin
  // also turns zoom off in the WebView settings).
  const viewport = document.querySelector('meta[name="viewport"]');
  if (viewport) {
    viewport.setAttribute('content', 'width=device-width,initial-scale=1,maximum-scale=1,user-scalable=no,viewport-fit=cover');
  }
}
