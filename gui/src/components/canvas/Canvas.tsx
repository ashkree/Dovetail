// The effort's canvas: a question tree on React Flow. Pan and zoom by mouse; everything else by
// the keymap (React Flow's own keys are off). Only the user moves cards: a drag ends in move_card.

import { useEffect, useMemo, useRef, useState } from 'react';
import {
  Handle, Position, ReactFlow, ReactFlowProvider, useReactFlow, useViewport,
  type Edge, type Node, type NodeChange, type NodeProps,
} from '@xyflow/react';
import type { Card } from '../../core/model';
import { dispatch, effortCanvas } from '../../keys/actions';
import { viewport } from '../../keys/viewport';
import { effortById } from '../../state/select';
import { send, useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { CardView } from './CardView';
import { IncomingStrip } from './IncomingStrip';

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 1.6;
const START = { x: 10, y: 10, zoom: 0.85 };
/** A card's size on the canvas (CSS .card width; a typical height), for fitting and keeping it in sight. */
const CARD_W = 250;
const CARD_H = 100;
const FIT_PAD = 16;
const FRAMEWORK_NAME = { question_tree: 'question tree', options_compare: 'options compare', free_board: 'free board' };

type CardNode = Node<{ card: Card; sel: boolean }, 'card'>;

function CardNodeView({ data }: NodeProps<CardNode>) {
  return <>
    <Handle type="target" position={Position.Top} isConnectable={false} />
    <CardView card={data.card} sel={data.sel} />
    <Handle type="source" position={Position.Bottom} isConnectable={false} />
  </>;
}
const NODE_TYPES = { card: CardNodeView };

export function Canvas() {
  const s = useSnapshot();
  const ui = useUi();
  const canvas = effortCanvas(s, ui.effort);
  if (!canvas) {
    const e = effortById(s, ui.effort);
    return (
      <div className="empty-main"><div className="empty-canvas">
        <div className="lead">{e?.name ?? 'This effort'} has no canvas yet. Start from a framework:</div>
        <div className="answers" style={{ justifyContent: 'center' }}>
          <button onClick={() => dispatch('pickFramework')}>1 · Question tree</button>
          <button onClick={() => dispatch('pickFramework')}>2 · Options compare</button>
          <button onClick={() => dispatch('pickFramework')}>3 · Free board</button>
        </div>
        <p className="dim">The filled canvas in this build is on <b>payment-providers</b>.</p>
      </div></div>
    );
  }
  const cards = s.cards.filter(c => c.canvas_id === canvas.id);
  return (
    <ReactFlowProvider>
      <div className="framework">framework: <b>{FRAMEWORK_NAME[canvas.framework]}</b> · {cards.length} cards · <Zoom /></div>
      <div className="canvas-wrap"><Flow cards={cards} /></div>
      <IncomingStrip canvasId={canvas.id} />
    </ReactFlowProvider>
  );
}

function Zoom() {
  const { zoom } = useViewport();
  return <>zoom {Math.round(zoom * 100)} %</>;
}

function Flow({ cards }: { cards: Card[] }) {
  const s = useSnapshot();
  const ui = useUi();
  const flow = useReactFlow();
  const wrap = useRef<HTMLDivElement>(null);
  /** Positions while a drag is under way; the snapshot keeps the last committed ones. */
  const [dragging, setDragging] = useState<Record<string, { x: number; y: number }>>({});
  const selOnCanvas = ui.canvasPart === 'cards' ? ui.cardSel : null;
  const cardsRef = useRef(cards);
  cardsRef.current = cards;

  const nodes: CardNode[] = useMemo(() => cards.map(c => ({
    id: c.id, type: 'card', position: dragging[c.id] ?? { x: c.x, y: c.y },
    data: { card: c, sel: c.id === selOnCanvas }, focusable: false, selectable: false,
  })), [cards, dragging, selOnCanvas]);
  const ids = new Set(cards.map(c => c.id));
  const edges: Edge[] = s.card_links.filter(l => ids.has(l.from) && ids.has(l.to))
    .map(l => ({ id: `${l.from}-${l.to}`, source: l.from, target: l.to, focusable: false, selectable: false }));

  useEffect(() => {
    viewport.canvas = {
      zoomIn: () => { void flow.zoomIn(); },
      zoomOut: () => { void flow.zoomOut(); },
      fit: () => {
        const box = wrap.current?.getBoundingClientRect();
        if (!box || !cardsRef.current.length) return;
        const xs = cardsRef.current.map(c => c.x), ys = cardsRef.current.map(c => c.y);
        const x0 = Math.min(...xs), y0 = Math.min(...ys);
        const w = Math.max(...xs) + CARD_W - x0, h = Math.max(...ys) + CARD_H - y0;
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

  const onNodesChange = (changes: NodeChange<CardNode>[]) => {
    for (const ch of changes) {
      if (ch.type === 'position' && ch.position && ch.dragging) {
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
          send({ type: 'move_card', card_id: node.id, x: Math.round(node.position.x), y: Math.round(node.position.y) });
          setDragging(d => { const { [node.id]: _gone, ...rest } = d; return rest; });
        }}
        onNodeClick={(e, node) => { dispatch('selectCard', node.id); if (e.detail >= 2) dispatch('openCard'); }}
        onPaneClick={() => dispatch('focusZone', 'main')}
        disableKeyboardA11y deleteKeyCode={null} selectionKeyCode={null} multiSelectionKeyCode={null}
        panActivationKeyCode={null} zoomActivationKeyCode={null}
        nodesConnectable={false} elementsSelectable={false} zoomOnDoubleClick={false}
        proOptions={{ hideAttribution: true }}
      />
    </div>
  );
}
