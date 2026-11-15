// Key hints drawn from the keymap, so hints change when a key does.

import type { Action } from '../../keys/actions';
import { keyName, keysFor, leaderPathFor } from '../../keys/resolve';

/** The first key bound to `action` in `scope`, as a <kbd>. */
export function Key({ action, scope }: { action: Action; scope?: string }) {
  const k = keysFor(action, scope)[0];
  return k ? <kbd>{keyName(k)}</kbd> : null;
}

/** The Space-leader path to `action`, as one <kbd> (`Space e w`). */
export function LeaderKey({ action }: { action: Action }) {
  const p = leaderPathFor(action);
  return p ? <kbd>{['Space', ...p.map(keyName)].join(' ')}</kbd> : null;
}

/** Plain text of the first key for `action`, for titles and button labels. */
export function keyText(action: Action, scope?: string): string {
  const k = keysFor(action, scope)[0];
  if (k) return keyName(k);
  const p = leaderPathFor(action);
  return p ? ['Space', ...p.map(keyName)].join(' ') : '';
}
