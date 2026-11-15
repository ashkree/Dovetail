//! `gui-text` spike (ADR 0020): a long fake coordinator transcript in egui at the mockup's
//! typography, with RSS, frame count and frame time on screen. Throwaway.
//!
//! Keys: j/k line, d/u half page, Space/b page, g/G top/bottom, Tab toggles all tool rows,
//! +/- text size, l line height, q quits.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui::{
    self, Align, Color32, FontData, FontDefinitions, FontFamily, FontId, Key, Layout, Margin,
    Rect, RichText, Sense, TextFormat, UiBuilder, Vec2, text::LayoutJob,
};

const BG: Color32 = Color32::from_rgb(0x17, 0x1b, 0x23);
const PANEL: Color32 = Color32::from_rgb(0x1d, 0x22, 0x2c);
const LINE: Color32 = Color32::from_rgb(0x2c, 0x33, 0x40);
const BODY: Color32 = Color32::from_rgb(0xd0, 0xd6, 0xf2);
const STRONG: Color32 = Color32::from_rgb(0xee, 0xf1, 0xfc);
const DIM: Color32 = Color32::from_rgb(0x9a, 0x9b, 0xb2);
const ACCENT: Color32 = Color32::from_rgb(0xb4, 0xbe, 0xfe);
const YOU: Color32 = Color32::from_rgb(0xf5, 0xc2, 0xe7);
const CODE: Color32 = Color32::from_rgb(0xa6, 0xe3, 0xa1);
const CODE_BG: Color32 = Color32::from_rgb(0x14, 0x18, 0x20);

const MEASURE: f32 = 720.0;
const TURN_GAP: f32 = 22.0;
const ESTIMATE: f32 = 140.0;

struct Args {
    turns: usize,
    size: f32,
    line_height: f32,
    renderer: eframe::Renderer,
    body: PathBuf,
    bold: Option<PathBuf>,
    mono: PathBuf,
}

fn first_existing(candidates: &[&str]) -> Option<PathBuf> {
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}

