//! The `tui-readability` spike (ADR 0030 §5) with `kitty-text-sizing` folded in. See
//! `docs/plan/tui-readability-spike.md` and `docs/spikes/tui-readability.md`.
//!
//! ```text
//! tui-readability-spike --probe                 detection + glyph width check, then exit
//! tui-readability-spike --variant a|b|c --fixture PATH [--no-osc66|--force-osc66] [--no-highlight]
//! tui-readability-spike --bench ...             headless frame time / bytes / RSS, then exit
//! ```

mod highlight;
mod layout;
mod markdown;
mod theme;
mod wrap;

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Attribute, Print, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{cursor, execute, queue};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::{TerminalOptions, Viewport};
use unicode_width::UnicodeWidthChar;

use layout::{A, B, C, Doc, Opts, Variant, layout as do_layout, slice_line};
use theme::Theme;

static BYTES: AtomicUsize = AtomicUsize::new(0);

/// Counts every byte ratatui and the heading pass write.
struct Counting<W>(W);

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.0.write(buf)?;
        BYTES.fetch_add(n, Ordering::Relaxed);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

fn rss_kb() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------- fixtures

struct Fixture {
    name: String,
    md: String,
}

/// A trimmed fixture (`{"role","text"}` per line) as one markdown document.
fn load_fixture(path: &str) -> io::Result<Fixture> {
    let raw = std::fs::read_to_string(path)?;
    let mut md = String::new();
    let mut tools: Vec<String> = Vec::new();
    let flush = |md: &mut String, tools: &mut Vec<String>| {
        if !tools.is_empty() {
            md.push_str(&format!("*{}*\n\n", tools.join("  \n")));
            tools.clear();
        }
    };
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let text = v["text"].as_str().unwrap_or("");
        match v["role"].as_str() {
            Some("tool") => tools.push(format!("⚙ {}", text.replace('*', "\\*"))),
            Some("user") => {
                flush(&mut md, &mut tools);
                md.push_str(&format!("**you ›** {text}\n\n"));
            }
            _ => {
                flush(&mut md, &mut tools);
                md.push_str(text);
                md.push_str("\n\n");
            }
        }
    }
    flush(&mut md, &mut tools);
    let name = std::path::Path::new(path)
        .file_stem()
        .map_or_else(|| path.to_string(), |s| s.to_string_lossy().into_owned());
    Ok(Fixture { name, md })
}

// ---------------------------------------------------------------- OSC 66

