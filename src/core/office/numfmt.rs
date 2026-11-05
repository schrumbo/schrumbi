//! Excel number formats: a format code turns a cell value into the text Excel shows.

/// Separators and names a number is written with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale {
    pub german: bool,
}

impl Locale {
    pub fn current() -> Self {
        Self { german: crate::core::i18n::lang() == crate::core::i18n::Lang::De }
    }

    fn decimal(self) -> char {
        if self.german { ',' } else { '.' }
    }

    fn group(self) -> char {
        if self.german { '.' } else { ',' }
    }
}

const MONTHS_DE: [&str; 12] = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
const MONTHS_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS_DE: [&str; 7] = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"];
const DAYS_EN: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// The code of a built-in format id, as the locale shows it.
pub fn builtin(id: u32, locale: Locale) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "#,##0 \"€\";-#,##0 \"€\"",
        6 => "#,##0 \"€\";[Red]-#,##0 \"€\"",
        7 => "#,##0.00 \"€\";-#,##0.00 \"€\"",
        8 => "#,##0.00 \"€\";[Red]-#,##0.00 \"€\"",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 if locale.german => "dd.mm.yyyy",
        14 => "m/d/yyyy",
        15 if locale.german => "dd. mmm yy",
        15 => "d-mmm-yy",
        16 if locale.german => "dd. mmm",
        16 => "d-mmm",
        17 if locale.german => "mmm yy",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "hh:mm",
        21 => "hh:mm:ss",
        22 if locale.german => "dd.mm.yyyy hh:mm",
        22 => "m/d/yyyy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mm:ss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Lit(String),
    Zero,
    Hash,
    Query,
    Dot,
    Comma,
    Percent,
    Exp(bool),
    Slash,
    Year(u8),
    Month(u8),
    Minute(u8),
    Day(u8),
    Hour(u8),
    Second(u8),
    SubSecond(u8),
    Elapsed(char, u8),
    AmPm(bool),
    At,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Cond {
    Lt(f64),
    Le(f64),
    Gt(f64),
    Ge(f64),
    Eq(f64),
    Ne(f64),
}

