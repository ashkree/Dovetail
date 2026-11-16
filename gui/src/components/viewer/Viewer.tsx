// The viewer: the focused slice's running web preview with the native variant panel beside it
// (ADR 0026). The core runs the preview; this opens it on the slice (the first time starts it) and
// shows its state. The user's variant picks are posted to the page as { dovetailVariants: { slot: option } }
// on load and on every change; nothing else is injected.

import { useEffect, useRef } from 'react';
import { dispatch, viewerAgent, viewerPreview, viewerSet } from '../../keys/actions';
import { send, useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { keyText } from '../overlays/Keys';
import { VariantPanel } from './VariantPanel';

export function Viewer() {
  const s = useSnapshot();
  const ui = useUi();
  const agent = viewerAgent(s, ui.effort);
  const preview = viewerPreview(s, ui.effort);
  const set = viewerSet(s, ui.effort);
  const frame = useRef<HTMLIFrameElement>(null);

  // The first time the viewer opens on a slice, the core starts its preview.
  const needsStart = agent !== undefined && preview?.state === 'not_started';
  useEffect(() => {
    if (needsStart && agent) send({ type: 'start_preview', agent_id: agent.id });
  }, [needsStart, agent?.id]);

  const url = preview?.state === 'running' ? preview.url : undefined;
  const selection: Record<string, string> = {};
  set?.slots.forEach((slot, i) => { selection[slot.name] = slot.options[set.choice[i] ?? 0] ?? ''; });
  const message = JSON.stringify(selection);
  const post = () => {
    if (!url) return;
    frame.current?.contentWindow?.postMessage({ dovetailVariants: JSON.parse(message) }, new URL(url, location.origin).origin);
  };
  useEffect(post, [message, url]);

  if (!agent) return <div className="empty-main">No slice in this effort has a web preview yet.</div>;
  const name = agent.name;
  return (
    <div className="viewer">
      <div className="preview">
        <div className="urlbar">
          <span>◆ {name}{agent.branch ? ` · ${agent.branch}` : ''}</span>
          <span className="mono">{url ?? ''}</span><span className="spacer" />
          <button onClick={() => dispatch('reload')}>⟳ {keyText('reload', 'viewer')}</button>
          <button onClick={() => dispatch('browser')}>↗ {keyText('browser', 'viewer')}</button>
        </div>
        {url ? (
          <iframe ref={frame} key={ui.previewLoads} src={url} title={`Preview of ${name}`} onLoad={post} />
        ) : (
          <PreviewMessage state={preview?.state ?? 'not_started'} output={preview?.output} project={s.open_project} agent={agent.id} />
        )}
      </div>
      <VariantPanel set={set} />
    </div>
  );
}

function PreviewMessage(props: { state: string; output: string | undefined; project: string | null; agent: string }) {
  if (props.state === 'no_command') {
    return (
      <div className="empty-main">
        <div>
          <p>
            This repo has no preview line, so there is nothing to show. Add one to the project's recipe
            (<code>~/.config/dovetail/projects/{props.project ?? '<project>'}.toml</code>); a repo of its own goes
            under <code>[repos.&lt;name&gt;]</code>:
          </p>
          <pre>preview = &quot;npm run dev -- --port {'{port}'}&quot;</pre>
          <p>Dovetail starts it in the slice's worktree on a free port.</p>
        </div>
      </div>
    );
  }
  if (props.state === 'failed') {
    return (
      <div className="empty-main">
        <div>
          <p>The preview did not run.</p>
          <pre>{props.output}</pre>
          <button className="chip" onClick={() => send({ type: 'restart_preview', agent_id: props.agent })}>Restart the preview</button>
        </div>
      </div>
    );
  }
  return <div className="empty-main">Starting the preview…</div>;
}
