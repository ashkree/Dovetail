// The coordinator chat: one per project, every message tagged with an effort. `f` filters to one
// effort's thread; slice reports are one collapsed line (`zo` opens one).

import { useEffect, useRef } from 'react';
import { dispatch } from '../../keys/actions';
import { effortById, visibleChat } from '../../state/select';
import { useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { ChatBar } from './ChatBar';
import { tagColour } from './EffortTag';
import { Message, ReportLine } from './Message';
import { keyText } from '../overlays/Keys';

export function Chat() {
  const s = useSnapshot();
  const ui = useUi();
  const project = s.projects.find(p => p.id === s.open_project);
  const entries = visibleChat(s, ui.filter);
  const scroll = useRef<HTMLDivElement>(null);
  const cur = useRef<HTMLDivElement>(null);
  const filtered = ui.filter === 'all' ? null : effortById(s, ui.filter);

  useEffect(() => {
    if (ui.chatCur >= 0) cur.current?.scrollIntoView({ block: 'nearest' });
    else if (scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [ui.chatCur, entries.length, ui.filter, ui.openReports.length, ui.main]);

  return (
    <div className={`zone chat-zone ${ui.zone === 'chat' ? 'focus' : ''}`}>
      <div className="zone-title" onClick={() => dispatch('focusZone', 'chat')}>Coordinator · {project?.name}</div>
      <div className="filter-min">
        <button onClick={() => dispatch('pickFilter')}>
          {filtered ? <><span className="dot" style={{ background: tagColour(filtered) }} /> {filtered.name}</> : 'All efforts'} ▾
        </button>
        <span className="dim">{keyText('cycleFilter', 'chat')}</span>
      </div>
      <div className="chat-scroll" ref={scroll}>
        {entries.map((m, i) => {
          const isCur = ui.zone === 'chat' && ui.chatCur === i;
          return m.report
            ? <ReportLine key={m.id} entry={m} cur={isCur} open={ui.openReports.includes(m.id)} ref={isCur ? cur : undefined} />
            : <Message key={m.id} entry={m} cur={isCur} showTag={ui.filter === 'all'} ref={isCur ? cur : undefined} />;
        })}
        {!entries.length && <p className="dim" style={{ marginTop: 22 }}>No messages yet.</p>}
      </div>
      <ChatBar />
    </div>
  );
}
