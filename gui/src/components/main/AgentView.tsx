// A slice's or subagent's own conversation in the main view: streamed like the coordinator chat,
// its tool activity as one dim line per turn, what it waits on you for above its own chat bar
// (answered with the inbox keys), and `i` types to it.

import { useEffect, useRef } from 'react';
import { dispatch } from '../../keys/actions';
import { AGENT_GLYPH, agentEntries, agentItems, INBOX_GLYPH, sessionOf } from '../../state/select';
import { useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { ChatBar } from '../chat/ChatBar';
import { Markdown } from '../chat/Markdown';
import { shownOf } from '../chat/Chat';
import { Message } from '../chat/Message';
import { keyText } from '../overlays/Keys';

const STATE_TEXT = { working: 'working', needs_you: 'needs you', done: 'done', idle: 'idle' } as const;

export function AgentView() {
  const s = useSnapshot();
  const ui = useUi();
  const agent = s.agents.find(a => a.id === ui.agent);
  const entries = agentEntries(s, ui.agent);
  const items = agentItems(s, ui.agent);
  const more = !!(ui.agent && s.conversations[ui.agent]?.has_more);
  const scroll = useRef<HTMLDivElement>(null);
  const cur = useRef<HTMLDivElement>(null);
  const last = entries.at(-1);

  useEffect(() => {
    if (ui.agentCur >= 0) cur.current?.scrollIntoView({ block: 'nearest' });
    else if (scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [ui.agentCur, entries.length, last?.text.length, items.length, ui.agent]);

  if (!agent) return <div className="empty-main">This agent has ended. {keyText('escape')} goes back.</div>;
  const slice = sessionOf(s, agent);
  const parent = slice === agent ? undefined : slice;
  const focus = ui.zone === 'main';
  return (
    <div className="agentview">
      <div className="agent-head">
        <span className="who">{AGENT_GLYPH[agent.kind]} {agent.name}</span>
        {parent && <span className="dim">of {parent.name}</span>}
        {agent.branch && <span className="branch">{agent.branch}</span>}
        <span className={`dim st-${agent.state}`}>{STATE_TEXT[agent.state]}</span>
        <span className="spacer" />
        <span className="dim">{agent.mode}</span>
      </div>
      <div className="chat-scroll" ref={scroll}>
        {more && <p className="dim">Older messages: {keyText('top', 'agent')}</p>}
        {entries.map((m, i) => {
          const isCur = focus && ui.agentCur === i;
          return m.tool
            ? <div key={m.id} ref={isCur ? cur : undefined} className={`tool-line ${isCur ? 'cur' : ''}`}>⚙ {m.text}</div>
            : <Message key={m.id} entry={m} shown={shownOf(s, m)} cur={isCur} showTag={false} ref={isCur ? cur : undefined} />;
        })}
        {!entries.length && <p className="dim" style={{ marginTop: 22 }}>No messages yet.</p>}
      </div>
      {items.map((it, i) => (
        <div key={it.id} className={`agent-item ${i === 0 && focus ? 'first' : ''}`}>
          <div className="item-head">
            <span className={`glyph g-${it.kind}`}>{INBOX_GLYPH[it.kind]}</span>
            <span>{it.title}</span><span className="spacer" /><span className="dim">{it.at}</span>
          </div>
          {i === 0 && <>
            <Markdown text={it.body} />
            <div className="answers">
              {it.kind === 'permission'
                ? <>
                  <button className="primary" onClick={() => dispatch('allow')}>{keyText('allow', 'agent')} · allow once</button>
                  <button onClick={() => dispatch('allowAlways')}>{keyText('allowAlways', 'agent')} · always allow</button>
                  <button onClick={() => dispatch('deny')}>{keyText('deny', 'agent')} · deny</button>
                  <button onClick={() => dispatch('denyReason')}>{keyText('denyReason', 'agent')} · with reason</button>
                </>
                : <>
                  {it.kind === 'question' && <button className="primary" onClick={() => dispatch('reply')}>{keyText('reply', 'agent')} · reply</button>}
                  <button onClick={() => dispatch('dismiss')}>{keyText('dismiss', 'agent')} · dismiss</button>
                </>}
            </div>
          </>}
        </div>
      ))}
      {/* A subagent runs inside its slice's turn: messages go to the slice. */}
      {parent ? <p className="dim agent-note">Type to its slice, {parent.name}: {keyText('openConversation', 'tree')} on it in the tree.</p> : <ChatBar agent={agent} />}
    </div>
  );
}
