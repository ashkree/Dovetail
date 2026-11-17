import { useEffect, useState } from 'react';
import { useSnapshot } from '../../state/store';
import { needsYou, runningEfforts } from '../../state/select';
import { scopeOf, useUi } from '../../state/ui';
import { SCOPES } from '../../keys/keymap';
import { modeTarget } from '../../keys/actions';

/** How often the busy ring turns a quarter (gui/DECISIONS.md 13). */
export const SPINNER_STEP_MS = 250;

/** The "a turn is running" ring, turned a quarter every `SPINNER_STEP_MS` from here. A CSS animation made
 * WebKitGTK redo the whole page's rendering 60 times a second (~25 % of a core while any turn
 * ran); one small state change four times a second costs next to nothing. */
function Spinner() {
  const [quarter, setQuarter] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setQuarter(q => (q + 1) % 4), SPINNER_STEP_MS);
    return () => clearInterval(id);
  }, []);
  return <span className="spin" style={{ transform: `rotate(${quarter * 90}deg)` }} />;
}

const pct = (f: number) => `${Math.round(f * 100)}%`;

/** Every status, Claude-desktop style: mode, pending keys, zone, needs-you on the left; the rest on the right. */
export function BottomBar() {
  const s = useSnapshot();
  const ui = useUi();
  const ws = ui.screen === 'ws';
  const mode = ui.mode === 'insert' ? 'INSERT' : ui.leader ? 'SPACE' : 'NORMAL';
  const cls = ui.mode === 'insert' ? 'insert' : ui.leader ? 'leader' : '';
  const scope = SCOPES[scopeOf(ui)];
  const st = s.status;
  const mt = modeTarget(s, ui);
  const running = ws ? runningEfforts(s) : st.machine_running;
  // A limit of 0 (an older core sends none) shows only the count, with no warning.
  const limited = st.effort_limit > 0;
  const over = limited && running > st.effort_limit;
  const of = limited ? ` / ${st.effort_limit}` : '';
  return (
    <div className="status">
      <span className={`mode ${cls}`}>{mode}</span>
      <span className="pending">{ui.leader ? ['Space', ...ui.leader].join(' ') : ui.pending}</span>
      <span className="st">{scope?.name ?? ''}</span>
      {ws && needsYou(s) > 0 && <span className="st warn">⚠ {needsYou(s)} need you</span>}
      <span className="grow" />
      <span className={`st ${over ? 'warn' : ''}`} title="Running efforts on this machine against the soft limit">
        {ws ? `efforts ${running}${of}${over ? ' ▲' : ''}` : `${running}${of} efforts running${over ? ' ▲' : ''}`}
      </span>
      {ws && <>
        <span className="st" title="Claude profile of this project">{st.account}</span>
        <span className="st" title="Coordinator context fill"><span className="ctx-bar"><i style={{ width: pct(st.context_fill) }} /></span>ctx {pct(st.context_fill)}</span>
        <span className="st" title="Usage limits of this profile">5h {pct(st.usage.five_hour)} · week {pct(st.usage.week)}</span>
        <span className="st"><b>{st.model}</b><span className="lvl">{st.effort_level}</span></span>
        <span className="st" title="Permission mode (Shift-Tab changes it)">{mt.name} · {mt.mode}</span>
        <span className="st" title={st.busy ? 'A turn is running' : 'Idle'}>{st.busy ? <Spinner /> : null}</span>
      </>}
    </div>
  );
}
