//! What the converter to Typst does not read itself: macros that hold across a page, and commands it lacks.
use std::sync::{LazyLock, Mutex};

use regex::Regex;

/// Typst draws `cases` columns without a gap and right-aligns the first; LaTeX has a gap and aligns both left.
fn case_columns(text: &str) -> String {
    static CASES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\\begin\{(d?cases|rcases)\}(.*?)\\end\{(?:d?cases|rcases)\}").unwrap());
    CASES
        .replace_all(text, |c: &regex::Captures| {
            let rows: Vec<String> = c[2]
                .split("\\\\")
                .map(|row| match row.split_once('&') {
                    Some((value, condition)) => format!("& {} & \\quad & {}", value.trim(), condition.trim()),
                    None => format!("& {}", row.trim()),
                })
                .collect();
            let name = if &c[1] == "dcases" { "cases" } else { &c[1] };
            format!("\\begin{{{name}}}{}\\end{{{name}}}", rows.join(" \\\\ "))
        })
        .into_owned()
}

/// Pairs plain bars into fences, so `|x|` reads as an absolute value and not as two relations. A single bar stays.
fn absolute_values(text: &str) -> String {
    const SIZED: [&str; 17] = ["left", "right", "middle", "big", "Big", "bigg", "Bigg", "bigl", "bigr", "Bigl", "Bigr", "biggl", "biggr", "Biggl", "Biggr", "bigm", "Bigm"];
    if text.contains("{array}") {
        return text.to_string();
    }
    let mut levels: Vec<Vec<usize>> = vec![Vec::new()];
    let mut pairs: Vec<usize> = Vec::new();
    let mut escaped = false;
    for (at, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '{' => levels.push(Vec::new()),
            '}' => {
                let bars = levels.pop().unwrap_or_default();
                if bars.len() % 2 == 0 {
                    pairs.extend(bars);
                }
                if levels.is_empty() {
                    levels.push(Vec::new());
                }
            }
            '|' if !SIZED.iter().any(|w| text[..at].ends_with(&format!("\\{w}"))) => levels.last_mut().unwrap().push(at),
            _ => {}
        }
    }
    for bars in levels {
        if bars.len() % 2 == 0 {
            pairs.extend(bars);
        }
    }
    pairs.sort_unstable();
    let mut out = String::with_capacity(text.len() + pairs.len() * 6);
    let mut last = 0;
    for (n, at) in pairs.iter().enumerate() {
        out.push_str(&text[last..*at]);
        out.push_str(if n % 2 == 0 { "\\lvert " } else { "\\rvert " });
        last = at + 1;
    }
    out.push_str(&text[last..]);
    out
}

/// Macro definitions seen so far; MathJax, and so Obsidian, keeps them for later formulas on the page.
static MACROS: LazyLock<Mutex<Vec<(String, String)>>> = LazyLock::new(Default::default);

fn group_end(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut escaped = false;
    for (at, c) in text[open..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + at);
                }
            }
            _ => {}
        }
    }
    None
}

