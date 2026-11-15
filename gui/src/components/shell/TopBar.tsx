export function TopBar() {
  return (
    <div className="header">
      <span className="brand">◆ Dovetail</span>
      <span className="crumb"><button className="dim">Projects</button><span className="sep">/</span>Shop</span>
      <span className="chip repo">shop-api</span><span className="chip repo">shop-web</span>
      <nav className="viewtabs">
        {['Chat', 'Canvas', 'Viewer', 'Reading'].map((n, i) => <button key={n} className={`vtab ${i === 0 ? 'on' : ''}`}>{n}</button>)}
      </nav>
      <span className="spacer" />
    </div>
  );
}