fn parse_args() -> Args {
    let mut turns = 2000;
    let mut size = 15.0;
    let mut line_height = 1.65;
    let mut renderer = eframe::Renderer::Glow;
    let mut body = first_existing(&[
        "/usr/share/fonts/inter/InterVariable.ttf",
        "/usr/share/fonts/inter/Inter-Regular.otf",
        "/usr/share/fonts/TTF/Inter-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    ]);
    let mut bold = first_existing(&[
        "/usr/share/fonts/inter/Inter-SemiBold.otf",
        "/usr/share/fonts/TTF/Inter-SemiBold.ttf",
        "/usr/share/fonts/noto/NotoSans-Medium.ttf",
        "/usr/share/fonts/noto/NotoSans-Bold.ttf",
    ]);
    let mut mono = first_existing(&[
        "/usr/share/fonts/TTF/JetBrainsMono-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    ]);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--turns" => turns = val().parse().expect("--turns N"),
            "--size" => size = val().parse().expect("--size PX"),
            "--line-height" => line_height = val().parse().expect("--line-height X"),
            "--renderer" => {
                renderer = match val().as_str() {
                    "wgpu" => eframe::Renderer::Wgpu,
                    _ => eframe::Renderer::Glow,
                }
            }
            "--font" => body = Some(val().into()),
            "--bold" => bold = Some(val().into()),
            "--mono" => mono = Some(val().into()),
            other => panic!("unknown argument {other}"),
        }
    }
    Args {
        turns,
        size,
        line_height,
        renderer,
        body: body.expect("no body font found; pass --font PATH"),
        bold,
        mono: mono.expect("no mono font found; pass --mono PATH"),
    }
}

// ---- fake transcript -------------------------------------------------------------------------

enum Span {
    Text(&'static str),
    Bold(&'static str),
    Code(&'static str),
}

enum Block {
    Para(Vec<Span>),
    CodeBlock(&'static str),
    Tool(String),
}

struct Turn {
    you: bool,
    time: String,
    blocks: Vec<Block>,
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }
}

const ASKS: &[&str] = &[
    "What's blocking slice-7?",
    "Make sure slice-4 stays out of the decision path until the fix lands.",
    "Can you split the storage work into two slices, one for the schema and one for the migration runner?",
    "Why did the last run fail?",
    "Give me the merge order for everything that's open.",
    "Start a slice for the inbox keyboard rework and keep it small.",
];

const SENTENCES: &[&[Span]] = &[
    &[Span::Text("It needs a key. slice-7 can only reach the eval path with "), Span::Code("FS_EVAL_VIRTUAL_KEY"), Span::Text(" set.")],
    &[Span::Text("It asked for it in the inbox as a Bash approval. "), Span::Bold("Allow it there"), Span::Text(" and the reproduction carries on.")],
    &[Span::Text("The merge order is #1231 first, then the 500 fix rebased on top, then the docs change, which touches nothing the others do.")],
    &[Span::Text("I told slice-4 to stay out of "), Span::Code("src/decision.rs"), Span::Text(" until slice-7 hands it back. It acknowledged and moved on to the OpenAPI spec.")],
    &[Span::Text("The test failure is a race in the fixture: two sessions share one scratch repository, and the second one deletes the branch the first is still on. "), Span::Bold("Giving each test its own directory fixes it.")],
    &[Span::Text("Storage splits cleanly. The schema slice owns "), Span::Code("migrations/"), Span::Text(" and "), Span::Code("schema.rs"), Span::Text("; the runner slice owns "), Span::Code("db/thread.rs"), Span::Text(". Neither needs the other's code to start.")],
    &[Span::Text("Nothing else needs you right now. slice-4 is about twenty minutes from a PR and slice-7 is waiting on the key.")],
    &[Span::Text("I read both diffs. They agree on the event shape but disagree on whether "), Span::Code("seq"), Span::Text(" restarts after a resync; the protocol says it does not, so slice-4's version is right.")],
];

const CODE_BLOCKS: &[&str] = &[
    "FS_EVAL_VIRTUAL_KEY=dev cargo test -p api get_decision",
    "fn apply(snapshot: &mut Snapshot, event: &Event) {\n    match event {\n        Event::TurnStarted { session, .. } => snapshot.mark_working(*session),\n        Event::TurnFinished { session, .. } => snapshot.mark_idle(*session),\n        _ => {}\n    }\n}",
    "git rebase origin/main && cargo test -p dovetail-core",
];

const TOOLS: &[&str] = &[
    "Read 2 files · 4 s",
    "Checked slice-7's stream · 1 s",
    "Sent 1 message · 2 s",
    "Ran cargo test · 38 s",
    "Searched 14 files for apply( · 2 s",
];

fn transcript(n: usize) -> Vec<Turn> {
    let mut rng = Lcg(0x5eed);
    (0..n)
        .map(|i| {
            let minutes = 8 * 60 + i;
            let time = format!("{:02}:{:02}", (minutes / 60) % 24, minutes % 60);
            if i % 2 == 0 {
                let ask = ASKS[rng.next(ASKS.len())];
                return Turn { you: true, time, blocks: vec![Block::Para(vec![Span::Text(ask)])] };
            }
            let mut blocks = Vec::new();
            for _ in 0..1 + rng.next(4) {
                let s = SENTENCES[rng.next(SENTENCES.len())];
                blocks.push(Block::Para(
                    s.iter()
                        .map(|sp| match sp {
                            Span::Text(t) => Span::Text(t),
                            Span::Bold(t) => Span::Bold(t),
                            Span::Code(t) => Span::Code(t),
                        })
                        .collect(),
                ));
                match rng.next(6) {
                    0 => blocks.push(Block::CodeBlock(CODE_BLOCKS[rng.next(CODE_BLOCKS.len())])),
                    1 | 2 => blocks.push(Block::Tool(TOOLS[rng.next(TOOLS.len())].to_string())),
                    _ => {}
                }
            }
            Turn { you: false, time, blocks }
        })
        .collect()
}

// ---- app -------------------------------------------------------------------------------------

struct App {
    turns: Vec<Turn>,
    heights: Vec<f32>,
    heights_width: f32,
    open_tools: HashSet<(usize, usize)>,
    all_open: bool,
    size: f32,
    line_height: f32,
    frames: u64,
    last_frame_ms: f32,
    font_name: String,
    renderer: &'static str,
}

fn rss_mb() -> f32 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<f32>().ok())
        .map(|pages| pages * 4096.0 / 1_048_576.0)
        .unwrap_or(0.0)
}

fn peak_rss_mb() -> f32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            let line = s.lines().find(|l| l.starts_with("VmHWM:"))?;
            line.split_whitespace().nth(1)?.parse::<f32>().ok()
        })
        .map(|kb| kb / 1024.0)
        .unwrap_or(0.0)
}

fn dimmed(c: Color32, old: bool) -> Color32 {
    if old { c.gamma_multiply(0.6) } else { c }
}

