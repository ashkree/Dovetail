import { memo, type Ref } from 'react';
import type { ChatEntry } from '../../core/model';
import { dispatch } from '../../keys/actions';
import { TagDot } from './EffortTag';
import { Markdown } from './Markdown';
import { keyText } from '../overlays/Keys';

/** What a message shows besides the entry: the speaker's name and the effort tag, as plain values
 * so the memoised message below re-renders only when something it shows changed (docs/plan/gui-perf.md). */
export interface Shown { name: string; tag?: { name: string; colour: string } }

const sameEntry = (a: ChatEntry, b: ChatEntry) =>
  a === b || (a.id === b.id && a.text === b.text && a.at === b.at && a.who === b.who && a.report === b.report);
const sameShown = (a: Shown, b: Shown) =>
  a.name === b.name && a.tag?.name === b.tag?.name && a.tag?.colour === b.tag?.colour;

interface MessageProps { entry: ChatEntry; shown: Shown; cur: boolean; showTag: boolean; ref?: Ref<HTMLDivElement> }

/** A message: speaker, effort tag, a rule and the time, always visible; you and the coordinator in their own colours. */
export const Message = memo(function Message({ entry, shown, cur, showTag, ref }: MessageProps) {
  const you = entry.who === 'you';
  return (
    <div ref={ref} className={`msg ${you ? 'by-you' : 'by-agent'} ${cur ? 'cur' : ''}`}>
      <div className="msg-h">
        <span className={`who ${you ? 'you' : ''}`}>{shown.name}</span>
        {showTag && shown.tag && <TagDot name={shown.tag.name} colour={shown.tag.colour} />}
        <span className="rule" /><span className="time">{entry.at}</span>
      </div>
      <div className="msg-body"><Markdown text={entry.text} /></div>
    </div>
  );
}, (a, b) => sameEntry(a.entry, b.entry) && sameShown(a.shown, b.shown) && a.cur === b.cur && a.showTag === b.showTag && a.ref === b.ref);

interface ReportProps { entry: ChatEntry; from: string; cur: boolean; open: boolean; ref?: Ref<HTMLDivElement> }

/** A slice report: one collapsed line; opens in place. */
export const ReportLine = memo(function ReportLine({ entry, from, cur, open, ref }: ReportProps) {
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
}, (a, b) => sameEntry(a.entry, b.entry) && a.from === b.from && a.cur === b.cur && a.open === b.open && a.ref === b.ref);