impl Cond {
    fn holds(self, v: f64) -> bool {
        match self {
            Cond::Lt(x) => v < x,
            Cond::Le(x) => v <= x,
            Cond::Gt(x) => v > x,
            Cond::Ge(x) => v >= x,
            Cond::Eq(x) => v == x,
            Cond::Ne(x) => v != x,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Section {
    toks: Vec<Tok>,
    color: Option<u32>,
    cond: Option<Cond>,
    date: bool,
    general: bool,
}

/// A parsed format code, ready to format many values.
#[derive(Debug, Clone, PartialEq)]
pub struct Format {
    sections: Vec<Section>,
}

/// What a value looks like in its cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub text: String,
    pub color: Option<u32>,
}

fn color_name(name: &str) -> Option<u32> {
    Some(match name.to_ascii_lowercase().as_str() {
        "black" | "schwarz" => 0x000000,
        "blue" | "blau" => 0x0000ff,
        "cyan" | "zyan" => 0x00ffff,
        "green" | "grün" => 0x008000,
        "magenta" => 0xff00ff,
        "red" | "rot" => 0xff0000,
        "white" | "weiß" => 0xffffff,
        "yellow" | "gelb" => 0xffff00,
        _ => return None,
    })
}

fn split_sections(code: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let (mut quoted, mut bracket, mut escape) = (false, false, false);
    for c in code.chars() {
        let cur = out.last_mut().unwrap();
        if escape {
            cur.push(c);
            escape = false;
            continue;
        }
        match c {
            '\\' if !quoted => {
                cur.push(c);
                escape = true;
            }
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            '[' if !quoted => {
                bracket = true;
                cur.push(c);
            }
            ']' if !quoted => {
                bracket = false;
                cur.push(c);
            }
            ';' if !quoted && !bracket => out.push(String::new()),
            c => cur.push(c),
        }
    }
    out
}

fn parse_cond(s: &str) -> Option<Cond> {
    let (op, rest) = ["<=", ">=", "<>", "<", ">", "="].iter().find_map(|op| s.strip_prefix(op).map(|r| (*op, r)))?;
    let v: f64 = rest.trim().parse().ok()?;
    Some(match op {
        "<=" => Cond::Le(v),
        ">=" => Cond::Ge(v),
        "<>" => Cond::Ne(v),
        "<" => Cond::Lt(v),
        ">" => Cond::Gt(v),
        _ => Cond::Eq(v),
    })
}

fn push_lit(toks: &mut Vec<Tok>, s: &str) {
    if let Some(Tok::Lit(l)) = toks.last_mut() {
        l.push_str(s);
    } else {
        toks.push(Tok::Lit(s.to_string()));
    }
}

fn parse_section(code: &str) -> Section {
    let chars: Vec<char> = code.chars().collect();
    let mut toks = Vec::new();
    let (mut color, mut cond) = (None, None);
    let mut i = 0;
    let run = |i: usize, c: char| chars[i..].iter().take_while(|x| x.eq_ignore_ascii_case(&c)).count();
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                let end = chars[i + 1..].iter().position(|&x| x == '"').map_or(chars.len(), |p| i + 1 + p);
                push_lit(&mut toks, &chars[i + 1..end].iter().collect::<String>());
                i = end + 1;
            }
            '\\' => {
                if let Some(n) = chars.get(i + 1) {
                    push_lit(&mut toks, &n.to_string());
                }
                i += 2;
            }
            '_' => {
                push_lit(&mut toks, " ");
                i += 2;
            }
            '*' => i += 2,
            '[' => {
                let end = chars[i..].iter().position(|&x| x == ']').map_or(chars.len(), |p| i + p);
                let inner: String = chars[i + 1..end.min(chars.len())].iter().collect();
                let lower = inner.to_ascii_lowercase();
                if let Some(rest) = inner.strip_prefix('$') {
                    let symbol = rest.split('-').next().unwrap_or("");
                    push_lit(&mut toks, symbol);
                } else if !lower.is_empty() && lower.chars().all(|x| x == 'h') {
                    toks.push(Tok::Elapsed('h', lower.len() as u8));
                } else if !lower.is_empty() && lower.chars().all(|x| x == 'm') {
                    toks.push(Tok::Elapsed('m', lower.len() as u8));
                } else if !lower.is_empty() && lower.chars().all(|x| x == 's') {
                    toks.push(Tok::Elapsed('s', lower.len() as u8));
                } else if let Some(c) = color_name(&inner) {
                    color = Some(c);
                } else if let Some(c) = parse_cond(&inner) {
                    cond = Some(c);
                }
                i = end + 1;
            }
            '0' => {
                toks.push(Tok::Zero);
                i += 1;
            }
            '#' => {
                toks.push(Tok::Hash);
                i += 1;
            }
            '?' => {
                toks.push(Tok::Query);
                i += 1;
            }
            '.' => {
                let n = chars[i + 1..].iter().take_while(|&&x| x == '0').count();
                let after_second = toks.iter().rev().take_while(|t| !matches!(t, Tok::Hour(_) | Tok::Minute(_))).any(|t| matches!(t, Tok::Second(_) | Tok::Elapsed('s', _)));
                if n > 0 && after_second {
                    toks.push(Tok::SubSecond(n as u8));
                    i += 1 + n;
                } else {
                    toks.push(Tok::Dot);
                    i += 1;
                }
            }
            ',' => {
                toks.push(Tok::Comma);
                i += 1;
            }
            '%' => {
                toks.push(Tok::Percent);
                i += 1;
            }
            'E' | 'e' if matches!(chars.get(i + 1), Some('+' | '-')) => {
                toks.push(Tok::Exp(chars[i + 1] == '+'));
                i += 2;
            }
            '/' if toks.iter().any(|t| matches!(t, Tok::Zero | Tok::Hash | Tok::Query)) && !toks.iter().any(|t| matches!(t, Tok::Day(_) | Tok::Month(_) | Tok::Year(_))) => {
                toks.push(Tok::Slash);
                i += 1;
            }
            '@' => {
                toks.push(Tok::At);
                i += 1;
            }
            'y' | 'Y' => {
                let n = run(i, 'y');
                toks.push(Tok::Year(n as u8));
                i += n;
            }
            'd' | 'D' => {
                let n = run(i, 'd');
                toks.push(Tok::Day(n as u8));
                i += n;
            }
            'h' | 'H' => {
                let n = run(i, 'h');
                toks.push(Tok::Hour(n as u8));
                i += n;
            }
            's' | 'S' => {
                let n = run(i, 's');
                toks.push(Tok::Second(n as u8));
                i += n;
            }
            'm' | 'M' => {
                let n = run(i, 'm');
                toks.push(Tok::Month(n as u8));
                i += n;
            }
            'a' | 'A' if code[code.char_indices().nth(i).map_or(0, |(b, _)| b)..].to_ascii_uppercase().starts_with("AM/PM") => {
                toks.push(Tok::AmPm(true));
                i += 5;
            }
            'a' | 'A' if code[code.char_indices().nth(i).map_or(0, |(b, _)| b)..].to_ascii_uppercase().starts_with("A/P") => {
                toks.push(Tok::AmPm(false));
                i += 3;
            }
            'g' | 'G' if code[code.char_indices().nth(i).map_or(0, |(b, _)| b)..].to_ascii_lowercase().starts_with("general") => {
                toks.push(Tok::Lit("\u{0}".into()));
                i += 7;
            }
            c => {
                push_lit(&mut toks, &c.to_string());
                i += 1;
            }
        }
    }
    for k in 0..toks.len() {
        if let Tok::Month(n) = toks[k]
            && n <= 2
        {
            let prev = toks[..k].iter().rev().find(|t| !matches!(t, Tok::Lit(_)));
            let next = toks[k + 1..].iter().find(|t| !matches!(t, Tok::Lit(_)));
            if matches!(prev, Some(Tok::Hour(_) | Tok::Elapsed('h', _))) || matches!(next, Some(Tok::Second(_))) {
                toks[k] = Tok::Minute(n);
            }
        }
    }
    let date = toks.iter().any(|t| matches!(t, Tok::Year(_) | Tok::Month(_) | Tok::Minute(_) | Tok::Day(_) | Tok::Hour(_) | Tok::Second(_) | Tok::Elapsed(..)));
    let general = toks.iter().any(|t| matches!(t, Tok::Lit(l) if l.contains('\u{0}')));
    Section { toks, color, cond, date, general }
}