/// Rewrites `\name{…}` with what `change` makes of its argument.
fn each_command(text: &str, name: &str, change: impl Fn(&str) -> String) -> String {
    let needle = format!("\\{name}");
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(&needle) {
        let after = at + needle.len();
        let next = rest[after..].chars().next();
        if next.is_some_and(|c| c.is_alphabetic()) {
            out.push_str(&rest[..after]);
            rest = &rest[after..];
            continue;
        }
        let open = after + rest[after..].len() - rest[after..].trim_start().len();
        let close = if rest[open..].starts_with('{') { group_end(rest, open) } else { None };
        out.push_str(&rest[..at]);
        match close {
            Some(close) => {
                out.push_str(&change(&rest[open + 1..close]));
                rest = &rest[close + 1..];
            }
            None => {
                rest = &rest[after..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A chemical formula as math: `SO4^2-` is `\mathrm{SO_4^{2-}}`, `->` an arrow. The common part of mhchem.
fn chemistry(formula: &str) -> String {
    static SPACED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    let mut parts = Vec::new();
    for token in SPACED.split(formula.trim()) {
        parts.push(match token {
            "->" => r"\rightarrow".to_string(),
            "<-" => r"\leftarrow".to_string(),
            "<->" => r"\leftrightarrow".to_string(),
            "<=>" => r"\rightleftharpoons".to_string(),
            "+" => "+".to_string(),
            other => chemical_term(other),
        });
    }
    parts.join(r"\;")
}

fn chemical_term(term: &str) -> String {
    static CHARGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\^\{?([0-9]*[+-])\}?").unwrap());
    static COUNT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([A-Za-z\)\]])([0-9]+)").unwrap());
    let (leading, rest): (String, String) = {
        let digits: String = term.chars().take_while(char::is_ascii_digit).collect();
        (digits.clone(), term[digits.len()..].to_string())
    };
    let counted = COUNT.replace_all(&rest, "${1}_{${2}}");
    let charged = CHARGE.replace_all(&counted, "^{${1}}");
    format!("{leading}\\mathrm{{{charged}}}")
}

fn environments(text: &str) -> String {
    static ALIGNAT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\(begin|end)\{(alignat\*?|alignedat|flalign\*?|xalignat\*?)\}(\{[^}]*\})?").unwrap());
    static MULTILINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\(begin|end)\{multline\*?\}").unwrap());
    static SUB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\(begin|end)\{subequations\}").unwrap());
    let text = ALIGNAT.replace_all(text, "\\$1{aligned}");
    let text = MULTILINE.replace_all(&text, "\\$1{gathered}");
    SUB.replace_all(&text, "").into_owned()
}

/// Remembers the macros a formula defines and takes the definitions out of it.
fn take_definitions(latex: &str) -> String {
    static NEW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\(?:newcommand|renewcommand|providecommand)\*?\s*\{?\\([A-Za-z]+)\}?(?:\[(\d)\])?\s*\{").unwrap());
    static DEF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\(?:def|gdef)\s*\\([A-Za-z]+)(#\d)*\s*\{").unwrap());
    static OPERATOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\DeclareMathOperator(\*?)\s*\{?\\([A-Za-z]+)\}?\s*\{").unwrap());
    let mut text = latex.to_string();
    loop {
        let found = OPERATOR
            .captures(&text)
            .map(|c| (c.get(0).unwrap().range(), c[2].to_string(), Some(!c[1].is_empty())))
            .or_else(|| NEW.captures(&text).map(|c| (c.get(0).unwrap().range(), c[1].to_string(), None)))
            .or_else(|| DEF.captures(&text).map(|c| (c.get(0).unwrap().range(), c[1].to_string(), None)));
        let Some((range, name, operator)) = found else { break };
        let open = range.end - 1;
        let Some(close) = group_end(&text, open) else { break };
        let inner = text[open + 1..close].to_string();
        let body = match operator {
            Some(true) => format!("\\operatorname*{{{inner}}}"),
            Some(false) => format!("\\operatorname{{{inner}}}"),
            None => inner,
        };
        let mut known = MACROS.lock().unwrap();
        known.retain(|(n, _)| *n != name);
        known.push((name, body));
        drop(known);
        text.replace_range(range.start..=close, "");
    }
    text
}

fn expand(text: &str) -> String {
    let known = MACROS.lock().unwrap().clone();
    let mut out = text.to_string();
    for _ in 0..6 {
        let before = out.clone();
        for (name, body) in known.iter().rev() {
            let Ok(pattern) = Regex::new(&format!(r"\\{name}(?:[^A-Za-z]|$)")) else { continue };
            if !pattern.is_match(&out) {
                continue;
            }
            if body.contains("#1") {
                let wanted = body.clone();
                out = each_command(&out, name, |arg| format!("{{{}}}", wanted.replace("#1", arg)));
            } else {
                let exact = Regex::new(&format!(r"\\{name}([^A-Za-z]|$)")).unwrap();
                let replacement = format!("{{{}}}$1", body.replace('$', "$$"));
                out = exact.replace_all(&out, replacement.as_str()).into_owned();
            }
        }
        if out == before {
            break;
        }
    }
    out
}

/// Commands nothing else draws: the look of a fraction is left to the fraction, side marks and rules fall away.
fn odd_commands(text: &str) -> String {
    static GENFRAC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\genfrac\{[^}]*\}\{[^}]*\}\{[^}]*\}\{[^}]*\}").unwrap());
    static SIDESET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\sideset\{[^}]*\}\{[^}]*\}").unwrap());
    static RULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\rule(\[[^\]]*\])?\{[^}]*\}\{[^}]*\}").unwrap());
    static HREF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\(?:href|url|vspace\*?)\{[^}]*\}").unwrap());
    let text = GENFRAC.replace_all(text, "\\frac");
    let text = SIDESET.replace_all(&text, "");
    let text = RULE.replace_all(&text, "");
    HREF.replace_all(&text, "").into_owned()
}

