// Android genre picker: the options open as a popup centred on the screen
// (CSS, `html.is-android .genre-menu`); this adds the backdrop that dims the
// page while it is open. A tap on the backdrop lands outside the dropdown,
// so topbar.js closes the menu, and nothing underneath receives the tap.
import { $ } from './dom.js';
import { IS_ANDROID } from './platform.js';

const menu = $('#genreMenu');

if (menu && IS_ANDROID) {
  let backdrop = null;
  const sync = () => {
    if (menu.hidden) {
      backdrop?.remove();
      backdrop = null;
      return;
    }
    if (backdrop) return;
    backdrop = document.createElement('div');
    backdrop.className = 'genre-backdrop';
    document.body.appendChild(backdrop);
  };
  new MutationObserver(sync).observe(menu, { attributes: true, attributeFilter: ['hidden'] });
  sync();
}