impl App {
    fn para_job(&self, spans: &[Span], width: f32, old: bool) -> LayoutJob {
        let mut job = LayoutJob { wrap: egui::text::TextWrapping { max_width: width, ..Default::default() }, ..Default::default() };
        let lh = Some(self.size * self.line_height);
        let body = FontId::new(self.size, FontFamily::Proportional);
        let bold = FontId::new(self.size, FontFamily::Name("bold".into()));
        let mono = FontId::new(self.size * 0.9, FontFamily::Monospace);
        for sp in spans {
            let (text, fmt) = match sp {
                Span::Text(t) => (*t, TextFormat { font_id: body.clone(), color: dimmed(BODY, old), line_height: lh, ..Default::default() }),
                Span::Bold(t) => (*t, TextFormat { font_id: bold.clone(), color: dimmed(STRONG, old), line_height: lh, ..Default::default() }),
                Span::Code(t) => (*t, TextFormat { font_id: mono.clone(), color: dimmed(CODE, old), background: CODE_BG, line_height: lh, ..Default::default() }),
            };
            job.append(text, 0.0, fmt);
        }
        job
    }

    fn turn_ui(&mut self, ui: &mut egui::Ui, i: usize, width: f32) {
        let old = i + 4 < self.turns.len();
        let you = self.turns[i].you;
        let header = |ui: &mut egui::Ui, time: &str| {
            ui.horizontal(|ui| {
                let (name, color) = if you { ("You", YOU) } else { ("coordinator", ACCENT) };
                ui.label(RichText::new(name).size(12.0).color(dimmed(color, old)).family(FontFamily::Name("bold".into())));
                ui.label(RichText::new(format!("· {time}")).size(12.0).color(dimmed(DIM, old)));
            });
        };
        let frame = if you && !old {
            egui::Frame::new().fill(PANEL).corner_radius(10).inner_margin(Margin::symmetric(16, 14))
        } else {
            egui::Frame::new()
        };
        frame.show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            let inner = if you && !old { width - 32.0 } else { width };
            header(ui, &self.turns[i].time.clone());
            for b in 0..self.turns[i].blocks.len() {
                match &self.turns[i].blocks[b] {
                    Block::Para(spans) => {
                        let job = self.para_job(spans, inner, old);
                        ui.label(job);
                    }
                    Block::CodeBlock(code) => {
                        let lh = Some(self.size * 0.9 * 1.5);
                        let mut job = LayoutJob::default();
                        job.append(code, 0.0, TextFormat { font_id: FontId::new(self.size * 0.9, FontFamily::Monospace), color: dimmed(CODE, old), line_height: lh, ..Default::default() });
                        egui::Frame::new().fill(CODE_BG).corner_radius(6).inner_margin(Margin::same(10)).show(ui, |ui| {
                            ui.set_width(inner - 20.0);
                            ui.label(job);
                        });
                    }
                    Block::Tool(label) => {
                        let open = self.all_open || self.open_tools.contains(&(i, b));
                        let arrow = if open { "▾" } else { "▸" };
                        let resp = ui.add(egui::Label::new(RichText::new(format!("{arrow} {label}")).size(12.5).color(DIM)).sense(Sense::click()));
                        if resp.clicked() && !self.open_tools.remove(&(i, b)) {
                            self.open_tools.insert((i, b));
                        }
                        if open {
                            ui.label(RichText::new("  src/core/apply.rs\n  src/core/snapshot.rs").size(12.5).color(DIM).family(FontFamily::Monospace));
                        }
                    }
                }
            }
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        let started = Instant::now();
        self.frames += 1;

        let mut scroll = 0.0f32;
        let mut jump: Option<bool> = None;
        let page = ctx.content_rect().height() - 120.0;
        let step = self.size * self.line_height * 3.0;
        ctx.input(|i| {
            if i.key_pressed(Key::J) { scroll -= step; }
            if i.key_pressed(Key::K) { scroll += step; }
            if i.key_pressed(Key::D) { scroll -= page / 2.0; }
            if i.key_pressed(Key::U) { scroll += page / 2.0; }
            if i.key_pressed(Key::Space) && !i.modifiers.shift { scroll -= page; }
            if i.key_pressed(Key::B) || (i.key_pressed(Key::Space) && i.modifiers.shift) { scroll += page; }
            if i.key_pressed(Key::G) { jump = Some(i.modifiers.shift); }
            if i.key_pressed(Key::Tab) { self.all_open = !self.all_open; self.heights_width = 0.0; }
            if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) { self.size += 1.0; self.heights_width = 0.0; }
            if i.key_pressed(Key::Minus) { self.size = (self.size - 1.0).max(10.0); self.heights_width = 0.0; }
            if i.key_pressed(Key::L) { self.line_height = if self.line_height > 1.6 { 1.4 } else { 1.65 }; self.heights_width = 0.0; }
            if i.key_pressed(Key::Q) { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
        });

        egui::Panel::top("stats").frame(egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(20, 8))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("coordinator").size(14.0).color(STRONG).family(FontFamily::Name("bold".into())));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!(
                        "RSS {:.0} MB (peak {:.0}) · frames {} · last frame {:.1} ms · {} turns · {} px / {:.2} · {} · {}",
                        rss_mb(), peak_rss_mb(), self.frames, self.last_frame_ms, self.turns.len(), self.size, self.line_height, self.font_name, self.renderer
                    )).size(12.0).color(DIM).family(FontFamily::Monospace));
                });
            });
            ui.painter().hline(ui.max_rect().x_range(), ui.max_rect().bottom() + 8.0, (1.0, LINE));
        });

        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG)).show(ui, |ui| {
            let mut area = egui::ScrollArea::vertical().auto_shrink(false).animated(true);
            match jump {
                Some(false) => area = area.vertical_scroll_offset(0.0),
                Some(true) => area = area.vertical_scroll_offset(f32::MAX),
                None => {}
            }
            area.show_viewport(ui, |ui, viewport| {
                if scroll != 0.0 {
                    ui.scroll_with_delta(Vec2::new(0.0, scroll));
                }
                let full = ui.max_rect();
                let width = MEASURE.min(full.width() - 56.0);
                let left = full.center().x - width / 2.0;
                if (self.heights_width - width).abs() > 0.5 {
                    self.heights = vec![ESTIMATE; self.turns.len()];
                    self.heights_width = width;
                }
                let mut y = 28.0;
                let mut changed = false;
                for i in 0..self.turns.len() {
                    let h = self.heights[i];
                    if y + h >= viewport.min.y && y <= viewport.max.y {
                        let rect = Rect::from_min_size(egui::pos2(left, full.top() + y), Vec2::new(width, f32::INFINITY));
                        let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
                        self.turn_ui(&mut child, i, width);
                        let measured = child.min_rect().height();
                        if (measured - h).abs() > 0.5 {
                            self.heights[i] = measured;
                            changed = true;
                        }
                    }
                    y += self.heights[i] + TURN_GAP;
                }
                ui.allocate_rect(Rect::from_min_size(full.min, Vec2::new(full.width(), y + 28.0)), Sense::hover());
                if changed {
                    ctx.request_repaint();
                }
            });
        });

        self.last_frame_ms = started.elapsed().as_secs_f32() * 1000.0;
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        eprintln!(
            "gui-text: renderer {} · font {} · {} turns · {} frames · peak RSS {:.0} MB · RSS at exit {:.0} MB",
            self.renderer, self.font_name, self.turns.len(), self.frames, peak_rss_mb(), rss_mb()
        );
    }
}

