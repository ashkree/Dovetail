// One card: kind and id, title, body; the selected card shows "↗ chat" (r) to put its reference in
// the chat bar. Used on the canvas and in the incoming strip.

import { memo } from 'react';
import type { Card, IncomingCard } from '../core/model';
import { dispatch } from '../../keys/actions';
import { keyText } from '../overlays/Keys';

export const CardView = memo(function CardView({ card, sel, under }: { card: Card | IncomingCard; sel: boolean; under?: string }) {
  const long = 'report_id' in card && !!card.report_id;
  return (
    <div className={`card ${card.kind} ${sel ? 'sel' : ''}`}>
      <div className="ct">
        {card.kind} · {card.id}{'unseen' in card && card.unseen && <span className="new-mark">new</span>}
        <button className="refbtn nodrag" title={`Insert this card's reference into the chat (${keyText('refCard', 'canvas')})`}
          onClick={e => { e.stopPropagation(); dispatch('refCardId', card.id); }}>↗ chat</button>
      </div>
      <div className="ttl">{card.title}</div>
      <div className="bd">
        {card.body}{long && <span className="dim"> · {keyText('openCard', 'canvas')} reads it</span>}
        {under && <span className="under">goes under {under}</span>}
      </div>
    </div>
  );
});