#[derive(Clone, Copy, PartialEq)]
enum Osc {
    On,
    Off(&'static str),
}

struct Probe {
    width: bool,
    scale: bool,
}

/// kitty's documented detection: CR, CPR, `w=2` space, CPR, `s=2` space, CPR (raw mode).
fn probe() -> io::Result<Probe> {
    let mut out = io::stdout();
    let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
    if !was_raw {
        enable_raw_mode()?;
    }
    let result = (|| {
        write!(out, "\r")?;
        out.flush()?;
        let p0 = cursor::position()?;
        write!(out, "\x1b]66;w=2; \x07")?;
        out.flush()?;
        let p1 = cursor::position()?;
        write!(out, "\x1b]66;s=2; \x07")?;
        out.flush()?;
        let p2 = cursor::position()?;
        Ok(Probe {
            width: p1.0 == p0.0 + 2,
            scale: p2.0 == p1.0 + 2,
        })
    })();
    // Clear the probe line and the rows the 2x2 block touched.
    let _ = write!(out, "\r\x1b[2K\x1b[J");
    let _ = out.flush();
    if !was_raw {
        let _ = disable_raw_mode();
    }
    result
}

fn detect(no: bool, force: bool) -> Osc {
    if no {
        return Osc::Off("--no-osc66");
    }
    if force {
        return Osc::On;
    }
    if std::env::var_os("TMUX").is_some() {
        return Osc::Off("tmux");
    }
    if std::env::var("TERM").as_deref() != Ok("xterm-kitty") {
        return Osc::Off("TERM is not xterm-kitty");
    }
    match probe() {
        Ok(p) if p.width && p.scale => Osc::On,
        Ok(_) => Osc::Off("probe: no support"),
        Err(_) => Osc::Off("probe: no CPR reply"),
    }
}

/// One OSC 66 sequence, wrapped for tmux's passthrough when running under tmux.
fn osc66(meta: &str, text: &str) -> String {
    let osc = format!("\x1b]66;{meta};{text}\x1b\\");
    if std::env::var_os("TMUX").is_some() {
        format!("\x1bPtmux;{}\x1b\\", osc.replace('\x1b', "\x1b\x1b"))
    } else {
        osc
    }
}

/// `--probe`: detection, environment, and a width check of every non-ASCII glyph in the fixtures.
fn run_probe(fixtures: &[Fixture], force: bool) -> io::Result<()> {
    println!("TERM={}", std::env::var("TERM").unwrap_or_default());
    println!("TERM_PROGRAM={}", std::env::var("TERM_PROGRAM").unwrap_or_default());
    println!("KITTY_WINDOW_ID={}", std::env::var("KITTY_WINDOW_ID").unwrap_or_default());
    println!("TMUX={}", std::env::var("TMUX").unwrap_or_default());
    if std::env::var_os("TMUX").is_some()
        && let Ok(o) = std::process::Command::new("tmux")
            .args(["show", "-gv", "allow-passthrough"])
            .output()
    {
        println!("tmux allow-passthrough={}", String::from_utf8_lossy(&o.stdout).trim());
    }
    match detect(false, false) {
        Osc::On => println!("osc66 detect: ON (width and scale parts both answered)"),
        Osc::Off(why) => println!("osc66 detect: OFF ({why})"),
    }
    if force || std::env::var_os("TMUX").is_none() {
        match probe() {
            Ok(p) => println!("osc66 raw probe: w=2 moved cursor: {}, s=2 moved cursor: {}", p.width, p.scale),
            Err(e) => println!("osc66 raw probe: error {e}"),
        }
    } else {
        println!("osc66 raw probe: skipped under tmux (tmux answers CPR itself; pass --force-osc66 to run it)");
    }
    // Width check: print each glyph after CR, read the CPR, compare with unicode-width.
    let mut seen: Vec<char> = Vec::new();
    for f in fixtures {
        for c in f.md.chars() {
            if !c.is_ascii() && !seen.contains(&c) {
                seen.push(c);
            }
        }
    }
    seen.truncate(400);
    enable_raw_mode()?;
    let mut out = io::stdout();
    let mut mismatches = Vec::new();
    let mut checked = 0;
    for c in &seen {
        write!(out, "\r\x1b[2K{c}")?;
        out.flush()?;
        match cursor::position() {
            Ok((col, _)) => {
                checked += 1;
                let theirs = col as usize;
                let ours = c.width().unwrap_or(0);
                if theirs != ours {
                    mismatches.push((*c, ours, theirs));
                }
            }
            Err(_) => break,
        }
    }
    write!(out, "\r\x1b[2K")?;
    out.flush()?;
    disable_raw_mode()?;
    println!("width check: {checked} distinct non-ASCII glyphs from the fixtures, {} mismatches", mismatches.len());
    for (c, ours, theirs) in mismatches {
        println!("  U+{:04X} {c}: unicode-width {ours}, terminal {theirs}", c as u32);
    }
    Ok(())
}

// ---------------------------------------------------------------- the app

struct App {
    th: Theme,
    fixtures: Vec<Fixture>,
    fixture: usize,
    variant: usize,
    solid: Option<bool>,
    hl: Option<bool>,
    no_highlight: bool,
    osc: Osc,
    key: Option<(usize, usize, usize, bool, bool)>,
    doc: Doc,
    top: usize,
    focus: Option<usize>,
    offsets: HashMap<usize, usize>,
    last_ms: f64,
    last_bytes: usize,
    body_h: usize,
}

const VARIANTS: [Variant; 3] = [A, B, C];

impl App {
    fn variant(&self) -> &Variant {
        &VARIANTS[self.variant]
    }
    fn hl_on(&self) -> bool {
        highlight::ENABLED && self.hl.unwrap_or(self.variant().highlight && !self.no_highlight)
    }
    fn solid_on(&self) -> bool {
        self.solid.unwrap_or(self.variant().solid)
    }
    fn osc_on(&self) -> bool {
        self.osc == Osc::On
    }

