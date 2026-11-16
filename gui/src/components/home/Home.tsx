// Home: every project, Google-Docs style. Recent ones as tiles, then all as a list.

import { useEffect, useRef } from 'react';
import { dispatch } from '../../keys/actions';
import { useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { Key } from '../overlays/Keys';

const RECENT = 4;
const THUMB = ['70%', '90%', '50%', '80%'];

export function Home() {
  const s = useSnapshot();
  const { homeSel } = useUi();
  const sel = useRef<HTMLTableRowElement>(null);
  useEffect(() => { sel.current?.scrollIntoView({ block: 'nearest' }); }, [homeSel]);
  return (
    <div className="home">
      <h2>Recent projects</h2>
      <div className="recent">
        {s.projects.slice(0, RECENT).map((p, i) => (
          <button key={p.id} className={`tile ${homeSel === i ? 'sel' : ''}`} onClick={() => dispatch('openProject', i)}>
            <div className="thumb">{THUMB.map((w, j) => <i key={j} style={{ width: w }} />)}</div>
            <div className="meta"><b>{p.name}</b><span className="dim">{p.opened}</span></div>
          </button>
        ))}
      </div>
      <h2>All projects</h2>
      <table className="projects">
        <thead><tr><th>Name</th><th>Repos</th><th>Account</th><th>Efforts</th><th>Needs you</th><th>Opened</th></tr></thead>
        <tbody>
          {s.projects.map((p, i) => (
            <tr key={p.id} ref={homeSel === i ? sel : undefined} className={homeSel === i ? 'sel' : ''}
              onClick={e => e.detail >= 2 ? dispatch('openProject', i) : dispatch('selectHome', i)}>
              <td><span className="pname">{p.name}</span><div className="dim" style={{ fontSize: '.88em' }}>{p.note}</div></td>
              <td>{p.repos.length ? p.repos.map(r => <span key={r} className="chip repo" style={{ marginRight: 4 }}>{r}</span>) : <span className="dim">no repo · research</span>}</td>
              <td>{p.account}</td>
              <td>{p.running} running / {p.efforts}</td>
              <td>{p.needs_you ? <span className="badge-n">{p.needs_you} ⚠</span> : <span className="dim">—</span>}</td>
              <td className="dim">{p.opened}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="dim" style={{ marginTop: 20 }}>
        <Key action="down" scope="home" />/<Key action="up" scope="home" /> move · <Key action="openProject" scope="home" /> open · <Key action="newProject" scope="home" /> new project · <Key action="keys" /> keyboard map
      </p>
    </div>
  );
}
