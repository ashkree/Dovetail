// Handles the dispatcher needs on things only the DOM or React Flow know: scroll positions,
// heading positions, the canvas zoom. Components register them on mount and clear them on unmount.

export interface CanvasHandle {
  zoomIn(): void;
  zoomOut(): void;
  fit(): void;
}

export const viewport: {
  reading: HTMLElement | null;
  canvas: CanvasHandle | null;
} = { reading: null, canvas: null };

/** The headings of the reading view, in order. */
export function readingHeadings(): HTMLElement[] {
  return viewport.reading ? Array.from(viewport.reading.querySelectorAll<HTMLElement>('h1, h2, h3')) : [];
}
