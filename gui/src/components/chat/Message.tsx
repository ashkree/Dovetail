import type { Ref } from 'react';
import type { ChatEntry } from '../../core/model';
import { dispatch } from '../../keys/actions';
import { effortById, speakerName } from '../../state/select';
import { useSnapshot } from '../../state/store';
import { EffortDot } from './EffortTag';
import { Markdown } from './Markdown';
import { keyText } from '../overlays/Keys';

/** A message: speaker, effort tag, a rule and the time, always visible; you and the coordinator in their own colours. */
export function Message({ entry, cur, showTag, ref }: { entry: ChatEntry; cur: boolean; showTag: boolean; ref?: Ref<HTMLDivElement> }) {
  const s = useSnapshot();
  const you = entry.who === 'you';
  return (
    <div ref={ref} className={`msg ${you ? 'by-you' : 'by-agent'} ${cur ? 'cur' : ''}`}>
      <div className="msg-h">
        <span className={`who ${you ? 'you' : ''}`}>{speakerName(s, entry.who)}</span>
        {showTag && <EffortDot effort={effortById(s, entry.effort_id)} />}
        <span className="rule" /><span className="time">{entry.at}</span>
      </div>
      <div className="msg-body"><Markdown text={entry.text} /></div>
    </div>
  );
}

/** A slice report: one collapsed line; opens in place. */
export function ReportLine({ entry, cur, open, ref }: { entry: ChatEntry; cur: boolean; open: boolean; ref?: Ref<HTMLDivElement> }) {
  const s = useSnapshot();
  const from = speakerName(s, entry.who);
  return (
    <div ref={ref} className={`msg ${cur ? 'cur' : ''}`}>
      <div className="report" onClick={() => dispatch('toggleReport', entry.id)}>
        <span>{open ? '▾' : '▸'}</span><span className="from">{from}</span><span className="t">{entry.text}</span>
        <span className="spacer" /><span>{entry.at}</span>
      </div>
      {open && (
        <div className="report-open">
          <Markdown text={`**${from} reports:** ${entry.text}.\n\nFull report: \`${keyText('openCard', 'canvas')}\` on its card, or \`${keyText('openReading')}\`.`} />
        </div>
      )}
    </div>
  );
}
