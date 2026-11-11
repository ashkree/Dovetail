//! HTML to markdown, for reading (ADR 0018: HTML documents are not rendered as pages). A cached
//! web page or an HTML artifact becomes headings, paragraphs, lists, links, emphasis, code and
//! images; scripts, styles and the `<head>` are dropped. `gx` opens the real page.
//!
//! Hand-written and forgiving: the text is untrusted and may be malformed, so unknown tags are
//! dropped and their text kept.

/// Whether `text` is an HTML document rather than markdown: it starts with a doctype or
/// `<html>`, or opens with a tag and holds several block tags.
pub fn looks_like_html(text: &str) -> bool {
    let start = text.trim_start();
    let head: String = start.chars().take(15).collect::<String>().to_lowercase();
    if head.starts_with("<!doctype html") || head.starts_with("<html") {
        return true;
    }
    if !start.starts_with('<') {
        return false;
    }
    let lower = start.to_lowercase();
    ["</p>", "</div>", "</li>", "</h1>", "</h2>", "</table>"]
        .iter()
        .map(|t| lower.matches(t).count())
        .sum::<usize>()
        >= 3
}

/// Elements whose content is never shown.
const HIDDEN: &[&str] = &["script", "style", "head", "noscript", "template", "svg"];

/// Converts HTML to markdown.
pub fn to_markdown(html: &str) -> String {
    let mut c = Converter::default();
    let mut rest = html;
    while !rest.is_empty() {
        match rest.find('<') {
            Some(0) => {
                let Some(end) = rest.find('>') else {
                    c.text(&rest[1..]);
                    break;
                };
                c.tag(&rest[1..end]);
                rest = &rest[end + 1..];
            }
            Some(at) => {
                c.text(&rest[..at]);
                rest = &rest[at..];
            }
            None => {
                c.text(rest);
                break;
            }
        }
    }
    tidy(&c.out)
}

#[derive(Default)]
struct Converter {
    out: String,
    /// Inside a hidden element: its name and how deep.
    hidden: Option<(String, usize)>,
    pre: bool,
    /// Open lists: `None` for bullets, the next number for an ordered list.
    lists: Vec<Option<u32>>,
    /// Open links' targets.
    links: Vec<String>,
}

impl Converter {
    fn tag(&mut self, raw: &str) {
        if raw.starts_with('!') || raw.starts_with('?') {
            return;
        }
        let closing = raw.starts_with('/');
        let body = raw.trim_start_matches('/').trim_end_matches('/');
        let name = body
            .split(|c: char| c.is_whitespace())
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if let Some((hidden, depth)) = &mut self.hidden {
            if *hidden == name {
                if closing {
                    *depth -= 1;
                    if *depth == 0 {
                        self.hidden = None;
                    }
                } else if !raw.ends_with('/') {
                    *depth += 1;
                }
            }
            return;
        }
        if HIDDEN.contains(&name.as_str()) {
            if !closing && !raw.ends_with('/') {
                self.hidden = Some((name, 1));
            }
            return;
        }
        match (name.as_str(), closing) {
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                let level = name[1..].parse::<usize>().unwrap_or(1);
                self.block();
                self.out.push_str(&"#".repeat(level));
                self.out.push(' ');
            }
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => self.block(),
            (
                "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "nav"
                | "table" | "figure" | "dl",
                _,
            ) => self.block(),
            ("tr" | "dt" | "dd" | "figcaption", _) => self.out.push('\n'),
            ("td" | "th", false) => self.out.push(' '),
            ("br", _) => self.out.push('\n'),
            ("hr", _) => {
                self.block();
                self.out.push_str("---");
                self.block();
            }
            ("ul", false) => {
                self.block();
                self.lists.push(None);
            }
            ("ol", false) => {
                self.block();
                self.lists.push(Some(1));
            }
            ("ul" | "ol", true) => {
                self.lists.pop();
                self.block();
            }
            ("li", false) => {
                let indent = "  ".repeat(self.lists.len().saturating_sub(1));
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "- ".to_string(),
                };
                self.out.push('\n');
                self.out.push_str(&indent);
                self.out.push_str(&marker);
            }
            ("blockquote", false) => {
                self.block();
                self.out.push_str("> ");
            }
            ("blockquote", true) => self.block(),
            ("strong" | "b", _) => self.out.push_str("**"),
            ("em" | "i", _) => self.out.push('*'),
            ("code", _) if !self.pre => self.out.push('`'),
            ("pre", false) => {
                self.block();
                self.out.push_str("```\n");
                self.pre = true;
            }
            ("pre", true) => {
                self.pre = false;
                if !self.out.ends_with('\n') {
                    self.out.push('\n');
                }
                self.out.push_str("```");
                self.block();
            }
            ("a", false) => {
                self.links.push(attr(body, "href").unwrap_or_default());
                self.out.push('[');
            }
            ("a", true) => {
                let href = self.links.pop().unwrap_or_default();
                if href.is_empty() || href.starts_with('#') || href.starts_with("javascript:") {
                    self.out.push(']');
                } else {
                    self.out.push_str(&format!("]({href})"));
                }
            }
            ("img", _) => {
                let src = attr(body, "src").unwrap_or_default();
                let alt = attr(body, "alt").unwrap_or_default();
                if !src.is_empty() {
                    self.out.push_str(&format!("![{alt}]({src})"));
                }
            }
            _ => {}
        }
    }

    fn text(&mut self, raw: &str) {
        if self.hidden.is_some() {
            return;
        }
        let text = decode(raw);
        if self.pre {
            self.out.push_str(&text);
            return;
        }
        let mut last_space = self.out.ends_with([' ', '\n']) || self.out.is_empty();
        for ch in text.chars() {
            if ch.is_whitespace() {
                if !last_space {
                    self.out.push(' ');
                    last_space = true;
                }
            } else {
                self.out.push(ch);
                last_space = false;
            }
        }
    }

    /// A blank line before what follows.
    fn block(&mut self) {
        self.out.push_str("\n\n");
    }
}

