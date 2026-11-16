// Applies the variants the Dovetail viewer posts (ADR 0026): { dovetailVariants: { <slot>: <option> } },
// every declared slot, on load and on every change. Slot names and option strings are the ones declared.
const OPTIONS = {
  theme: ['Dusk', 'Midnight', 'Soft dark'],
  layout: ['Grid', 'List'],
  header: ['A · centred, text toggle', 'B · left logo, icon toggle', 'C · compact'],
  footer: ['Slim', 'Full'],
};
const FOOTERS = ['© Shop · Shipping · Returns', ['About', 'Shipping', 'Returns', 'Contact', 'Careers', '© Shop']];
window.addEventListener('message', (e) => {
  const v = e.data && e.data.dovetailVariants;
  if (e.origin !== location.origin || !v) return;
  const at = (slot) => Math.max(0, (OPTIONS[slot] || []).indexOf(v[slot]));
  const $ = (id) => document.getElementById(id);
  $('site').className = 'site t' + at('theme');
  $('header').className = 'h' + at('header');
  $('tog').textContent = at('header') === 0 ? 'Dark mode: on' : '☾';
  $('grid').className = 'grid' + (at('layout') === 1 ? ' list' : '');
  if (v.hero) $('hero').textContent = v.hero;
  const f = $('footer');
  f.className = at('footer') === 1 ? 'full' : '';
  f.replaceChildren(...(at('footer') === 1 ? FOOTERS[1].map((t) => Object.assign(document.createElement('span'), { textContent: t })) : [FOOTERS[0]]));
});
