// The one seam between the frontend and a core: the core's own messages (generated from
// crates/dovetail-api). TauriCore carries them to the Rust core in the app's process; MockCore
// fakes one in a browser. Components never see which one is behind it.

import type { Command, ServerMsg } from './generated';

export interface CoreLink {
  /** Receive every message from the core, starting with a snapshot. Returns an unsubscribe. */
  subscribe(fn: (msg: ServerMsg) => void): () => void;
  send(cmd: Command): void;
}
