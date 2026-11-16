// Applies the variants the Dovetail GUI posts: { type: 'dovetail-variants', choice: { slotId: index }, text: { slotId: label } }.
const FOOTERS = ['© Shop · Shipping · Returns', ['About', 'Shipping', 'Returns', 'Contact', 'Careers', '© Shop']];
window.addEventListener('message', (e) => {
  if (e.origin !== location.origin || !e.data || e.data.type !== 'dovetail-variants') return;
  const c = e.data.choice || {};
  const $ = (id) => document.getElementById(id);
  $('site').className = 'site t' + (c.theme || 0);
  $('header').className = 'h' + (c.header || 0);
  $('tog').textContent = (c.header || 0) === 0 ? 'Dark mode: on' : '☾';
  $('grid').className = 'grid' + (c.layout === 1 ? ' list' : '');
  if (e.data.text && e.data.text.hero) $('hero').textContent = e.data.text.hero;
  const f = $('footer');
  f.className = c.footer === 1 ? 'full' : '';
  f.replaceChildren(...(c.footer === 1 ? FOOTERS[1].map((t) => Object.assign(document.createElement('span'), { textContent: t })) : [FOOTERS[0]]));
});