impl Format {
    pub fn parse(code: &str) -> Self {
        let code = code.trim();
        if code.is_empty() || code.eq_ignore_ascii_case("general") || code.eq_ignore_ascii_case("standard") {
            return Self { sections: vec![Section { toks: vec![Tok::Lit("\u{0}".into())], color: None, cond: None, date: false, general: true }] };
        }
        Self { sections: split_sections(code).iter().map(|s| parse_section(s)).collect() }
    }

    /// Whether the first section reads the value as a date or time.
    pub fn is_date(&self) -> bool {
        self.sections.first().is_some_and(|s| s.date)
    }

    /// Whether numbers show in General, which may drop digits to fit a column.
    pub fn is_general(&self) -> bool {
        self.sections.first().is_some_and(|s| s.general)
    }

    fn pick(&self, v: f64) -> (Option<&Section>, bool) {
        let numeric: Vec<&Section> = self.sections.iter().filter(|s| !s.toks.contains(&Tok::At)).collect();
        if numeric.iter().any(|s| s.cond.is_some()) {
            for s in &numeric {
                if s.cond.is_some_and(|c| c.holds(v)) {
                    return (Some(s), false);
                }
            }
            return (numeric.iter().find(|s| s.cond.is_none()).copied().or(numeric.last().copied()), false);
        }
        match (numeric.len(), v) {
            (0, _) => (None, false),
            (1, _) => (numeric.first().copied(), false),
            (_, v) if v > 0. => (numeric.first().copied(), false),
            (_, v) if v < 0. => (numeric.get(1).copied(), true),
            (n, _) if n >= 3 => (numeric.get(2).copied(), false),
            _ => (numeric.first().copied(), false),
        }
    }

    /// The text of a number in this format.
    pub fn number(&self, v: f64, date1904: bool, locale: Locale) -> Shown {
        let (section, strip_sign) = self.pick(v);
        let Some(section) = section else { return Shown { text: general(v, locale), color: None } };
        let v = if strip_sign { v.abs() } else { v };
        let text = if section.date { date(section, v, date1904, locale).unwrap_or_else(|| "#".repeat(8)) } else { number(section, v, locale) };
        Shown { text, color: section.color }
    }

