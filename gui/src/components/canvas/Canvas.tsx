// The effort's canvas (ADR 0027): its blocks, each a framed region with a layout, and their cards,
// on React Flow. Pan and zoom by mouse; everything else by the keymap (React Flow's own keys are
// off). The core places new blocks and cards; only the user moves cards: a drag ends in move_card.

import { useEffect, useMemo, useRef, useState } from 'react';
import {
  Handle, Position, ReactFlow, ReactFlowProvider, useReactFlow, useViewport,
  type Edge, type Node, type NodeChange, type NodeProps,
} from '@xyflow/react';
import type { Block, Card, Layout } from '../../core/model';
import { blockExtent, CARD_H, CARD_W } from '../../core/geometry';
import { BLOCK_KEY, dispatch, effortBlocks } from '../../keys/actions';
import { keyText } from '../overlays/Keys';
import { viewport } from '../../keys/viewport';
import { effortById } from '../../state/select';
import { send, useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { CardView } from './CardView';
import { IncomingStrip } from './IncomingStrip';

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 1.6;
const START = { x: 10, y: 10, zoom: 0.85 };
const FIT_PAD = 16;
const LAYOUT_NAME: Record<Layout, string> = { question_tree: 'question tree', options_compare: 'options compare', free_area: 'free area' };

type CardNode = Node<{ card: Card; sel: boolean }, 'card'>;
type BlockNode = Node<{ block: Block }, 'block'>;

function CardNodeView({ data }: NodeProps<CardNode>) {
  return <>
    <Handle type="target" position={Position.Top} isConnectable={false} />
    <CardView card={data.card} sel={data.sel} />
    <Handle type="source" position={Position.Bottom} isConnectable={false} />
  </>;
}

function BlockNodeView({ data }: NodeProps<BlockNode>) {
  return <div className="block-frame"><div className="block-title">{data.block.title} <span className="dim">· {LAYOUT_NAME[data.block.layout]}</span></div></div>;
}
const NODE_TYPES = { card: CardNodeView, block: BlockNodeView };

export function Canvas() {
  const s = useSnapshot();
  const ui = useUi();
  const blocks = effortBlocks(s, ui.effort);
  const ids = new Set(blocks.map(b => b.id));
  const cards = s.cards.filter(c => ids.has(c.block_id));
  const e = effortById(s, ui.effort);
  return (
    <ReactFlowProvider>
      <div className="framework">{e?.name ?? 'Canvas'} · {blocks.length} blocks · {cards.length} cards · <Zoom /></div>
      <div className="canvas-wrap">
        {blocks.length ? <Flow blocks={blocks} cards={cards} /> : (
          <div className="empty-main"><div className="empty-canvas">
            <div className="lead">No blocks yet. Agents add them as they work, or start one ({keyText('newBlock1', 'canvas').charAt(0)}):</div>
            <div className="answers" style={{ justifyContent: 'center' }}>
              {(Object.entries(BLOCK_KEY) as [keyof typeof BLOCK_KEY, Layout][]).map(([action, l]) => (
                <button key={l} onClick={() => dispatch('pickLayout', l)}>{keyText(action, 'canvas').split('').join(' ')} · {LAYOUT_NAME[l]}</button>
              ))}
            </div>
          </div></div>
        )}
      </div>
      <IncomingStrip effortId={ui.effort} />
    </ReactFlowProvider>
  );
}

function Zoom() {
  const { zoom } = useViewport();
  return <>zoom {Math.round(zoom * 100)} %</>;
}

function Flow({ blocks, cards }: { blocks: Block[]; cards: Card[] }) {
  const s = useSnapshot();
  const ui = useUi();
  const flow = useReactFlow();
  const wrap = useRef<HTMLDivElement>(null);
  /** Positions while a drag is under way; the snapshot keeps the last committed ones. */
  const [dragging, setDragging] = useState<Record<string, { x: number; y: number }>>({});
  const selOnCanvas = ui.canvasPart === 'cards' ? ui.cardSel : null;
  const frames = blocks.map(b => ({ block: b, ...blockExtent(b, cards) }));
  const framesRef = useRef(frames);
  framesRef.current = frames;

  const nodes: (CardNode | BlockNode)[] = useMemo(() => [
    ...frames.map((f): BlockNode => ({
      id: `block-${f.block.id}`, type: 'block', position: { x: f.x, y: f.y }, data: { block: f.block },
      style: { width: f.w, height: f.h }, draggable: false, focusable: false, selectable: false, zIndex: -1,
    })),
    ...cards.map((c): CardNode => ({
      id: c.id, type: 'card', position: dragging[c.id] ?? { x: c.x, y: c.y },
      data: { card: c, sel: c.id === selOnCanvas }, focusable: false, selectable: false,
    })),
  ], [frames, cards, dragging, selOnCanvas]);
  const ids = new Set(cards.map(c => c.id));
  const edges: Edge[] = s.card_links.filter(l => ids.has(l.from) && ids.has(l.to))
    .map(l => ({ id: `${l.from}-${l.to}`, source: l.from, target: l.to, focusable: false, selectable: false }));

  useEffect(() => {
    viewport.canvas = {
      zoomIn: () => { void flow.zoomIn(); },
      zoomOut: () => { void flow.zoomOut(); },
      fit: () => {
        const box = wrap.current?.getBoundingClientRect();
        const fs = framesRef.current;
        if (!box || !fs.length) return;
        const x0 = Math.min(...fs.map(f => f.x)), y0 = Math.min(...fs.map(f => f.y));
        const w = Math.max(...fs.map(f => f.x + f.w)) - x0, h = Math.max(...fs.map(f => f.y + f.h)) - y0;
        const zoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, Math.min((box.width - 2 * FIT_PAD) / w, (box.height - 2 * FIT_PAD) / h)));
        void flow.setViewport({ x: (box.width - w * zoom) / 2 - x0 * zoom, y: FIT_PAD - y0 * zoom, zoom });
      },
    };
    return () => { viewport.canvas = null; };
  }, [flow]);

  // Keep the selected card in sight.
  useEffect(() => {
    const c = cards.find(x => x.id === ui.cardSel);
    const box = wrap.current?.getBoundingClientRect();
    if (!c || !box) return;
    const { x, y, zoom } = flow.getViewport();
    const left = c.x * zoom + x, top = c.y * zoom + y;
    if (left < 0 || top < 0 || left + CARD_W * zoom > box.width || top + CARD_H * zoom > box.height) {
      void flow.setCenter(c.x + CARD_W / 2, c.y + CARD_H / 2, { zoom });
    }
  }, [ui.cardSel, cards, flow]);

  const onNodesChange = (changes: NodeChange<CardNode | BlockNode>[]) => {
    for (const ch of changes) {
      if (ch.type === 'position' && ch.position && ch.dragging && !ch.id.startsWith('block-')) {
        const p = ch.position;
        setDragging(d => ({ ...d, [ch.id]: p }));
      }
    }
  };

  return (
    <div ref={wrap} style={{ position: 'absolute', inset: 0 }}>
      <ReactFlow
        nodes={nodes} edges={edges} nodeTypes={NODE_TYPES}
        defaultViewport={START} minZoom={MIN_ZOOM} maxZoom={MAX_ZOOM}
        onNodesChange={onNodesChange}
        onNodeDragStop={(_, node) => {
          if (node.type !== 'card') return;
          send({ type: 'move_card', card_id: node.id, x: Math.round(node.position.x), y: Math.round(node.position.y) });
          setDragging(d => { const { [node.id]: _gone, ...rest } = d; return rest; });
        }}
        onNodeClick={(e, node) => {
          if (node.type !== 'card') { dispatch('focusZone', 'main'); return; }
          dispatch('selectCard', node.id);
          if (e.detail >= 2) dispatch('openCard');
        }}
        onPaneClick={() => dispatch('focusZone', 'main')}
        disableKeyboardA11y deleteKeyCode={null} selectionKeyCode={null} multiSelectionKeyCode={null}
        panActivationKeyCode={null} zoomActivationKeyCode={null}
        nodesConnectable={false} elementsSelectable={false} zoomOnDoubleClick={false}
        proOptions={{ hideAttribution: true }}
      />
    </div>
  );
}
