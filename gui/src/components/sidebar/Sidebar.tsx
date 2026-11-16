// The sidebar: the inbox (grows with its items, up to 40 % of the height) over the efforts tree.
// Folds to a rail; its left edge drags to widen it.

import type { MouseEvent } from 'react';
import { setUi, useUi } from '../../state/ui';
import { Inbox } from './Inbox';
import { EffortsTree } from './EffortsTree';
import { Rail } from './Rail';

const MIN_W = 240;
const MAX_W = 700;

function startDrag(e: MouseEvent<HTMLDivElement>): void {
  e.preventDefault();
  const handle = e.currentTarget;
  const right = handle.closest('.ws')?.getBoundingClientRect().right ?? window.innerWidth;
  handle.classList.add('drag');
  const move = (ev: globalThis.MouseEvent) => setUi({ sideW: Math.max(MIN_W, Math.min(MAX_W, Math.round(right - ev.clientX - 8))) });
  const up = () => {
    handle.classList.remove('drag');
    window.removeEventListener('mousemove', move);
    window.removeEventListener('mouseup', up);
  };
  window.addEventListener('mousemove', move);
  window.addEventListener('mouseup', up);
}

export function Sidebar() {
  const { sideCollapsed } = useUi();
  if (sideCollapsed) return <Rail />;
  return (
    <div className="sidebar">
      <div className="side-handle" title="Drag to widen" onMouseDown={startDrag} />
      <Inbox />
      <EffortsTree />
    </div>
  );
}
