// Canvas sizes in canvas pixels, the same numbers the core places by
// (`dovetail_api::canvas::geometry`; ts-rs exports no constants, so they are repeated here).

import type { Block, Card } from './model';

export const CARD_W = 250;
export const CARD_H = 100;
export const GAP = 40;
export const HEADER = 40;
export const PAD = 20;

export interface Rect { x: number; y: number; w: number; h: number }

/** The frame a block draws: around its cards, at least one cell big (as the core reckons it). */
export function blockExtent(b: Block, cards: Pick<Card, 'block_id' | 'x' | 'y'>[]): Rect {
  let right = b.x + 2 * PAD + CARD_W;
  let bottom = b.y + HEADER + CARD_H + PAD;
  for (const c of cards) {
    if (c.block_id !== b.id) continue;
    right = Math.max(right, c.x + CARD_W + PAD);
    bottom = Math.max(bottom, c.y + CARD_H + PAD);
  }
  return { x: b.x, y: b.y, w: right - b.x, h: bottom - b.y };
}
