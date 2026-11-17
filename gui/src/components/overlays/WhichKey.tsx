// The which-key pop-up: the keys one level down while the Space leader is active, or after a
// prefix the keymap names in PREFIX_POPUPS (`b` on the canvas). All from the table.

import { dispatch } from '../../keys/actions';
import { PREFIX_POPUPS } from '../../keys/keymap';
import { bindingsFor, leaderNode } from '../../keys/resolve';
import { keyName } from '../../keys/resolve';
import { scopeOf, setUi, useUi } from '../../state/ui';

export function WhichKey() {
  const ui = useUi();
  const { leader } = ui;
  if (!leader) return <PrefixKeys scope={scopeOf(ui)} pending={ui.pending} />;
  const node = leaderNode(leader);
  if (!node) return null;
  return (
    <div className="whichkey">
      <h4>Space{leader.map(k => ' ' + k).join('')}{leader.length ? ' · ' + node.name : ''}</h4>
      <div className="wk">
        {Object.entries(node.keys).map(([k, v]) => {
          const leaf = Array.isArray(v);
          const run = () => {
            if (leaf) { setUi({ leader: null }); dispatch(v[0]); } else setUi({ leader: [...leader, k] });
          };
          return [
            <kbd key={k + 'k'}>{keyName(k)}</kbd>,
            <button key={k} className={leaf ? '' : 'grp-n'} onClick={run}>{leaf ? v[1] : `${v.name} ▸`}</button>,
          ];
        })}
      </div>
    </div>
  );
}

/** The keys that finish a pending prefix the keymap pops up for. */
function PrefixKeys({ scope, pending }: { scope: string; pending: string }) {
  const name = PREFIX_POPUPS[scope]?.[pending];
  if (!name) return null;
  const rest = bindingsFor(scope).filter(b => b[0].startsWith(pending) && b[0] !== pending);
  return (
    <div className="whichkey">
      <h4>{keyName(pending)} · {name}</h4>
      <div className="wk">
        {rest.map(([k, action, label]) => [
          <kbd key={k + 'k'}>{keyName(k.slice(pending.length))}</kbd>,
          <button key={k} onClick={() => { setUi({ pending: '' }); dispatch(action); }}>{label.startsWith(`${name}: `) ? label.slice(name.length + 2) : label}</button>,
        ])}
      </div>
    </div>
  );
}
