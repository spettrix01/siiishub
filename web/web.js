// Browser additions to the app's interface (siiishub-server), loaded after
// the app's own scripts.
import './player.js';

const root = document.documentElement;

// The login page cannot read the user's data before signing in: it gets a
// copy of the theme from here.
function keepTheme() {
  try {
    localStorage.setItem('siiishub-web-theme', root.getAttribute('data-theme') === 'light' ? 'light' : 'dark');
    localStorage.setItem('siiishub-web-accent', root.getAttribute('data-accent') || 'orange');
  } catch {}
}
new MutationObserver(keepTheme).observe(root, { attributes: true, attributeFilter: ['data-theme', 'data-accent'] });
keepTheme();
