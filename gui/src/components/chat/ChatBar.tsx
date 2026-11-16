// The chat bar: Insert mode types here. It shows the effort the next message is tagged with
// (Ctrl-t changes it). Enter sends, Alt-Enter is a new line (keys come from the table).

import { useEffect, useRef } from 'react';
import { effortById } from '../../state/select';
import { useSnapshot } from '../../state/store';
import { setUi, useUi } from '../../state/ui';
import { EffortDot } from './EffortTag';
import { keyText } from '../overlays/Keys';

export function ChatBar() {
  const s = useSnapshot();
  const ui = useUi();
  const input = useRef<HTMLTextAreaElement>(null);
  const insert = ui.mode === 'insert';
  const busy = !!ui.modal || !!ui.overlay;

  useEffect(() => {
    const ta = input.current;
    if (!ta) return;
    if (insert && !busy) {
      ta.focus();
      ta.setSelectionRange(ta.value.length, ta.value.length);
    } else if (!insert && document.activeElement === ta) ta.blur();
  }, [insert, busy]);

  return (
    <div className={`chatbar ${insert ? 'active' : ''}`}>
      <div className="chatbar-inner">
        <div className="to"><EffortDot effort={effortById(s, ui.tag ?? ui.effort)} /><span className="spacer" /></div>
        <textarea ref={input} rows={2} value={ui.draft}
          placeholder={insert ? `Type a message · ${keyText('send', 'insert')} sends · ${keyText('newline', 'insert')} new line` : `${keyText('insert')} to type`}
          onChange={e => setUi({ draft: e.target.value })}
          onFocus={() => { if (!insert) setUi({ mode: 'insert', zone: 'chat' }); }} />
      </div>
    </div>
  );
}