/// The value of attribute `name` in a tag's text, quoted or not.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name) {
        let start = from + at;
        from = start + name.len();
        let before = lower[..start].chars().next_back();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or(""),
            _ => rest.split(|c: char| c.is_whitespace()).next().unwrap_or(""),
        };
        return Some(decode(value));
    }
    None
}

/// Decodes the common character references.
fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest.find(';').filter(|&e| e <= 10);
        let decoded = end.and_then(|e| {
            let name = &rest[1..e];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                "mdash" => Some('—'),
                "ndash" => Some('–'),
                "hellip" => Some('…'),
                _ => name.strip_prefix('#').and_then(|n| {
                    let code = match n.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => n.parse().ok(),
                    };
                    code.and_then(char::from_u32)
                }),
            };
            ch.map(|c| (c, e))
        });
        match decoded {
            Some((ch, e)) => {
                out.push(ch);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Trims each line's trailing space and keeps at most one blank line in a row, except inside
/// code fences.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut blank = true;
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let line = if fenced { line } else { line.trim() };
        if line.is_empty() && !fenced {
            if !blank {
                out.push('\n');
            }
            blank = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
        blank = false;
    }
    out.trim_end().to_string() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_told_from_markdown() {
        assert!(looks_like_html(
            "<!DOCTYPE html><html><body>x</body></html>"
        ));
        assert!(looks_like_html("  <html lang=en>"));
        assert!(looks_like_html("<div><p>a</p><p>b</p><p>c</p></div>"));
        assert!(!looks_like_html("# Report\n\nText with <b>bold</b>."));
        assert!(!looks_like_html("<b>one</b> tag"));
    }

    #[test]
    fn blocks_inline_styles_links_and_images_convert() {
        let html = "<html><head><title>T</title><style>p{}</style></head><body>\
                    <h1>Crate &amp; more</h1><p><strong>nucleo</strong> is <em>fast</em>. See \
                    <a href=\"https://docs.rs/x\"><code>Matcher</code></a>.</p>\
                    <ul><li>one</li><li>two</li></ul><ol><li>a</li><li>b</li></ol>\
                    <img src=\"fig.png\" alt=\"A figure\"><script>evil()</script>\
                    <pre><code>let x = 1;\n  y</code></pre></body></html>";
        let md = to_markdown(html);
        assert_eq!(
            md,
            "# Crate & more\n\n**nucleo** is *fast*. See [`Matcher`](https://docs.rs/x).\n\n\
             - one\n- two\n\n1. a\n2. b\n\n![A figure](fig.png)\n\n```\nlet x = 1;\n  y\n```\n"
        );
    }

    #[test]
    fn malformed_html_keeps_its_text() {
        let md = to_markdown("<p>unclosed <b>bold <div>and &unknown; &#x41; &#66;</p><");
        assert!(md.contains("unclosed **bold"), "{md}");
        assert!(md.contains("&unknown; A B"), "{md}");
    }
}