/// Symbols the converter has no working name for, as the characters themselves.
fn symbols(text: &str) -> String {
    static NAMES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\([A-Za-z]+)").unwrap());
    NAMES
        .replace_all(text, |c: &regex::Captures| {
            match &c[1] {
                "llbracket" => "⟦",
                "rrbracket" => "⟧",
                "circledast" => "⊛",
                "circleddash" => "⊝",
                "circledcirc" => "⊚",
                "dashleftarrow" => "⇠",
                "dashrightarrow" => "⇢",
                "mho" => "℧",
                "gtrapprox" => "⪆",
                "lessapprox" => "⪅",
                "gtreqqless" => "⪌",
                "lesseqqgtr" => "⪋",
                "ngeqq" => "≱",
                "nleqq" => "≰",
                "nleqslant" => "≰",
                "ngeqslant" => "≱",
                "nsubseteqq" => "⊈",
                "nsupseteqq" => "⊉",
                "argmax" => "\\operatorname*{arg\\,max}",
                "argmin" => "\\operatorname*{arg\\,min}",
                _ => return c[0].to_string(),
            }
            .to_string()
        })
        .into_owned()
}

/// Reads macros from a whole note before drawing, so a formula may use one from further up in any order of drawing.
pub fn learn_macros(note: &str) {
    if note.contains("\\newcommand") || note.contains("\\def") || note.contains("\\DeclareMathOperator") || note.contains("\\renewcommand") {
        static MATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\$\$(.+?)\$\$|\$([^$\n]+?)\$").unwrap());
        for formula in MATH.captures_iter(note) {
            take_definitions(formula.get(1).or(formula.get(2)).map_or("", |m| m.as_str()));
        }
    }
}

/// The formula in the form the converter understands.
pub fn prepare(latex: &str) -> String {
    static GAP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{c\d+::((?:[^{}]|\{[^{}]*\})*?)(?:::[^{}]*)?\}\}").unwrap());
    let shown = GAP.replace_all(latex, "$1");
    let text = take_definitions(&shown);
    let text = expand(&text);
    let text = absolute_values(&case_columns(&environments(&text)));
    let text = each_command(&text, "ce", |f| format!("{{{}}}", chemistry(f)));
    let text = each_command(&text, "pu", |f| format!("{{\\mathrm{{{f}}}}}"));
    let text = text.replace("\\mathscr", "\\mathcal");
    let text = each_command(&text, "cancelto", |f| format!("\\overset{{{f}}}"));
    let text = each_command(&text, "smash", |f| format!("{{{f}}}"));
    let text = odd_commands(&text);
    let text = symbols(&text);
    each_command(&text, "unicode", |code| {
        let digits = code.trim_start_matches(['x', 'U', '+']);
        u32::from_str_radix(digits, 16).ok().and_then(char::from_u32).map_or(String::new(), |c| c.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chemistry_reads_like_mhchem() {
        assert_eq!(chemistry("H2O"), r"\mathrm{H_{2}O}");
        assert_eq!(chemistry("SO4^2-"), r"\mathrm{SO_{4}^{2-}}");
        assert_eq!(chemistry("2H2 + O2 -> 2H2O"), r"2\mathrm{H_{2}}\;+\;\mathrm{O_{2}}\;\rightarrow\;2\mathrm{H_{2}O}");
    }

    #[test]
    fn macros_hold_for_later_formulas() {
        assert_eq!(prepare(r"\newcommand{\QQ}{\mathbb{Q}}\QQ"), r"{\mathbb{Q}}");
        assert_eq!(prepare(r"x\in\QQ"), r"x\in{\mathbb{Q}}");
        assert_eq!(prepare(r"\def\VV{V}\VV_1"), r"{V}_1");
        assert_eq!(prepare(r"\DeclareMathOperator{\Sp}{Sp}\Sp A"), r"{\operatorname{Sp}} A");
        assert_eq!(prepare(r"\newcommand{\twice}[1]{#1 #1}\twice{a}"), r"{a a}");
    }

    #[test]
    fn bars_pair_up_only_when_they_can() {
        assert_eq!(absolute_values("|x| + |y|"), r"\lvert x\rvert  + \lvert y\rvert ");
        assert_eq!(absolute_values("P(A|B)"), "P(A|B)");
        assert_eq!(absolute_values(r"\left|x\right| \| y \|"), r"\left|x\right| \| y \|");
        assert_eq!(absolute_values(r"\frac{|a|}{b}|c"), r"\frac{\lvert a\rvert }{b}|c");
        assert_eq!(absolute_values(r"\begin{array}{c|c|c}a\end{array}"), r"\begin{array}{c|c|c}a\end{array}");
    }

    #[test]
    fn case_columns_keep_their_gap() {
        assert_eq!(case_columns(r"\begin{cases} x & x>0 \\ -x \end{cases}"), r"\begin{cases}& x & \quad & x>0 \\ & -x\end{cases}");
    }

    #[test]
    fn other_environments_become_ones_that_work() {
        assert_eq!(environments(r"\begin{alignat}{2}a\end{alignat}"), r"\begin{aligned}a\end{aligned}");
        assert_eq!(environments(r"\begin{multline*}a\end{multline*}"), r"\begin{gathered}a\end{gathered}");
    }
}