    /// Rebuilds blocks and layout when the variant, fixture, pane width, OSC or highlight changed.
    fn ensure_layout(&mut self, pane_w: usize) {
        let key = (self.variant, self.fixture, pane_w, self.osc_on(), self.hl_on());
        if self.key == Some(key) {
            return;
        }
        let v = *self.variant();
        let avail = pane_w.saturating_sub(2 * v.margin_x).max(20);
        let blocks = markdown::render(&self.fixtures[self.fixture].md, v.prose_w.min(avail), avail, &self.th);
        let o = Opts {
            pane_w,
            osc66: self.osc_on(),
            highlight: self.hl_on(),
        };
        self.doc = do_layout(&blocks, &v, &o, &self.th);
        self.key = Some(key);
        self.offsets.clear();
        self.focus = None;
        self.top = self.top.min(self.doc.rows.len().saturating_sub(1));
    }

    fn max_top(&self) -> usize {
        self.doc.rows.len().saturating_sub(self.body_h)
    }

    fn scroll(&mut self, delta: isize) {
        self.top = self.top.saturating_add_signed(delta).min(self.max_top());
    }

    fn visible_wides(&self) -> Vec<usize> {
        let (a, b) = (self.top, self.top + self.body_h);
        (0..self.doc.wides.len())
            .filter(|i| {
                let w = &self.doc.wides[*i];
                w.first < b && w.first + w.rows > a
            })
            .collect()
    }

    fn focus_step(&mut self, back: bool) {
        let vis = self.visible_wides();
        if vis.is_empty() {
            return;
        }
        let pos = self.focus.and_then(|f| vis.iter().position(|v| *v == f));
        let next = match (pos, back) {
            (None, false) => 0,
            (None, true) => vis.len() - 1,
            (Some(p), false) => (p + 1) % vis.len(),
            (Some(p), true) => (p + vis.len() - 1) % vis.len(),
        };
        self.focus = Some(vis[next]);
    }

    fn pan(&mut self, to: Option<isize>) {
        let Some(f) = self.focus else { return };
        let w = &self.doc.wides[f];
        let max = w.total_w.saturating_sub(w.vis_w);
        let cur = self.offsets.get(&f).copied().unwrap_or(0);
        let new = match to {
            Some(d) if d.abs() == 8 => cur.saturating_add_signed(d).min(max),
            Some(0) => 0,
            _ => max,
        };
        self.offsets.insert(f, new);
    }

    fn headings_in_view(&self) -> Vec<&layout::Heading> {
        self.doc
            .headings
            .iter()
            .filter(|h| h.row >= self.top && h.row + h.rows <= self.top + self.body_h)
            .collect()
    }

