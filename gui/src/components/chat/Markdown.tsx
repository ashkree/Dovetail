// Markdown as React elements (no raw HTML is injected). On top of GFM: `[[F3 title]]` is a card
// reference (shown `↗ F3 title`) and `[F3]` / `[S2, S3]` cite cards.

import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

interface MdNode { type: string; value?: string; children?: MdNode[]; data?: { hName: string; hProperties: { className: string } } }

const REFS = /\[\[([A-Z]\d+)([^\]]*)\]\]|\[([A-Z]\d+(?:, [A-Z]\d+)*)\]/g;

function span(className: string, value: string): MdNode {
  return { type: 'cardRef', data: { hName: 'span', hProperties: { className } }, children: [{ type: 'text', value }] };
}

/** Split text nodes around card references and citations. */
function remarkCardRefs() {
  const walk = (node: MdNode) => {
    if (!node.children) return;
    // Merge adjacent text first: brackets can arrive as separate text nodes.
    const merged: MdNode[] = [];
    for (const c of node.children) {
      const last = merged[merged.length - 1];
      if (c.type === 'text' && last?.type === 'text') last.value = (last.value ?? '') + (c.value ?? '');
      else merged.push(c);
    }
    node.children = merged.flatMap(c => {
      if (c.type !== 'text' || !c.value) { walk(c); return [c]; }
      const out: MdNode[] = [];
      let at = 0;
      for (const m of c.value.matchAll(REFS)) {
        if (m.index > at) out.push({ type: 'text', value: c.value.slice(at, m.index) });
        out.push(m[1] ? span('ref', `↗ ${m[1]}${m[2] ?? ''}`) : span('cite', `[${m[3]}]`));
        at = m.index + m[0].length;
      }
      if (at < c.value.length) out.push({ type: 'text', value: c.value.slice(at) });
      return out;
    });
  };
  return (tree: MdNode) => { walk(tree); };
}

export function Markdown({ text }: { text: string }) {
  return <ReactMarkdown remarkPlugins={[remarkGfm, remarkCardRefs]}>{text}</ReactMarkdown>;
}
