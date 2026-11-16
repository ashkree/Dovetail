// The one modal: the picker (Space Space and the switchers), prompts, the `:` command line and
// forms (new project, add repo, new slice). Keys are handled by useKeys; this draws it and takes the mouse.

import { useEffect, useRef } from 'react';
import { useSnapshot } from '../../state/store';
import { setUi, useUi, type Modal, type PickItem } from '../../state/ui';
import { pickerRows } from '../../keys/useKeys';
import { EffortTag } from '../chat/EffortTag';

export function Picker() {
  const { modal } = useUi();
  const input = useRef<HTMLInputElement>(null);
  const sel = modal?.type === 'form' ? modal.sel : 0;
  useEffect(() => { input.current?.focus(); }, [modal?.type, modal && 'title' in modal ? modal.title : '', sel]);
  if (!modal) return null;
  if (modal.type === 'form') {
    const set = (i: number, value: string) =>
      setUi({ modal: { ...modal, fields: modal.fields.map((f, j) => (j === i ? { ...f, value } : f)) } });
    return (
      <div className="modal">
        <div className="ttl form">{modal.title}</div>
        {modal.fields.map((f, i) => (
          <div key={f.label} className={`row field ${i === modal.sel ? 'sel' : ''}`}>
            <div className="dim" style={{ fontSize: '.85em' }}>{f.label}</div>
            <input ref={i === modal.sel ? input : undefined} value={f.value} placeholder={f.hint}
              onFocus={() => i !== modal.sel && setUi({ modal: { ...modal, sel: i } })}
              onChange={e => set(i, e.target.value)} />
          </div>
        ))}
        <div className="row field dim" style={{ padding: '12px 16px' }}>
          <kbd>Tab</kbd> next field · <kbd>Enter</kbd> create · <kbd>Esc</kbd> cancel
        </div>
      </div>
    );
  }
  if (modal.type === 'command') {
    return (
      <div className="modal">
        <div className="cmd"><span className="prefix">:</span>
          <input ref={input} value={modal.value} onChange={e => setUi({ modal: { ...modal, value: e.target.value } })} />
        </div>
      </div>
    );
  }
  if (modal.type === 'prompt') {
    return (
      <div className="modal">
        <div className="ttl">{modal.title}</div>
        <input ref={input} value={modal.value} onChange={e => setUi({ modal: { ...modal, value: e.target.value } })} />
      </div>
    );
  }
  return <PickerList modal={modal} input={input} />;
}

function PickerList({ modal, input }: { modal: Extract<Modal, { type: 'picker' }>; input: React.RefObject<HTMLInputElement | null> }) {
  const rows = pickerRows(modal);
  return (
    <div className="modal">
      <div className="ttl">{modal.title}</div>
      <input ref={input} value={modal.q} placeholder="type to filter · Ctrl-j/Ctrl-k or arrows · Enter · Esc"
        onChange={e => setUi({ modal: { ...modal, q: e.target.value, sel: 0 } })} />
      <div className="rows">
        {rows.map((it, i) => (
          <div key={it.key} className={`row ${i === modal.sel ? 'sel' : ''}`} onClick={() => { setUi({ modal: null }); it.go(); }}>
            <PickRow item={it} />
          </div>
        ))}
      </div>
    </div>
  );
}

function PickRow({ item }: { item: PickItem }) {
  const s = useSnapshot();
  return <>
    {item.glyph && <span className={item.glyphClass ?? ''}>{item.glyph}</span>}
    {item.effort ? <EffortTag effort={s.efforts.find(e => e.id === item.effort)} /> : <span className={item.detail ? '' : 't'}>{item.label}</span>}
    {item.detail && <span className="t dim">{item.detail}</span>}
  </>;
}
