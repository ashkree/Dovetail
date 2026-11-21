//! Small formatting helpers shared by the buffers.

use jiff::{Timestamp, tz::TimeZone};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// `HH:MM` in `tz`.
pub fn hhmm(ms: i64, tz: &TimeZone) -> String {
    match Timestamp::from_millisecond(ms) {
        Ok(ts) => ts.to_zoned(tz.clone()).strftime("%H:%M").to_string(),
        Err(_) => "--:--".into(),
    }
}

/// Date and time, as `Nov 3 10:14`, in `tz`.
pub fn when(ms: i64, tz: &TimeZone) -> String {
    match Timestamp::from_millisecond(ms) {
        Ok(ts) => ts.to_zoned(tz.clone()).strftime("%b %-d %H:%M").to_string(),
        Err(_) => "--".into(),
    }
}

/// A file size as `412 KB` or `1.2 MB` (binary units, at least `1 KB`).
pub fn bytes(size: u64) -> String {
    dovetail_api::conversation::size_text(size)
}

/// An attachment chip, as in the chat bar and the conversation: `[image 1 · 412 KB]`.
pub fn image_chip(n: usize, size: u64) -> String {
    format!("[image {n} · {}]", bytes(size))
}

/// A token count as `950`, `17k` or `1.2M`.
pub fn tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}k", n / 1_000)
    } else {
        n.to_string()
    }
}

/// A run time as `45s` or `3m 05s`.
pub fn duration(ms: u64) -> String {
    let s = ms / 1_000;
    if s >= 60 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

/// Cuts `text` to at most `width` terminal cells, ending in `…` when it was cut.
pub fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// First line of `text`, for one-line previews.
pub fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

/// Greedy word wrap to `width` cells; words longer than a line are split. Keeps blank lines.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for word in paragraph.split_whitespace() {
            let w = word.width();
            if used > 0 && used + 1 + w > width {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            if used > 0 {
                line.push(' ');
                used += 1;
            }
            if w > width {
                for c in word.chars() {
                    let cw = c.width().unwrap_or(0);
                    if used + cw > width {
                        lines.push(std::mem::take(&mut line));
                        used = 0;
                    }
                    line.push(c);
                    used += cw;
                }
            } else {
                line.push_str(word);
                used += w;
            }
        }
        lines.push(line);
    }
    lines
}

/// Time since `then_ms`, as the switcher shows it: `now`, `2m`, `3h`, `4d`.
pub fn ago(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1_000;
    match secs {
        0..=44 => "now".into(),
        45..=3_599 => format!("{}m", (secs + 30) / 60),
        3_600..=86_399 => format!("{}h", secs / 3_600),
        _ => format!("{}d", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ago_picks_a_unit() {
        assert_eq!(ago(100_000, 100_000), "now");
        assert_eq!(ago(100_000, 70_000), "now");
        assert_eq!(ago(200_000, 80_000), "2m");
        assert_eq!(ago(10_000_000, 0), "2h");
        assert_eq!(ago(200_000_000, 0), "2d");
        assert_eq!(ago(0, 5_000), "now", "a time in the future reads as now");
    }

    #[test]
    fn truncate_marks_the_cut() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
        assert_eq!(truncate("abc", 0), "");
    }

    #[test]
    fn wrap_breaks_on_words_and_splits_long_ones() {
        assert_eq!(wrap("aa bb cc", 5), vec!["aa bb", "cc"]);
        assert_eq!(wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(wrap("a\n\nb", 5), vec!["a", "", "b"]);
    }
}