fn main() -> eframe::Result {
    let args = parse_args();
    let font_name = args.body.file_stem().unwrap().to_string_lossy().into_owned();
    let renderer = if args.renderer == eframe::Renderer::Wgpu { "wgpu" } else { "glow" };
    let read = |p: &PathBuf| std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));

    let mut fonts = FontDefinitions::empty();
    fonts.font_data.insert("body".into(), Arc::new(FontData::from_owned(read(&args.body))));
    fonts.font_data.insert("mono".into(), Arc::new(FontData::from_owned(read(&args.mono))));
    let bold_key = if let Some(b) = &args.bold {
        fonts.font_data.insert("bold".into(), Arc::new(FontData::from_owned(read(b))));
        "bold"
    } else {
        "body"
    };
    fonts.families.insert(FontFamily::Proportional, vec!["body".into(), "mono".into()]);
    fonts.families.insert(FontFamily::Monospace, vec!["mono".into(), "body".into()]);
    fonts.families.insert(FontFamily::Name("bold".into()), vec![bold_key.into(), "body".into(), "mono".into()]);

    let turns = transcript(args.turns);
    let app = App {
        heights: Vec::new(),
        heights_width: 0.0,
        open_tools: HashSet::new(),
        all_open: false,
        size: args.size,
        line_height: args.line_height,
        frames: 0,
        last_frame_ms: 0.0,
        font_name,
        renderer,
        turns,
    };

    let options = eframe::NativeOptions {
        renderer: args.renderer,
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 860.0]).with_title("gui-text spike"),
        ..Default::default()
    };
    eframe::run_native(
        "gui-text spike",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_fonts(fonts);
            cc.egui_ctx.all_styles_mut(|s| {
                s.visuals = egui::Visuals::dark();
                s.visuals.panel_fill = BG;
                s.visuals.override_text_color = None;
                s.visuals.widgets.noninteractive.fg_stroke.color = BODY;
            });
            Ok(Box::new(app))
        }),
    )
}
