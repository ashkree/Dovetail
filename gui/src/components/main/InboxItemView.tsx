// An inbox item in full in the main view. A permission request answers with y A d D; a criterion
// review with y (approve: Dovetail ticks it) or d / D (send back); a question with r.

import type { Action } from '../../keys/actions';
import { dispatch } from '../../keys/actions';
import { effortById, INBOX_GLYPH } from '../../state/select';
import { useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import type { InboxKind } from '../../core/model';
import { EffortTag } from '../chat/EffortTag';
import { Markdown } from '../chat/Markdown';
import { keyText } from '../overlays/Keys';

const ANSWERS: Record<InboxKind, [Action, string][]> = {
  permission: [['allow', 'allow once'], ['allowAlways', 'always allow'], ['deny', 'deny'], ['denyReason', 'deny with reason']],
  criterion: [['allow', 'approve, tick the criterion'], ['deny', 'send back'], ['denyReason', 'send back with reason']],
  question: [['reply', 'reply'], ['dismiss', 'dismiss']],
  finished: [['openAgent', 'open'], ['dismiss', 'dismiss']],
};

export function InboxItemView() {
  const s = useSnapshot();
  const ui = useUi();
  const it = s.inbox.find(x => x.id === ui.mainItem);
  if (!it) return <div className="empty-main">Answered.</div>;
  const agent = s.agents.find(a => a.id === it.agent_id)?.name ?? it.agent_id;
  return (
    <div className="itemview">
      <article>
        <div className="item-head">
          <span className={`glyph g-${it.kind}`}>{INBOX_GLYPH[it.kind]}</span>
          <EffortTag effort={effortById(s, it.effort_id)} />
          <span className="dim">from {agent} · {it.at} ago</span>
        </div>
        <Markdown text={it.body} />
        <div className="answers">
          {ANSWERS[it.kind].map(([a, label], i) => (
            <button key={a} className={i === 0 ? 'primary' : ''} onClick={() => dispatch(a)}>{keyText(a, 'item')} · {label}</button>
          ))}
          <button onClick={() => dispatch('escape')}>{keyText('escape')} · back</button>
        </div>
      </article>
    </div>
  );
}
