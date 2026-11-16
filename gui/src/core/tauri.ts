// TauriCore: the CoreLink to the Rust core hosted by the app's own process (gui/src-tauri,
// ADR 0023). Two calls and nothing else: `core_subscribe` with a channel that receives every
// ServerMsg in order (the shell asks the core for a fresh snapshot first), and `core_send`.

import { Channel, invoke } from '@tauri-apps/api/core';
import type { Command, ServerMsg } from './generated';
import type { CoreLink } from './link';

/** Whether the page runs inside the Tauri window rather than a plain browser. */
export function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export class TauriCore implements CoreLink {
  constructor(private readonly failed: (message: string) => void) {}

  subscribe(fn: (msg: ServerMsg) => void): () => void {
    let live = true;
    const channel = new Channel<ServerMsg>();
    channel.onmessage = msg => { if (live) fn(msg); };
    invoke('core_subscribe', { channel }).catch(e => this.failed(String(e)));
    return () => { live = false; };
  }

  send(command: Command): void {
    invoke('core_send', { command }).catch(e => this.failed(String(e)));
  }
}