    /// One frame: ratatui's draw, then OSC 66 headings, then flush. Returns (time, bytes).
    fn frame<W: Write>(&mut self, term: &mut Terminal<CrosstermBackend<W>>) -> io::Result<()> {
        let t0 = Instant::now();
        let b0 = BYTES.load(Ordering::Relaxed);
        let size = term.size()?;
        self.body_h = (size.height as usize).saturating_sub(1);
        self.ensure_layout(size.width as usize);
        self.top = self.top.min(self.max_top());
        let status = self.status_line();
        let visible_scaled: Vec<usize> = self
            .doc
            .headings
            .iter()
            .enumerate()
            .filter(|(_, h)| h.row >= self.top && h.row + h.rows <= self.top + self.body_h)
            .map(|(i, _)| i)
            .collect();
        let this: &App = self;
        term.draw(|f| {
            let area = f.area();
            this.paint(f.buffer_mut(), area, &visible_scaled, &status);
        })?;
        if self.osc_on() {
            let w = term.backend_mut().writer_mut();
            for h in self.headings_in_view() {
                queue!(
                    w,
                    cursor::MoveTo(h.x as u16, (h.row - self.top) as u16),
                    SetForegroundColor(to_ct(self.th.bold)),
                    SetAttribute(Attribute::Bold)
                )?;
                if self.solid_on() {
                    queue!(w, SetBackgroundColor(to_ct(self.th.ink)))?;
                }
                if h.level == 1 {
                    queue!(w, Print(osc66("s=2", &h.text)))?;
                } else {
                    // 1.5x (s=2 with n/d = 3/4) in 2x2 boxes, two characters per 4 cells.
                    let chars: Vec<char> = h.text.chars().collect();
                    for pair in chars.chunks(2) {
                        let s: String = pair.iter().collect();
                        queue!(w, Print(osc66("s=2:n=3:d=4:w=2", &s)))?;
                    }
                }
                queue!(w, SetAttribute(Attribute::Reset))?;
            }
            w.flush()?;
        }
        self.last_ms = t0.elapsed().as_secs_f64() * 1000.0;
        self.last_bytes = BYTES.load(Ordering::Relaxed) - b0;
        Ok(())
    }

    fn status_line(&self) -> String {
        let osc = match self.osc {
            Osc::On => "on".to_string(),
            Osc::Off("--no-osc66") => "forced-off".to_string(),
            Osc::Off(why) => format!("off ({why})"),
        };
        format!(
            " {} · {} · osc66:{osc} · hl:{} · solid:{} · {:.2} ms · {} B · rss {} MB · row {}/{} ",
            self.variant().name,
            self.fixtures[self.fixture].name,
            if self.hl_on() { "on" } else { "off" },
            if self.solid_on() { "on" } else { "off" },
            self.last_ms,
            self.last_bytes,
            rss_kb() / 1024,
            self.top,
            self.doc.rows.len(),
        )
    }

