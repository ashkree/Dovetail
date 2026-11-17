// The coordinator chat: one per project, every message tagged with an effort. `f` filters to one
// effort's thread; slice reports are one collapsed line (`zo` opens one).

import { useEffect, useRef } from 'react';
import { dispatch } from '../../keys/actions';
import { effortById, speakerName, visibleChat } from '../../state/select';
import { useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { ChatBar } from './ChatBar';
import type { ChatEntry, Snapshot } from '../../core/model';
import { tagColour } from './EffortTag';
import { Message, ReportLine, type Shown } from './Message';
import { keyText } from '../overlays/Keys';

/** What a message shows besides its entry. */
export function shownOf(s: Snapshot, m: ChatEntry): Shown {
  const effort = effortById(s, m.effort_id);
  return { name: speakerName(s, m.who), ...(effort ? { tag: { name: effort.name, colour: tagColour(effort) } } : {}) };
}

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
            ? <ReportLine key={m.id} entry={m} from={speakerName(s, m.who)} cur={isCur} open={ui.openReports.includes(m.id)} ref={isCur ? cur : undefined} />
            : <Message key={m.id} entry={m} shown={shownOf(s, m)} cur={isCur} showTag={ui.filter === 'all'} ref={isCur ? cur : undefined} />;
        })}
        {!entries.length && <p className="dim" style={{ marginTop: 22 }}>No messages yet.</p>}
      </div>
      <ChatBar />
    </div>
  );
}
