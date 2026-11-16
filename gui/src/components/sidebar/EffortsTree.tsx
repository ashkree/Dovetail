import { useEffect, useRef } from 'react';
import { dispatch } from '../../keys/actions';
import { KIND_GLYPH, STATUS_LABEL, treeRows, type TreeRow } from '../../state/select';
import { useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { tagColour } from '../chat/EffortTag';

/** Efforts → tasks → agents → subagents. `o` folds an effort. */
export function EffortsTree() {
  const s = useSnapshot();
  const ui = useUi();
  const focus = ui.zone === 'tree' ? 'focus' : '';
  const rows = treeRows(s, ui.folded);
  const sel = useRef<HTMLDivElement>(null);
  useEffect(() => { sel.current?.scrollIntoView({ block: 'nearest' }); }, [ui.treeSel, ui.zone]);
  return (
    <div className={`part ${focus}`}>
      <div className={`zone-title ${focus}`} onClick={() => dispatch('focusZone', 'tree')}>Efforts</div>
      <div className="list">
        {rows.map((r, i) => (
          <div key={rowKey(r)} ref={i === ui.treeSel ? sel : undefined}
            className={`row ${rowClass(r, ui.effort)} ${i === ui.treeSel ? 'sel' : ''}`}
            onClick={ev => { dispatch('selectTree', i); if (ev.detail >= 2) dispatch('openRow'); }}>
            <Row r={r} folded={!!ui.folded[r.effort.id]} />
          </div>
        ))}
      </div>
    </div>
  );
}

const rowKey = (r: TreeRow) => r.type === 'effort' ? 'e' + r.effort.id : r.type === 'task' ? 't' + r.task.id : 'a' + r.agent.id;

function rowClass(r: TreeRow, current: string | null): string {
  if (r.type === 'effort') return `effort-row ${r.effort.id === current ? 'current' : ''}`;
  if (r.type === 'task') return 'indent1';
  return `indent${Math.min(4, r.depth + 2)}`;
}

function Row({ r, folded }: { r: TreeRow; folded: boolean }) {
  if (r.type === 'effort') {
    const met = r.effort.criteria.filter(c => c.state === 'met').length;
    return <>
      <span className="dim">{folded ? '▸' : '▾'}</span><span className="dot" style={{ background: tagColour(r.effort) }} />
      <span className="t">{r.effort.name}</span><span className="kind" title={r.effort.kind}>{KIND_GLYPH[r.effort.kind]}</span>
      <span className="tm">☑ {met}/{r.effort.criteria.length}</span>
    </>;
  }
  if (r.type === 'task') {
    return <><span className="t">{r.task.title}</span><span className={`status-pill st-${r.task.status}`}>{STATUS_LABEL[r.task.status]}</span></>;
  }
  const a = r.agent;
  const glyph = a.state === 'needs_you' ? <span className="needs">⚠</span> : a.state === 'done' ? <span className="good">✓</span> : <span className="agent-g">◐</span>;
  if (a.kind === 'slice') return <>{glyph}<span>◆ {a.name}</span><span className="branch t">{a.branch ?? ''}</span></>;
  return <>{glyph}<span className="t">◇ {a.name}</span></>;
}