    fn paint(&self, buf: &mut ratatui::buffer::Buffer, area: Rect, scaled: &[usize], status: &str) {
        let th = &self.th;
        if self.solid_on() {
            buf.set_style(area, Style::new().bg(th.ink));
        }
        let pane_w = area.width as usize;
        for r in 0..self.body_h {
            let Some(row) = self.doc.rows.get(self.top + r) else { break };
            let y = area.y + r as u16;
            if let Some((color, w)) = row.bg {
                let w = w.min(pane_w.saturating_sub(row.x)) as u16;
                buf.set_style(Rect::new(row.x as u16, y, w, 1), Style::new().bg(color));
            }
            if let Some(idx) = row.wide {
                let wi = &self.doc.wides[idx];
                let off = self.offsets.get(&idx).copied().unwrap_or(0);
                let line = slice_line(&row.line, off, wi.vis_w);
                buf.set_line(row.x as u16, y, &line, wi.vis_w as u16);
                if off + wi.vis_w < wi.total_w {
                    let x = (row.x + wi.vis_w).saturating_sub(1) as u16;
                    buf[(x, y)].set_symbol("▸").set_fg(th.dim);
                }
                if off > 0 {
                    buf[(row.x as u16, y)].set_symbol("◂").set_fg(th.dim);
                }
                if self.focus == Some(idx) && row.x > 0 {
                    buf[(row.x as u16 - 1, y)].set_symbol("▎").set_fg(th.focus);
                }
            } else if row.heading.is_some_and(|h| scaled.contains(&h)) {
                // Drawn by the OSC 66 pass after ratatui; leave the cells blank.
            } else {
                buf.set_line(row.x as u16, y, &row.line, pane_w.saturating_sub(row.x) as u16);
            }
        }
        let y = area.y + area.height - 1;
        let line = Line::from(Span::styled(status.to_string(), Style::new().fg(th.ink).bg(th.dim)));
        buf.set_style(Rect::new(0, y, area.width, 1), Style::new().bg(th.dim));
        buf.set_line(0, y, &line, area.width);
    }
}

fn to_ct(c: ratatui::style::Color) -> crossterm::style::Color {
    match c {
        ratatui::style::Color::Rgb(r, g, b) => crossterm::style::Color::Rgb { r, g, b },
        _ => crossterm::style::Color::Reset,
    }
}

// ---------------------------------------------------------------- bench

fn pct(sorted: &[f64], p: f64) -> f64 {
    sorted
        .get(((sorted.len() as f64 - 1.0) * p).round() as usize)
        .copied()
        .unwrap_or(0.0)
}

/// Headless: a fixed 200x50 viewport over a byte-counting sink; a scripted scroll through the
/// fixture, then RSS after highlighting every code block.
fn run_bench(mut app: App, osc: bool) -> io::Result<()> {
    let rss_start = rss_kb();
    app.osc = if osc { Osc::On } else { Osc::Off("--no-osc66") };
    let backend = CrosstermBackend::new(Counting(io::sink()));
    let mut term = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 200, 50)),
        },
    )?;
    let t = Instant::now();
    app.frame(&mut term)?; // loads and lays out
    let load_ms = t.elapsed().as_secs_f64() * 1000.0;
    let rss_loaded = rss_kb();
    // Force every code block through the highlighter and every syntax to compile.
    let t = Instant::now();
    highlight::warm();
    let mut langs = 0;
    let blocks = markdown::render(&app.fixtures[app.fixture].md, 88, 192, &app.th);
    for b in &blocks {
        if let markdown::Block::Wide { raw: Some((lang, code)), .. } = b
            && highlight::highlight(lang, code, &app.th).is_some()
        {
            langs += 1;
        }
    }
    let hl_all_ms = t.elapsed().as_secs_f64() * 1000.0;
    let rss_hl = rss_kb();
    let mut ms = Vec::new();
    let mut bytes = Vec::new();
    while app.top < app.max_top() && ms.len() < 3000 {
        app.scroll(3);
        app.frame(&mut term)?;
        ms.push(app.last_ms);
        bytes.push(app.last_bytes as f64);
    }
    ms.sort_by(f64::total_cmp);
    bytes.sort_by(f64::total_cmp);
    let mean = bytes.iter().sum::<f64>() / bytes.len().max(1) as f64;
    println!(
        "bench variant={} fixture={} osc={} hl={} rows={} headings_scaled={} wides={} code_blocks={} hl_blocks={langs} \
         frames={} ms_p50={:.3} ms_p95={:.3} bytes_p50={:.0} bytes_mean={mean:.0} bytes_p95={:.0} \
         load_ms={load_ms:.1} hl_all_ms={hl_all_ms:.1} rss_start_kb={rss_start} rss_loaded_kb={rss_loaded} rss_hl_kb={rss_hl}",
        app.variant().name.split(' ').next().unwrap_or(""),
        app.fixtures[app.fixture].name,
        if osc { "on" } else { "off" },
        if app.hl_on() { "on" } else { "off" },
        app.doc.rows.len(),
        app.doc.headings.len(),
        app.doc.wides.len(),
        app.doc.code_blocks,
        ms.len(),
        pct(&ms, 0.5),
        pct(&ms, 0.95),
        pct(&bytes, 0.5),
        pct(&bytes, 0.95),
    );
    Ok(())
}

