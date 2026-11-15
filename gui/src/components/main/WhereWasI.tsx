// "Where was I": comes up in the middle when switching effort. What the effort is, last done, what
// waits on you, next step, acceptance criteria. Esc hides it; w in the tree or Space e w brings it back.

import type { Effort } from '../../core/model';
import { dispatch } from '../../keys/actions';
import { effortById, KIND_GLYPH } from '../../state/select';
import { useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { tagColour } from '../chat/EffortTag';
import { Markdown } from '../chat/Markdown';
import { Key, LeaderKey } from '../overlays/Keys';

export function WhereWasIPopup() {
  const s = useSnapshot();
  const ui = useUi();
  const e = effortById(s, ui.effort);
  if (ui.screen !== 'ws' || !ui.wwi || !e) return null;
  return (
    <div className="wwi-pop" onClick={ev => { if (ev.target === ev.currentTarget) dispatch('escape'); }}>
      <WhereWasICard effort={e} flash={ui.wwiFlash > 0} key={`${e.id}-${ui.wwiFlash}`} />
    </div>
  );
}

const CRIT = {
  met: <span className="g-criterion">☑</span>,
  claimed: <span className="g-question">◔</span>,
  open: <span className="dim">☐</span>,
};

export function WhereWasICard({ effort: e, flash }: { effort: Effort; flash: boolean }) {
  return (
    <div className={`wwi ${flash ? 'flash' : ''}`}>
      <h3><span className="dot" style={{ background: tagColour(e) }} />Where was I · {e.name} <span className="kind">{KIND_GLYPH[e.kind]} {e.kind}</span></h3>
      <div className="goal">{e.question_or_goal}</div>
      <dl>
        <dt>Last done</dt><dd><Markdown text={e.where_was_i.last} /></dd>
        <dt>Waits on you</dt><dd><Markdown text={e.where_was_i.waits} /></dd>
        <dt>Next step</dt><dd><Markdown text={e.where_was_i.next} /></dd>
        <dt>Criteria</dt>
        <dd>
          <ul className="crit">
            {e.criteria.map(c => (
              <li key={c.id}>{CRIT[c.state]} {c.text}{c.state === 'claimed' && <span className="dim"> · in your inbox</span>}</li>
            ))}
            {!e.criteria.length && <li className="dim">None yet.</li>}
          </ul>
        </dd>
      </dl>
      <div className="foot"><Key action="escape" /> hide · <Key action="wwi" scope="tree" /> in the tree or <LeaderKey action="wwi" /> brings it back</div>
    </div>
  );
}
