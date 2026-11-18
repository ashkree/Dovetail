//! Syntax highlighting with syntect, scopes mapped onto Mist (no theme files shipped):
//! keyword/storage → bold colour + BOLD, string → `code`, comment → `dim`, type → `italic`
//! colour, number → `needs_you`, function name → `link`; the rest is body. Compiled out without
//! the `highlight` feature, so RSS can be compared.

use ratatui::text::Line;

use crate::theme::Theme;

#[cfg(feature = "highlight")]
mod imp {
    use std::sync::OnceLock;

    use ratatui::style::{Modifier, Style};
    use ratatui::text::{Line, Span};
    use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

    use crate::theme::Theme;

    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();

    pub fn warm() {
        syntaxes();
    }

    fn syntaxes() -> &'static SyntaxSet {
        SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines)
    }

    struct Scopes {
        comment: Scope,
        string: Scope,
        keyword: Scope,
        storage: Scope,
        ty: [Scope; 2],
        number: Scope,
        func: Scope,
    }

    fn scopes() -> Scopes {
        let s = |n: &str| Scope::new(n).expect("valid scope");
        Scopes {
            comment: s("comment"),
            string: s("string"),
            keyword: s("keyword"),
            storage: s("storage"),
            ty: [s("entity.name.type"), s("support.type")],
            number: s("constant.numeric"),
            func: s("entity.name.function"),
        }
    }

    fn style(stack: &ScopeStack, sc: &Scopes, theme: &Theme) -> Style {
        let has = |p: &Scope| stack.as_slice().iter().any(|s| p.is_prefix_of(*s));
        if has(&sc.comment) {
            theme.dim()
        } else if has(&sc.string) {
            theme.fg(theme.code)
        } else if has(&sc.keyword) || has(&sc.storage) {
            Style::new().fg(theme.bold).add_modifier(Modifier::BOLD)
        } else if sc.ty.iter().any(has) {
            Style::new().fg(theme.italic).add_modifier(Modifier::ITALIC)
        } else if has(&sc.number) {
            theme.fg(theme.needs_you)
        } else if has(&sc.func) {
            theme.fg(theme.link)
        } else {
            theme.body()
        }
    }

    pub fn highlight(lang: &str, code: &str, theme: &Theme) -> Option<Vec<Line<'static>>> {
        if lang.is_empty() {
            return None;
        }
        let ss = syntaxes();
        let syntax = ss.find_syntax_by_token(lang)?;
        let sc = scopes();
        let mut state = ParseState::new(syntax);
        let mut stack = ScopeStack::new();
        let mut out = Vec::new();
        for raw in code.split('\n') {
            let line = format!("{raw}\n");
            let ops = state.parse_line(&line, ss).ok()?;
            let mut spans = Vec::new();
            let mut last = 0;
            let mut push = |stack: &ScopeStack, text: &str| {
                let text = text.trim_end_matches('\n');
                if !text.is_empty() {
                    spans.push(Span::styled(text.to_string(), style(stack, &sc, theme)));
                }
            };
            for (i, op) in ops {
                push(&stack, &line[last..i]);
                stack.apply(&op).ok()?;
                last = i;
            }
            push(&stack, &line[last..]);
            out.push(Line::from(spans));
        }
        Some(out)
    }
}

/// Highlighted lines of `code`, or `None` when the language is unknown or the feature is off.
pub fn highlight(lang: &str, code: &str, theme: &Theme) -> Option<Vec<Line<'static>>> {
    #[cfg(feature = "highlight")]
    {
        imp::highlight(lang, code, theme)
    }
    #[cfg(not(feature = "highlight"))]
    {
        let _ = (lang, code, theme);
        None
    }
}

/// Loads the syntax set now (it is lazy otherwise), so RSS can be read before and after.
pub fn warm() {
    #[cfg(feature = "highlight")]
    imp::warm();
}

pub const ENABLED: bool = cfg!(feature = "highlight");