    /// The text of a string cell: a text section can wrap it in literals.
    pub fn text(&self, s: &str) -> Shown {
        match self.sections.iter().find(|x| x.toks.contains(&Tok::At)) {
            Some(sec) => {
                let mut out = String::new();
                for t in &sec.toks {
                    match t {
                        Tok::At => out.push_str(s),
                        Tok::Lit(l) => out.push_str(l),
                        _ => {}
                    }
                }
                Shown { text: out, color: sec.color }
            }
            None => Shown { text: s.to_string(), color: None },
        }
    }
}

/// Excel's General format: up to eleven characters, scientific when it must.
pub fn general(v: f64, locale: Locale) -> String {
    if v == 0. {
        return "0".into();
    }
    if !v.is_finite() {
        return "#ZAHL!".into();
    }
    let a = v.abs();
    let s = if !(1e-9..1e11).contains(&a) {
        let s = format!("{:.5E}", v);
        let (m, e) = s.split_once('E').unwrap_or((&s, "0"));
        let m = m.trim_end_matches('0').trim_end_matches('.');
        let e: i32 = e.parse().unwrap_or(0);
        format!("{m}E{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    } else {
        let int_digits = if a >= 1. { a.log10().floor() as i32 + 1 } else { 1 };
        let decimals = (10 - int_digits).clamp(0, 10) as usize;
        let s = format!("{:.*}", decimals, v);
        if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
    };
    s.replace('.', &locale.decimal().to_string())
}

fn number(sec: &Section, v: f64, locale: Locale) -> String {
    let toks = &sec.toks;
    if sec.general {
        let mut out = String::new();
        for t in toks {
            if let Tok::Lit(l) = t {
                out.push_str(&l.replace('\u{0}', &general(v, locale)));
            }
        }
        return out;
    }
    let is_digit = |t: &Tok| matches!(t, Tok::Zero | Tok::Hash | Tok::Query);
    let first_digit = toks.iter().position(is_digit);
    let last_digit = toks.iter().rposition(is_digit);
    let Some((first, last)) = first_digit.zip(last_digit) else {
        let mut out = String::new();
        if v < 0. {
            out.push('-');
        }
        for t in toks {
            match t {
                Tok::Lit(l) => out.push_str(l),
                Tok::Percent => out.push('%'),
                _ => {}
            }
        }
        return out;
    };
    let percent = toks.iter().filter(|t| **t == Tok::Percent).count();
    let mut scale = 100f64.powi(percent as i32);
    let mut trailing = last + 1;
    while toks.get(trailing) == Some(&Tok::Comma) {
        scale /= 1000.;
        trailing += 1;
    }
    let exp = toks.iter().position(|t| matches!(t, Tok::Exp(_)));
    let slash = toks.iter().position(|t| *t == Tok::Slash);
    let dot = toks.iter().position(|t| *t == Tok::Dot).filter(|&d| exp.is_none_or(|e| d < e) && slash.is_none());
    let int_end = dot.or(exp).or(slash.and_then(|s| toks[..s].iter().rposition(|t| matches!(t, Tok::Lit(_))).filter(|&p| p > first))).unwrap_or(last + 1).max(first);
    let int_toks: Vec<&Tok> = toks[first..int_end].iter().filter(|t| is_digit(t)).collect();
    let grouped = toks[first..int_end].contains(&Tok::Comma);
    let min_int = int_toks.iter().filter(|t| ***t == Tok::Zero).count();
    let dec_toks: Vec<&Tok> = dot.map(|d| toks[d + 1..exp.unwrap_or(toks.len())].iter().filter(|t| is_digit(t)).collect()).unwrap_or_default();
    let decimals = dec_toks.len();
    let min_dec = dec_toks.iter().rposition(|t| **t == Tok::Zero).map_or(0, |p| p + 1);
    let x = v.abs() * scale;
    let (int_s, dec_s, exp_s) = if let Some(e) = exp {
        let exp_digits = toks[e + 1..].iter().filter(|t| is_digit(t)).count().max(1);
        let step = int_toks.len().max(1) as i32;
        let mut ex = if x == 0. { 0 } else { x.log10().floor() as i32 };
        if step > 1 {
            ex = ex.div_euclid(step) * step;
        }
        let m = x / 10f64.powi(ex);
        let s = format!("{:.*}", decimals, m);
        let (i, d) = s.split_once('.').unwrap_or((&s, ""));
        let sign = match (&toks[e], ex < 0) {
            (_, true) => "-",
            (Tok::Exp(true), false) => "+",
            _ => "",
        };
        (i.to_string(), d.to_string(), Some(format!("E{sign}{:0w$}", ex.abs(), w = exp_digits)))
    } else if let Some(s) = slash {
        let den_toks = toks[s + 1..].iter().take_while(|t| is_digit(t) || matches!(t, Tok::Lit(l) if l.chars().all(|c| c.is_ascii_digit()))).collect::<Vec<_>>();
        let fixed: Option<u64> = match den_toks.first() {
            Some(Tok::Lit(l)) => l.parse().ok(),
            _ => None,
        };
        let has_int = toks[first..s].iter().any(|t| matches!(t, Tok::Lit(l) if l.trim().is_empty() && !l.is_empty()));
        let whole = if has_int { x.trunc() } else { 0. };
        let frac = x - whole;
        let (num, den) = match fixed {
            Some(d) => ((frac * d as f64).round() as u64, d),
            None => best_fraction(frac, 10u64.pow(den_toks.len().clamp(1, 4) as u32) - 1),
        };
        let mut out = String::new();
        if v < 0. {
            out.push('-');
        }
        if has_int && (whole > 0. || num == 0) {
            out.push_str(&format!("{}", whole as u64));
        }
        if num > 0 {
            if has_int && whole > 0. {
                out.push(' ');
            }
            out.push_str(&format!("{num}/{den}"));
        }
        let mut pre = String::new();
        for t in &toks[..first] {
            if let Tok::Lit(l) = t {
                pre.push_str(l);
            }
        }
        return format!("{pre}{out}");
    } else {
        let s = format!("{:.*}", decimals, x);
        let (i, d) = s.split_once('.').unwrap_or((&s, ""));
        (i.to_string(), d.to_string(), None)
    };
    let int_s = int_s.trim_start_matches('0');
    let int_s = if int_s.len() < min_int { format!("{}{}", "0".repeat(min_int - int_s.len()), int_s) } else { int_s.to_string() };
    let int_s = if grouped { crate::core::i18n::group_digits(&int_s, locale.group()) } else { int_s };
    let mut dec_s = dec_s;
    while dec_s.len() > min_dec && dec_s.ends_with('0') {
        dec_s.pop();
    }
    let negative = v < 0. && (int_s.chars().chain(dec_s.chars()).any(|c| c.is_ascii_digit() && c != '0'));
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    let mut placed_int = false;
    for (k, t) in toks.iter().enumerate() {
        match t {
            Tok::Lit(l) => out.push_str(l),
            Tok::Percent => out.push('%'),
            Tok::Zero | Tok::Hash | Tok::Query if k < int_end => {
                if !placed_int {
                    out.push_str(&int_s);
                    placed_int = true;
                }
            }
            Tok::Dot if Some(k) == dot => {
                if !dec_s.is_empty() || dec_toks.iter().any(|t| **t == Tok::Query) {
                    out.push(locale.decimal());
                }
                out.push_str(&dec_s);
                let pad = dec_toks.len().saturating_sub(dec_s.len().max(min_dec));
                let queries = dec_toks.iter().skip(dec_s.len()).filter(|t| ***t == Tok::Query).count().min(pad);
                out.push_str(&" ".repeat(queries));
            }
            Tok::Exp(_) => {
                if let Some(e) = &exp_s {
                    out.push_str(e);
                }
            }
            _ => {}
        }
    }
    out
}

fn best_fraction(x: f64, max_den: u64) -> (u64, u64) {
    let mut best = (x.round() as u64, 1);
    let mut err = (x - best.0 as f64).abs();
    for d in 1..=max_den {
        let n = (x * d as f64).round() as u64;
        let e = (x - n as f64 / d as f64).abs();
        if e < err - 1e-12 {
            best = (n, d);
            err = e;
        }
    }
    best
}

/// Calendar parts of an Excel serial date.
pub fn civil(serial: f64, date1904: bool) -> Option<(i64, u32, u32, u32, u32, f64, u32)> {
    if !(serial.is_finite() && (-1.0..2_958_466.).contains(&serial)) {
        return None;
    }
    let mut days = serial.floor() as i64;
    let secs = (serial - serial.floor()) * 86400.;
    let secs = (secs * 1000.).round() / 1000.;
    let (mut s_total, mut frac) = (secs.floor() as i64, secs - secs.floor());
    if frac > 0.9995 {
        s_total += 1;
        frac = 0.;
    }
    if s_total >= 86400 {
        s_total -= 86400;
        days += 1;
    }
    if !date1904 && days == 60 {
        return Some((1900, 2, 29, (s_total / 3600) as u32, (s_total / 60 % 60) as u32, (s_total % 60) as f64 + frac, 2));
    }
    let base = if date1904 { days + 1462 } else if days < 60 { days + 1 } else { days };
    let (y, m, d) = from_days(base);
    let weekday = (base + 5).rem_euclid(7) as u32;
    Some((y, m, d, (s_total / 3600) as u32, (s_total / 60 % 60) as u32, (s_total % 60) as f64 + frac, weekday))
}

fn from_days(z_from_1899_12_30: i64) -> (i64, u32, u32) {
    let z = z_from_1899_12_30 - 25_569 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn date(sec: &Section, v: f64, date1904: bool, locale: Locale) -> Option<String> {
    let sub = sec.toks.iter().find_map(|t| if let Tok::SubSecond(n) = t { Some(*n) } else { None });
    let step = 86400. * 10f64.powi(sub.unwrap_or(0) as i32);
    let v = (v * step).round() / step;
    let (y, mo, d, h, mi, s, wd) = civil(v, date1904)?;
    let ampm = sec.toks.iter().any(|t| matches!(t, Tok::AmPm(_)));
    let (s_int, s_frac) = match sub {
        Some(n) => {
            let p = 10f64.powi(n as i32);
            let r = (s * p).round() / p;
            (r.floor() as u32, r - r.floor())
        }
        None => (s.round().min(59.) as u32, 0.),
    };
    let months = if locale.german { MONTHS_DE } else { MONTHS_EN };
    let days = if locale.german { DAYS_DE } else { DAYS_EN };
    let mut out = String::new();
    for t in &sec.toks {
        match *t {
            Tok::Lit(ref l) => out.push_str(l),
            Tok::Year(n) if n <= 2 => out.push_str(&format!("{:02}", y.rem_euclid(100))),
            Tok::Year(_) => out.push_str(&format!("{y:04}")),
            Tok::Month(1) => out.push_str(&mo.to_string()),
            Tok::Month(2) => out.push_str(&format!("{mo:02}")),
            Tok::Month(3) => out.push_str(&months[mo as usize - 1].chars().take(3).collect::<String>()),
            Tok::Month(5) => out.push_str(&months[mo as usize - 1].chars().take(1).collect::<String>()),
            Tok::Month(_) => out.push_str(months[mo as usize - 1]),
            Tok::Day(1) => out.push_str(&d.to_string()),
            Tok::Day(2) => out.push_str(&format!("{d:02}")),
            Tok::Day(3) => out.push_str(&days[wd as usize].chars().take(if locale.german { 2 } else { 3 }).collect::<String>()),
            Tok::Day(_) => out.push_str(days[wd as usize]),
            Tok::Hour(n) => {
                let hh = if ampm { (h + 11) % 12 + 1 } else { h };
                out.push_str(&if n >= 2 { format!("{hh:02}") } else { hh.to_string() });
            }
            Tok::Minute(n) => out.push_str(&if n >= 2 { format!("{mi:02}") } else { mi.to_string() }),
            Tok::Second(n) => out.push_str(&if n >= 2 { format!("{s_int:02}") } else { s_int.to_string() }),
            Tok::SubSecond(n) => {
                out.push(locale.decimal());
                out.push_str(&format!("{:.*}", n as usize, s_frac)[2..]);
            }
            Tok::Elapsed(unit, n) => {
                let total = (v * 86_400_000.).round() / 1000.;
                let x = match unit {
                    'h' => (total / 3600.).floor(),
                    'm' => (total / 60.).floor(),
                    _ => total.round(),
                } as i64;
                out.push_str(&format!("{:0w$}", x, w = n as usize));
            }
            Tok::AmPm(full) => {
                let pm = h >= 12;
                out.push_str(match (full, pm) {
                    (true, true) => "PM",
                    (true, false) => "AM",
                    (false, true) => "P",
                    (false, false) => "A",
                });
            }
            Tok::Percent => out.push('%'),
            Tok::Dot => out.push('.'),
            Tok::Comma => out.push(','),
            Tok::Slash => out.push('/'),
            Tok::Zero => out.push('0'),
            _ => {}
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DE: Locale = Locale { german: true };
    const EN: Locale = Locale { german: false };

    fn f(code: &str, v: f64, l: Locale) -> String {
        Format::parse(code).number(v, false, l).text
    }

    #[test]
    fn formats_numbers_like_excel() {
        assert_eq!(f("General", 1234.5, DE), "1234,5");
        assert_eq!(f("General", 0.1 + 0.2, EN), "0.3");
        assert_eq!(f("General", 123456789012., EN), "1.23457E+11");
        assert_eq!(f("0.00", 1.23456, DE), "1,23");
        assert_eq!(f(".00", 0.5, EN), ".50");
        assert!(!f("E+00", 5., EN).is_empty());
        assert_eq!(f("#,##0.00", 1234567.891, DE), "1.234.567,89");
        assert_eq!(f("#,##0.00", 1234567.891, EN), "1,234,567.89");
        assert_eq!(f("#,##0.00 €", -1234.5, DE), "-1.234,50 €");
        assert_eq!(f("0%", 0.256, DE), "26%");
        assert_eq!(f("0.0%", 0.256, EN), "25.6%");
        assert_eq!(f("0.00E+00", 12345., EN), "1.23E+04");
        assert_eq!(f("#,##0;[Red]-#,##0", -5., EN), "-5");
        assert_eq!(Format::parse("#,##0;[Red]-#,##0").number(-5., false, EN).color, Some(0xff0000));
        assert_eq!(f("0.00;(0.00);\"null\"", 0., EN), "null");
        assert_eq!(f("0.00;(0.00)", -2., EN), "(2.00)");
        assert_eq!(f("#,##0,", 1234567., EN), "1,235");
        assert_eq!(f("0.0#", 2.5, EN), "2.5");
        assert_eq!(f("0.0#", 2.567, EN), "2.57");
        assert_eq!(f("[$€-407] #,##0", 9., DE), "€ 9");
        assert_eq!(f("# ?/?", 1.5, EN), "1 1/2");
        assert_eq!(f("[>=100]\"viel\";\"wenig\"", 150., EN), "viel");
        assert_eq!(Format::parse("@\" Stk\"").text("3").text, "3 Stk");
    }

    #[test]
    fn formats_dates_and_times() {
        assert_eq!(f("dd.mm.yyyy", 45943.5, DE), "13.10.2025");
        assert_eq!(f("yyyy-mm-dd hh:mm", 45943.5, DE), "2025-10-13 12:00");
        assert_eq!(f("d. mmmm yyyy", 45943., DE), "13. Oktober 2025");
        assert_eq!(f("dddd", 45943., EN), "Monday");
        assert_eq!(f("[h]:mm:ss", 1.9999999999999998, EN), "48:00:00");
        assert_eq!(f("hh:mm:ss", 0.5 + 59.6 / 86400., EN), "12:01:00");
        assert_eq!(f("hh:mm:ss.0", 0.5 + 59.96 / 86400., EN), "12:01:00.0");
        assert_eq!(f("ddd", 45943., DE), "Mo");
        assert_eq!(f("h:mm AM/PM", 0.75, EN), "6:00 PM");
        assert_eq!(f("[h]:mm:ss", 1.5, EN), "36:00:00");
        assert_eq!(f("mm:ss.0", 1. / 86400. * 61.25, DE), "01:01,3");
        assert_eq!(f("m/d/yyyy", 1., EN), "1/1/1900");
        assert_eq!(f("m/d/yyyy", 61., EN), "3/1/1900");
        assert_eq!(f("dd.mm.yyyy", 60., DE), "29.02.1900");
        assert_eq!(Format::parse("dd.mm.yyyy").number(0., true, DE).text, "01.01.1904");
        assert!(Format::parse("dd.mm.yyyy").is_date() && !Format::parse("0.00").is_date());
        assert_eq!(f(builtin(14, DE).unwrap(), 45943., DE), "13.10.2025");
    }
}
