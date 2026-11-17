// An effort's tag: as a coloured dot and its name (message headers, chat bar), or as a pill with
// its kind glyph (inbox item, pickers).

import type { Effort } from '../../core/model';
import { KIND_GLYPH } from '../../state/select';

export const tagColour = (e: Effort | undefined) => e ? `var(--tag-${e.colour})` : 'var(--dim)';

/** The dot and name from plain values, so a memoised component can take them as props. */
export function TagDot({ name, colour }: { name: string; colour: string }) {
  return <span className="tagdot" title={name}><span className="dot" style={{ background: colour }} /><span className="tagname">{name}</span></span>;
}

export function EffortDot({ effort }: { effort: Effort | undefined }) {
  if (!effort) return null;
  return <TagDot name={effort.name} colour={tagColour(effort)} />;
}

export function EffortTag({ effort }: { effort: Effort | undefined }) {
  if (!effort) return null;
  return <span className="tag" style={{ color: tagColour(effort) }}>{KIND_GLYPH[effort.kind]} {effort.name}</span>;
}
