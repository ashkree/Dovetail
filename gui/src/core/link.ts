// The one seam between the frontend and a core. MockCore implements it now; TauriCore will later
// (plan §8). Components never see which one is behind it.

import type { Command, ServerMsg } from './protocol';

export interface CoreLink {
  /** Receive every message from the core, starting with a snapshot. Returns an unsubscribe. */
  subscribe(fn: (msg: ServerMsg) => void): () => void;
  send(cmd: Command): void;
}
