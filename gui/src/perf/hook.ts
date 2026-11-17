// Perf instrumentation, part 1: a stand-in for the React DevTools hook that counts which
// components render on each commit. Only active in a build made with VITE_PERF=1 (see
// docs/plan/gui-perf.md); in a normal build this file does nothing. Imported first in main.tsx,
// before React loads, because React looks the hook up when it starts.

interface Fiber {
  tag: number; type: unknown; flags: number; child: Fiber | null; sibling: Fiber | null; alternate: Fiber | null;
}

export const renderStats = { counting: false, commits: 0, renders: new Map<string, number>() };

const FUNCTION_TAGS = new Set([0, 11, 14, 15]); // function, forwardRef, memo, simple memo
const PERFORMED_WORK = 1;

function nameOf(f: Fiber): string {
  const t = f.type as { displayName?: string; name?: string; type?: { name?: string }; render?: { name?: string } } | null;
  return t?.displayName ?? t?.name ?? t?.type?.name ?? t?.render?.name ?? 'anonymous';
}

function walk(f: Fiber | null): void {
  while (f) {
    if (FUNCTION_TAGS.has(f.tag) && f.alternate && (f.flags & PERFORMED_WORK)) {
      const n = nameOf(f);
      renderStats.renders.set(n, (renderStats.renders.get(n) ?? 0) + 1);
    }
    walk(f.child);
    f = f.sibling;
  }
}

if (import.meta.env.VITE_PERF) {
  (window as unknown as Record<string, unknown>).__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
    supportsFiber: true,
    renderers: new Map(),
    inject: () => 1,
    checkDCE: () => {},
    onCommitFiberUnmount: () => {},
    onPostCommitFiberRoot: () => {},
    onCommitFiberRoot: (_id: number, root: { current: Fiber }) => {
      renderStats.commits++;
      if (renderStats.counting) walk(root.current);
    },
  };
}

if (import.meta.env.VITE_PERF) {
  // Errors in a perf window have nowhere else to go.
  const report = (what: string) => {
    const t = (window as unknown as { __TAURI_INTERNALS__?: { invoke(c: string, a: object): Promise<unknown> } }).__TAURI_INTERNALS__;
    void t?.invoke('perf_log', { line: `ERROR ${what}` });
  };
  window.addEventListener('error', e => report(`${e.message} at ${e.filename}:${e.lineno}`));
  window.addEventListener('unhandledrejection', e => report(`rejection ${String((e.reason as Error)?.stack ?? e.reason)}`));
  report('hook installed');
}
