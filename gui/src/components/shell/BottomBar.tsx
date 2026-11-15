export function BottomBar() {
  return (
    <div className="status">
      <span className="mode">NORMAL</span>
      <span className="pending" />
      <span className="st">Chat</span>
      <span className="st warn">⚠ 4 need you</span>
      <span className="grow" />
      <span className="st warn">efforts 5 / 4 ▲</span>
      <span className="st">work</span>
      <span className="st"><span className="ctx-bar"><i style={{ width: '62%' }} /></span>ctx 62%</span>
      <span className="st">5h 34% · week 18%</span>
      <span className="st"><b>Opus 5.5</b><span className="lvl">Medium</span></span>
      <span className="st">Auto</span>
      <span className="st"><span className="spin" /></span>
    </div>
  );
}
