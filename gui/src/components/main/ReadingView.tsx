// The reading view: a long report as markdown. ]] / [[ step by heading, r puts a reference in the chat bar.

import { useEffect, useRef } from 'react';
import { viewport } from '../../keys/viewport';
import { useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { readingReport } from '../../state/select';
import { Markdown } from '../chat/Markdown';

export function ReadingView() {
  const s = useSnapshot();
  const ui = useUi();
  const el = useRef<HTMLDivElement>(null);
  const report = readingReport(s, ui.cardSel, ui.effort);

  useEffect(() => {
    viewport.reading = el.current;
    return () => { viewport.reading = null; };
  }, []);

  useEffect(() => {
    const hs = el.current ? Array.from(el.current.querySelectorAll<HTMLElement>('h1, h2, h3')) : [];
    hs.forEach((h, i) => h.classList.toggle('cur', i === ui.readHead));
    const h = hs[ui.readHead];
    if (h && ui.readHead > 0) h.scrollIntoView({ block: 'start' });
    else if (ui.readHead === 0) el.current?.scrollTo({ top: 0 });
  }, [ui.readHead, report?.id]);

  if (!report) return <div className="empty-main">No report to read in this project yet.</div>;
  return (
    <div className="reading" ref={el}>
      <article><Markdown text={report.markdown} /></article>
    </div>
  );
}
