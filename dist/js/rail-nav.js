export function wireRailNav(railList, navPrev, navNext) {
  function updateNavVisibility() {
    const overflow = railList.scrollWidth - railList.clientWidth;
    const hasOverflow = overflow > 4;
    if (!hasOverflow) {
      navPrev.hidden = true;
      navNext.hidden = true;
      return;
    }
    const x = railList.scrollLeft;
    navPrev.hidden = x <= 4;
    navNext.hidden = x >= overflow - 4;
  }
  function scrollByCards(direction) {
    const card = railList.querySelector('.trending-card');
    const step = card ? card.getBoundingClientRect().width + 14 : 200;
    railList.scrollBy({ left: direction * step * 3, behavior: 'smooth' });
  }
  navPrev.addEventListener('click', () => scrollByCards(-1));
  navNext.addEventListener('click', () => scrollByCards(1));
  railList.addEventListener('scroll', updateNavVisibility, { passive: true });
  window.addEventListener('resize', updateNavVisibility);
  return { updateNavVisibility };
}
