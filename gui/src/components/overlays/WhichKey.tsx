// The which-key pop-up while the Space leader is active: the keys one level down, from the table.

import { dispatch } from '../../keys/actions';
import { leaderNode } from '../../keys/resolve';
import { keyName } from '../../keys/resolve';
import { setUi, useUi } from '../../state/ui';

export function WhichKey() {
  const { leader } = useUi();
  if (!leader) return null;
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
