// Platform flags. The Android build has the embedded player behind a
// transparent WebView and no window chrome: a few desktop-only controls are
// hidden and the mobile layout is enabled through the `is-android` class on
// <html>.
export const IS_ANDROID = /\bAndroid\b/i.test(navigator.userAgent);

if (IS_ANDROID) {
  document.documentElement.classList.add('is-android');
  // No pinch / double-tap zoom of the page: the layout is already sized for
  // the display. The Android WebView honours user-scalable=no (the plugin
  // also turns zoom off in the WebView settings).
  const viewport = document.querySelector('meta[name="viewport"]');
  if (viewport) {
    viewport.setAttribute('content', 'width=device-width,initial-scale=1,maximum-scale=1,user-scalable=no,viewport-fit=cover');
  }
}
