// The chat bar: Insert mode types here. The coordinator's shows the effort the next message is
// tagged with (Ctrl-t changes it); an agent view's types to that agent, tagged with its task's
// effort. Enter sends, Alt-Enter is a new line (keys come from the table).

import { useEffect, useRef } from 'react';
import type { Agent } from '../../core/model';
import { effortById } from '../../state/select';
import { useSnapshot } from '../../state/store';
import { barAgent, setUi, useUi } from '../../state/ui';
import { EffortDot } from './EffortTag';
import { keyText } from '../overlays/Keys';

export function ChatBar({ agent }: { agent?: Agent }) {
  const s = useSnapshot();
  const ui = useUi();
  const input = useRef<HTMLTextAreaElement>(null);
  const mine = barAgent(ui) === (agent?.id ?? null);
  const insert = ui.mode === 'insert' && mine;
  const busy = !!ui.modal || !!ui.overlay;
  const draft = agent ? ui.agentDraft : ui.draft;

  useEffect(() => {
    const ta = input.current;
    if (!ta) return;
    if (insert && !busy) {
      ta.focus();
      ta.setSelectionRange(ta.value.length, ta.value.length);
    } else if (!insert && document.activeElement === ta) ta.blur();
  }, [insert, busy]);

  const effort = effortById(s, agent ? agent.effort_id : ui.tag ?? ui.effort);
  return (
    <div className={`chatbar ${insert ? 'active' : ''}`}>
      <div className="chatbar-inner">
        <div className="to">
          {agent && <span>to {agent.name}</span>}
          <EffortDot effort={effort} /><span className="spacer" />
        </div>
        <textarea ref={input} rows={2} value={draft}
          placeholder={insert ? `Type a message · ${keyText('send', 'insert')} sends · ${keyText('newline', 'insert')} new line` : `${keyText('insert')} to type`}
          onChange={e => setUi(agent ? { agentDraft: e.target.value } : { draft: e.target.value })}
          onFocus={() => { if (!insert) setUi(agent ? { mode: 'insert', zone: 'main' } : { mode: 'insert', zone: 'chat' }); }} />
      </div>
    </div>
  );
}