// ---------------------------------------------------------------- main

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |n: &str| args.iter().any(|a| a == n);
    let value = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();

    let mut paths: Vec<String> = match value("--fixture") {
        Some(p) => vec![p],
        None => {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
            let mut v: Vec<String> = std::fs::read_dir(&dir)
                .map(|d| {
                    d.flatten()
                        .map(|e| e.path().to_string_lossy().into_owned())
                        .filter(|p| p.ends_with(".jsonl"))
                        .collect()
                })
                .unwrap_or_default();
            v.sort();
            v
        }
    };
    if paths.is_empty() {
        eprintln!("no fixtures: run ./fetch-fixtures.sh first (or pass --fixture PATH)");
        paths.clear();
    }
    let fixtures: Vec<Fixture> = paths.iter().filter_map(|p| load_fixture(p).ok()).collect();

    if flag("--probe") {
        return run_probe(&fixtures, flag("--force-osc66"));
    }
    if fixtures.is_empty() {
        std::process::exit(1);
    }

    let variant = match value("--variant").as_deref() {
        Some("b") => 1,
        Some("c") => 2,
        _ => 0,
    };
    let mut app = App {
        th: Theme::mist(),
        fixtures,
        fixture: 0,
        variant,
        solid: None,
        hl: None,
        no_highlight: flag("--no-highlight"),
        osc: Osc::Off("not probed"),
        key: None,
        doc: Doc {
            rows: Vec::new(),
            headings: Vec::new(),
            wides: Vec::new(),
            code_blocks: 0,
        },
        top: 0,
        focus: None,
        offsets: HashMap::new(),
        last_ms: 0.0,
        last_bytes: 0,
        body_h: 0,
    };
    if let Some(name) = value("--fixture").and_then(|p| {
        std::path::Path::new(&p)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    }) && let Some(i) = app.fixtures.iter().position(|f| f.name == name)
    {
        app.fixture = i;
    }

    if flag("--bench") {
        let osc = value("--osc").as_deref() != Some("off");
        return run_bench(app, osc);
    }

    app.osc = detect(flag("--no-osc66"), flag("--force-osc66"));
    enable_raw_mode()?;
    let mut out = Counting(io::stdout());
    execute!(out, EnterAlternateScreen)?;
    let mut term = Terminal::new(CrosstermBackend::new(out))?;
    let result = run(&mut app, &mut term);
    disable_raw_mode()?;
    execute!(term.backend_mut().writer_mut(), LeaveAlternateScreen, cursor::Show)?;
    result
}

fn run<W: Write>(app: &mut App, term: &mut Terminal<CrosstermBackend<W>>) -> io::Result<()> {
    loop {
        app.frame(term)?;
        // Redraw only on input or resize.
        let Event::Key(KeyEvent { code, modifiers, .. }) = event::read()? else {
            continue;
        };
        let half = (app.body_h / 2).max(1) as isize;
        match code {
            KeyCode::Char('q') => return Ok(()),
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
            KeyCode::Char('j') | KeyCode::Down => app.scroll(1),
            KeyCode::Char('k') | KeyCode::Up => app.scroll(-1),
            KeyCode::Char('d') | KeyCode::PageDown => app.scroll(half),
            KeyCode::Char('u') | KeyCode::PageUp => app.scroll(-half),
            KeyCode::Char('g') => app.top = 0,
            KeyCode::Char('G') => app.top = app.max_top(),
            KeyCode::Tab => app.focus_step(false),
            KeyCode::BackTab => app.focus_step(true),
            KeyCode::Char('l') => app.pan(Some(8)),
            KeyCode::Char('h') => app.pan(Some(-8)),
            KeyCode::Char('0') => app.pan(Some(0)),
            KeyCode::Char('$') => app.pan(None),
            KeyCode::Char(c @ '1'..='3') => app.variant = c as usize - '1' as usize,
            KeyCode::Char('t') => app.fixture = (app.fixture + 1) % app.fixtures.len(),
            KeyCode::Char('S') => app.solid = Some(!app.solid_on()),
            KeyCode::Char('H') => app.hl = Some(!app.hl_on()),
            KeyCode::Char('O') => {
                app.osc = if app.osc_on() { Osc::Off("toggled") } else { Osc::On };
            }
            _ => {}
        }
    }
}
