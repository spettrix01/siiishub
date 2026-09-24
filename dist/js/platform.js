// Platform flags. The Android build has the embedded player behind a
// transparent WebView and no window chrome: a few desktop-only controls are
// hidden through the `is-android` class on <html>. Phones and tablets get the
// mobile layout (`is-phone`); Android TV, which has no touchscreen, keeps the
// big-screen layout and is driven with the remote's D-pad (`is-tv`,
// tv-nav.js).
export const IS_ANDROID = /\bAndroid\b/i.test(navigator.userAgent);
export const IS_TV = IS_ANDROID && navigator.maxTouchPoints === 0;
export const IS_PHONE = IS_ANDROID && !IS_TV;

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
