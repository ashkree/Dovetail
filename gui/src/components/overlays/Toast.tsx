// One short line at the bottom; hides itself by a CSS animation (no timer). `**x**` shows bold.

import { useUi } from '../../state/ui';

export function Toast() {
  const { toast } = useUi();
  if (!toast) return null;
  return <div className="toast" key={toast.n}>{bolds(toast.text)}</div>;
}

function bolds(text: string) {
  return text.split('**').map((part, i) => i % 2 ? <b key={i}>{part}</b> : part);
}
