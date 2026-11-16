// Dev server for the GUI mock: serves HTML, CSS and TypeScript, no dependencies.
// .ts files are served as JavaScript with their types stripped (node:module, Node >= 23).
// Pages reload themselves when a file in this folder changes.
//
//   node mock/gui/serve.mjs            # http://127.0.0.1:8765
//   PORT=9000 node mock/gui/serve.mjs

import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { stripTypeScriptTypes } from 'node:module';
import { fileURLToPath } from 'node:url';

process.removeAllListeners('warning'); // silence the experimental-feature warning
const ROOT = path.dirname(fileURLToPath(import.meta.url));
const PORT = Number(process.env.PORT || 8765);
const TYPES = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.mjs': 'text/javascript', '.svg': 'image/svg+xml', '.png': 'image/png', '.json': 'application/json' };

const RELOAD = `<script>new EventSource('/__reload').onmessage = () => location.reload();</script>`;
const clients = new Set();
let timer = null;
fs.watch(ROOT, { recursive: true }, () => {
  clearTimeout(timer);
  timer = setTimeout(() => { for (const res of clients) res.write('data: reload\n\n'); }, 80);
});

http.createServer((req, res) => {
  const url = new URL(req.url, 'http://x');
  if (url.pathname === '/__reload') {
    res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-store', Connection: 'keep-alive' });
    res.write('\n');
    clients.add(res);
    req.on('close', () => clients.delete(res));
    return;
  }
  const rel = decodeURIComponent(url.pathname === '/' ? '/index.html' : url.pathname);
  const file = path.join(ROOT, rel);
  if (!file.startsWith(ROOT + path.sep)) { res.writeHead(403).end(); return; }
  fs.readFile(file, 'utf8', (err, text) => {
    if (err) { res.writeHead(404, { 'Content-Type': 'text/plain' }).end('not found: ' + rel); return; }
    const ext = path.extname(file);
    let body = text, type = TYPES[ext] || 'text/plain';
    try {
      if (ext === '.ts') { body = stripTypeScriptTypes(text, { sourceUrl: rel }); type = 'text/javascript'; }
    } catch (e) {
      body = `console.error(${JSON.stringify(`TypeScript error in ${rel}: ${e.message}`)});`; type = 'text/javascript';
      console.error(`${rel}: ${e.message}`);
    }
    if (ext === '.html') body = body.replace('</body>', RELOAD + '</body>');
    res.writeHead(200, { 'Content-Type': type + '; charset=utf-8', 'Cache-Control': 'no-store' }).end(body);
  });
}).listen(PORT, '127.0.0.1', () => console.log(`GUI mock on http://127.0.0.1:${PORT}`));
