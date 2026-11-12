use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};
use serde_yaml_ng::Value as Yaml;

use super::frontmatter::{self, PropertyKind};
use super::index::VaultIndex;
use super::moment;

/// A value in a formula or a table cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Date(DateTime<Local>),
    Duration(i64),
    List(Vec<Value>),
    Link(String, Option<String>),
    File(PathBuf),
    Regex(String),
}

impl Value {
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.,
            Value::Text(s) | Value::Regex(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            _ => true,
        }
    }

    /// As shown in a cell.
    pub fn display(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => format_number(*n),
            Value::Text(s) | Value::Regex(s) => s.clone(),
            Value::Date(d) => {
                if d.format("%H:%M:%S").to_string() == "00:00:00" { d.format("%Y-%m-%d").to_string() } else { d.format("%Y-%m-%d %H:%M").to_string() }
            }
            Value::Duration(ms) => format!("{}s", ms / 1000),
            Value::List(l) => l.iter().map(Value::display).collect::<Vec<_>>().join(", "),
            Value::Link(path, shown) => shown.clone().unwrap_or_else(|| path.clone()),
            Value::File(p) => p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            Value::Text(s) => s.trim().parse().ok(),
            Value::Bool(b) => Some(if *b { 1. } else { 0. }),
            _ => None,
        }
    }

    fn compare(&self, other: &Value) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => a.partial_cmp(b),
            (Value::Date(a), Value::Date(b)) => a.partial_cmp(b),
            (Value::Date(a), Value::Text(b)) => parse_date(b).and_then(|b| a.partial_cmp(&b)),
            (Value::Text(a), Value::Date(b)) => parse_date(a).and_then(|a| a.partial_cmp(b)),
            (Value::Duration(a), Value::Duration(b)) => a.partial_cmp(b),
            (Value::Bool(a), Value::Bool(b)) => a.partial_cmp(b),
            (Value::Null, Value::Null) => Some(std::cmp::Ordering::Equal),
            (Value::Null, _) | (_, Value::Null) => None,
            (a, b) => match (a.number(), b.number()) {
                (Some(x), Some(y)) if matches!(a, Value::Number(_)) || matches!(b, Value::Number(_)) => x.partial_cmp(&y),
                _ => Some(a.display().to_lowercase().cmp(&b.display().to_lowercase())),
            },
        }
    }

    fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Link(a, _), Value::Link(b, _)) => a.eq_ignore_ascii_case(b),
            (Value::Null, Value::Text(t)) | (Value::Text(t), Value::Null) => t.is_empty(),
            _ => self.compare(other) == Some(std::cmp::Ordering::Equal),
        }
    }
}

fn format_number(n: f64) -> String {
    if n.fract() == 0. && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{}", (n * 1e6).round() / 1e6) }
}

fn parse_date(s: &str) -> Option<DateTime<Local>> {
    let s = s.trim();
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Local.from_local_datetime(&d.and_hms_opt(0, 0, 0)?).earliest();
    }
    for pattern in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(d) = chrono::NaiveDateTime::parse_from_str(s, pattern) {
            return Local.from_local_datetime(&d).earliest();
        }
    }
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Local))
}

/// Reads `1h`, `2 days`, `-3M` as milliseconds; months count as 30 days and years as 365.
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, s) = s.strip_prefix('-').map_or((1, s), |r| (-1, r));
    let s = s.strip_prefix('+').unwrap_or(s).trim();
    let split = s.find(|c: char| !c.is_ascii_digit() && c != '.')?;
    let amount: f64 = s[..split].parse().ok()?;
    let unit = s[split..].trim();
    let factor: i64 = match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => 1000,
        "m" | "min" | "mins" | "minute" | "minutes" => 60_000,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3_600_000,
        "d" | "day" | "days" => 86_400_000,
        "w" | "week" | "weeks" => 7 * 86_400_000,
        "M" | "month" | "months" => 30 * 86_400_000,
        "y" | "year" | "years" => 365 * 86_400_000,
        _ => return None,
    };
    Some(sign * (amount * factor as f64) as i64)
}

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value),
    Name(String),
    Member(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Unary(char, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    ListOf(Vec<Expr>),
}

struct Parser<'a> {
    chars: Vec<char>,
    at: usize,
    source: &'a str,
}

impl Parser<'_> {
    fn spaces(&mut self) {
        while self.chars.get(self.at).is_some_and(|c| c.is_whitespace()) {
            self.at += 1;
        }
    }

    fn eat(&mut self, token: &str) -> bool {
        self.spaces();
        let t: Vec<char> = token.chars().collect();
        if self.chars[self.at..].starts_with(&t) {
            self.at += t.len();
            true
        } else {
            false
        }
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        while self.eat("||") {
            left = Expr::Binary("||".into(), left.into(), self.and()?.into());
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.compare()?;
        while self.eat("&&") {
            left = Expr::Binary("&&".into(), left.into(), self.compare()?.into());
        }
        Ok(left)
    }

    fn compare(&mut self) -> Result<Expr, String> {
        let left = self.sum()?;
        for op in ["==", "!=", ">=", "<=", ">", "<"] {
            if self.eat(op) {
                return Ok(Expr::Binary(op.into(), left.into(), self.sum()?.into()));
            }
        }
        Ok(left)
    }

    fn sum(&mut self) -> Result<Expr, String> {
        let mut left = self.product()?;
        loop {
            if self.eat("+") {
                left = Expr::Binary("+".into(), left.into(), self.product()?.into());
            } else if self.eat("-") {
                left = Expr::Binary("-".into(), left.into(), self.product()?.into());
            } else {
                return Ok(left);
            }
        }
    }

    fn product(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        loop {
            self.spaces();
            let op = match self.chars.get(self.at) {
                Some('*') => "*",
                Some('/') => "/",
                Some('%') => "%",
                _ => return Ok(left),
            };
            self.at += 1;
            left = Expr::Binary(op.into(), left.into(), self.unary()?.into());
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        self.spaces();
        if self.chars.get(self.at) == Some(&'!') && self.chars.get(self.at + 1) != Some(&'=') {
            self.at += 1;
            return Ok(Expr::Unary('!', self.unary()?.into()));
        }
        if self.chars.get(self.at) == Some(&'-') {
            self.at += 1;
            return Ok(Expr::Unary('-', self.unary()?.into()));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.primary()?;
        loop {
            self.spaces();
            match self.chars.get(self.at) {
                Some('.') => {
                    self.at += 1;
                    let name = self.word().ok_or_else(|| self.error("Name erwartet"))?;
                    e = Expr::Member(e.into(), name);
                }
                Some('(') => {
                    self.at += 1;
                    let args = self.arguments(')')?;
                    e = Expr::Call(e.into(), args);
                }
                Some('[') => {
                    self.at += 1;
                    let inner = self.or()?;
                    if !self.eat("]") {
                        return Err(self.error("] fehlt"));
                    }
                    e = Expr::Index(e.into(), inner.into());
                }
                _ => return Ok(e),
            }
        }
    }

    fn arguments(&mut self, close: char) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        self.spaces();
        if self.chars.get(self.at) == Some(&close) {
            self.at += 1;
            return Ok(args);
        }
        loop {
            args.push(self.or()?);
            self.spaces();
            match self.chars.get(self.at) {
                Some(',') => self.at += 1,
                Some(c) if *c == close => {
                    self.at += 1;
                    return Ok(args);
                }
                _ => return Err(self.error("Klammer fehlt")),
            }
        }
    }

    fn word(&mut self) -> Option<String> {
        self.spaces();
        let start = self.at;
        while self.chars.get(self.at).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
            self.at += 1;
        }
        (self.at > start).then(|| self.chars[start..self.at].iter().collect())
    }

    fn error(&self, what: &str) -> String {
        format!("{what}: {}", self.source)
    }

    fn primary(&mut self) -> Result<Expr, String> {
        self.spaces();
        match self.chars.get(self.at).copied() {
            Some('(') => {
                self.at += 1;
                let e = self.or()?;
                if !self.eat(")") {
                    return Err(self.error(") fehlt"));
                }
                Ok(e)
            }
            Some('[') => {
                self.at += 1;
                Ok(Expr::ListOf(self.arguments(']')?))
            }
            Some(q @ ('"' | '\'')) => {
                self.at += 1;
                let mut out = String::new();
                while let Some(c) = self.chars.get(self.at).copied() {
                    self.at += 1;
                    if c == q {
                        return Ok(Expr::Literal(Value::Text(out)));
                    }
                    if c == '\\'
                        && let Some(next) = self.chars.get(self.at).copied()
                    {
                        self.at += 1;
                        out.push(next);
                    } else {
                        out.push(c);
                    }
                }
                Err(self.error("Anführungszeichen fehlt"))
            }
            Some('/') => {
                self.at += 1;
                let mut out = String::new();
                while let Some(c) = self.chars.get(self.at).copied() {
                    self.at += 1;
                    if c == '/' {
                        while self.chars.get(self.at).is_some_and(|c| c.is_alphabetic()) {
                            self.at += 1;
                        }
                        return Ok(Expr::Literal(Value::Regex(out)));
                    }
                    out.push(c);
                }
                Err(self.error("/ fehlt"))
            }
            Some(c) if c.is_ascii_digit() => {
                let start = self.at;
                while self.chars.get(self.at).is_some_and(|c| c.is_ascii_digit() || (*c == '.' && self.chars.get(self.at + 1).is_some_and(char::is_ascii_digit))) {
                    self.at += 1;
                }
                let text: String = self.chars[start..self.at].iter().collect();
                text.parse().map(|n| Expr::Literal(Value::Number(n))).map_err(|_| self.error("Zahl"))
            }
            _ => match self.word().as_deref() {
                Some("true") => Ok(Expr::Literal(Value::Bool(true))),
                Some("false") => Ok(Expr::Literal(Value::Bool(false))),
                Some("null") => Ok(Expr::Literal(Value::Null)),
                Some(name) => Ok(Expr::Name(name.to_string())),
                None => Err(self.error("Ausdruck erwartet")),
            },
        }
    }
}

/// Parses a formula or filter statement.
pub fn parse(source: &str) -> Result<Expr, String> {
    let mut p = Parser { chars: source.chars().collect(), at: 0, source };
    let e = p.or()?;
    p.spaces();
    if p.at < p.chars.len() { Err(p.error("Unerwartetes Zeichen")) } else { Ok(e) }
}

/// The note a row stands for, and what formulas may reach.
pub struct Row<'a> {
    pub path: &'a Path,
    pub index: &'a VaultIndex,
    /// The file named by `this`.
    pub this: Option<&'a Path>,
    pub formulas: &'a HashMap<String, Expr>,
    pub depth: std::cell::Cell<u32>,
    pub now: DateTime<Local>,
}

fn value_from_property(values: &[String], kind: PropertyKind) -> Value {
    let one = |s: &String| -> Value {
        if let Some(inner) = s.strip_prefix("[[").and_then(|x| x.strip_suffix("]]")) {
            let (path, shown) = inner.split_once('|').map_or((inner, None), |(p, a)| (p, Some(a.to_string())));
            return Value::Link(path.to_string(), shown);
        }
        match kind {
            PropertyKind::Number => s.parse().map_or(Value::Text(s.clone()), Value::Number),
            PropertyKind::Checkbox => Value::Bool(s == "true"),
            PropertyKind::Date | PropertyKind::DateTime => parse_date(s).map_or(Value::Text(s.clone()), Value::Date),
            _ => Value::Text(s.clone()),
        }
    };
    match kind {
        PropertyKind::List | PropertyKind::Tags => Value::List(values.iter().map(one).collect()),
        _ => values.first().map_or(Value::Null, one),
    }
}

fn file_field(path: &Path, field: &str, index: &VaultIndex) -> Value {
    let meta = std::fs::metadata(path).ok();
    let time = |t: Option<std::time::SystemTime>| t.map_or(Value::Null, |t| Value::Date(DateTime::<Local>::from(t)));
    let facts = index.notes.get(path);
    match field {
        "name" => Value::Text(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
        "basename" => Value::Text(path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
        "ext" => Value::Text(path.extension().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
        "path" => Value::Text(path.strip_prefix(&index.root).unwrap_or(path).to_string_lossy().replace('\\', "/")),
        "folder" => Value::Text(path.parent().and_then(|p| p.strip_prefix(&index.root).ok()).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default()),
        "size" => Value::Number(meta.map_or(0., |m| m.len() as f64)),
        "mtime" => time(meta.and_then(|m| m.modified().ok())),
        "ctime" => time(meta.and_then(|m| m.created().ok())),
        "tags" => Value::List(facts.map_or(Vec::new(), |f| f.tags.iter().map(|t| Value::Text(t.clone())).collect())),
        "aliases" => Value::List(facts.map_or(Vec::new(), |f| f.aliases.iter().map(|t| Value::Text(t.clone())).collect())),
        "links" => Value::List(facts.map_or(Vec::new(), |f| f.links.iter().map(|(l, _, _)| Value::Link(l.target.path.clone(), None)).collect())),
        "backlinks" => Value::List(index.backlinks.get(path).map_or(Vec::new(), |l| l.iter().map(|m| Value::File(m.source.clone())).collect())),
        "properties" => Value::List(facts.map_or(Vec::new(), |f| f.properties.iter().map(|p| Value::Text(p.name.clone())).collect())),
        _ => Value::Null,
    }
}

fn note_property(path: &Path, name: &str, index: &VaultIndex) -> Value {
    index.notes.get(path).and_then(|f| f.properties.iter().find(|p| p.name == name)).map_or(Value::Null, |p| value_from_property(&p.values, p.kind))
}

impl Row<'_> {
    pub fn eval(&self, e: &Expr) -> Result<Value, String> {
        match e {
            Expr::Literal(v) => Ok(v.clone()),
            Expr::Name(name) => Ok(match name.as_str() {
                "file" => Value::File(self.path.to_path_buf()),
                "this" => self.this.map_or(Value::Null, |p| Value::File(p.to_path_buf())),
                "note" | "formula" => Value::Text(format!("\u{1}{name}")),
                other => note_property(self.path, other, self.index),
            }),
            Expr::Member(object, field) => {
                if let Expr::Name(root) = object.as_ref() {
                    match root.as_str() {
                        "note" => return Ok(note_property(self.path, field, self.index)),
                        "formula" => return self.formula(field),
                        _ => {}
                    }
                }
                let base = self.eval(object)?;
                Ok(match base {
                    Value::File(p) => {
                        let v = file_field(&p, field, self.index);
                        if v == Value::Null && !matches!(field.as_str(), "mtime" | "ctime") { note_property(&p, field, self.index) } else { v }
                    }
                    Value::Text(s) if field == "length" => Value::Number(s.chars().count() as f64),
                    Value::List(l) if field == "length" => Value::Number(l.len() as f64),
                    Value::Date(d) => match field.as_str() {
                        "year" => Value::Number(d.year() as f64),
                        "month" => Value::Number(d.month() as f64),
                        "day" => Value::Number(d.day() as f64),
                        "hour" => Value::Number(chrono::Timelike::hour(&d) as f64),
                        "minute" => Value::Number(chrono::Timelike::minute(&d) as f64),
                        "second" => Value::Number(chrono::Timelike::second(&d) as f64),
                        _ => Value::Null,
                    },
                    _ => Value::Null,
                })
            }
            Expr::Index(object, index) => {
                let (base, at) = (self.eval(object)?, self.eval(index)?);
                Ok(match (&base, &at) {
                    (Value::List(l), Value::Number(n)) => l.get(*n as usize).cloned().unwrap_or(Value::Null),
                    (Value::Text(t), Value::Text(name)) if t == "\u{1}note" => note_property(self.path, name, self.index),
                    (Value::Text(t), Value::Text(name)) if t == "\u{1}formula" => self.formula(name)?,
                    (Value::File(p), Value::Text(name)) => note_property(p, name, self.index),
                    _ => Value::Null,
                })
            }
            Expr::ListOf(items) => Ok(Value::List(items.iter().map(|i| self.eval(i)).collect::<Result<_, _>>()?)),
            Expr::Unary('!', inner) => Ok(Value::Bool(!self.eval(inner)?.truthy())),
            Expr::Unary(_, inner) => Ok(self.eval(inner)?.number().map_or(Value::Null, |n| Value::Number(-n))),
            Expr::Binary(op, a, b) => self.binary(op, a, b),
            Expr::Call(callee, args) => self.call(callee, args),
        }
    }

    fn formula(&self, name: &str) -> Result<Value, String> {
        if self.depth.get() > 32 {
            return Err("Formel verweist auf sich selbst".to_string());
        }
        let Some(e) = self.formulas.get(name) else { return Ok(Value::Null) };
        self.depth.set(self.depth.get() + 1);
        let out = self.eval(e);
        self.depth.set(self.depth.get() - 1);
        out
    }

    fn binary(&self, op: &str, a: &Expr, b: &Expr) -> Result<Value, String> {
        if op == "&&" {
            return Ok(Value::Bool(self.eval(a)?.truthy() && self.eval(b)?.truthy()));
        }
        if op == "||" {
            let left = self.eval(a)?;
            return if left.truthy() { Ok(left) } else { self.eval(b) };
        }
        let (x, y) = (self.eval(a)?, self.eval(b)?);
        use std::cmp::Ordering::*;
        Ok(match op {
            "==" => Value::Bool(x.equals(&y)),
            "!=" => Value::Bool(!x.equals(&y)),
            ">" => Value::Bool(x.compare(&y) == Some(Greater)),
            "<" => Value::Bool(x.compare(&y) == Some(Less)),
            ">=" => Value::Bool(matches!(x.compare(&y), Some(Greater | Equal))),
            "<=" => Value::Bool(matches!(x.compare(&y), Some(Less | Equal))),
            _ => arithmetic(op, x, y),
        })
    }

    fn call(&self, callee: &Expr, args: &[Expr]) -> Result<Value, String> {
        if let Expr::Member(object, method) = callee {
            if let (Value::List(list), "filter" | "map" | "reduce") = (&self.peek(object)?, method.as_str()) {
                return self.list_lambda(list, method, args);
            }
            let base = self.eval(object)?;
            let values: Vec<Value> = args.iter().map(|a| self.eval(a)).collect::<Result<_, _>>()?;
            return method_call(&base, method, &values, self);
        }
        let Expr::Name(name) = callee else { return Err("Das lässt sich nicht aufrufen".into()) };
        if name == "if" {
            let condition = args.first().map(|c| self.eval(c)).transpose()?.is_some_and(|v| v.truthy());
            return match (condition, args.get(1), args.get(2)) {
                (true, Some(e), _) | (false, _, Some(e)) => self.eval(e),
                _ => Ok(Value::Null),
            };
        }
        let values: Vec<Value> = args.iter().map(|a| self.eval(a)).collect::<Result<_, _>>()?;
        global_call(name, &values, self)
    }

    fn peek(&self, e: &Expr) -> Result<Value, String> {
        self.eval(e)
    }

    /// `value`, `index` and `acc` inside list functions.
    fn list_lambda(&self, list: &[Value], method: &str, args: &[Expr]) -> Result<Value, String> {
        let Some(body) = args.first() else { return Err("Argument fehlt".into()) };
        let bind = |e: &Expr, value: &Value, index: usize, acc: Option<&Value>| substitute(e, value, index, acc);
        Ok(match method {
            "filter" => {
                let mut out = Vec::new();
                for (i, v) in list.iter().enumerate() {
                    if self.eval(&bind(body, v, i, None))?.truthy() {
                        out.push(v.clone());
                    }
                }
                Value::List(out)
            }
            "map" => Value::List(list.iter().enumerate().map(|(i, v)| self.eval(&bind(body, v, i, None))).collect::<Result<_, _>>()?),
            _ => {
                let mut acc = args.get(1).map(|a| self.eval(a)).transpose()?.unwrap_or(Value::Null);
                for (i, v) in list.iter().enumerate() {
                    acc = self.eval(&bind(body, v, i, Some(&acc)))?;
                }
                acc
            }
        })
    }
}

fn substitute(e: &Expr, value: &Value, index: usize, acc: Option<&Value>) -> Expr {
    let go = |x: &Expr| substitute(x, value, index, acc);
    match e {
        Expr::Name(n) if n == "value" => Expr::Literal(value.clone()),
        Expr::Name(n) if n == "index" => Expr::Literal(Value::Number(index as f64)),
        Expr::Name(n) if n == "acc" && acc.is_some() => Expr::Literal(acc.cloned().unwrap_or(Value::Null)),
        Expr::Member(a, b) => Expr::Member(go(a).into(), b.clone()),
        Expr::Index(a, b) => Expr::Index(go(a).into(), go(b).into()),
        Expr::Call(a, args) => Expr::Call(go(a).into(), args.iter().map(go).collect()),
        Expr::Unary(c, a) => Expr::Unary(*c, go(a).into()),
        Expr::Binary(op, a, b) => Expr::Binary(op.clone(), go(a).into(), go(b).into()),
        Expr::ListOf(items) => Expr::ListOf(items.iter().map(go).collect()),
        other => other.clone(),
    }
}

fn arithmetic(op: &str, x: Value, y: Value) -> Value {
    let duration = |v: &Value| match v {
        Value::Duration(d) => Some(*d),
        Value::Text(s) => parse_duration(s),
        _ => None,
    };
    if let Value::Date(d) = &x {
        if let Value::Date(other) = &y
            && op == "-"
        {
            return Value::Duration((*d - *other).num_milliseconds());
        }
        if let Some(ms) = duration(&y) {
            let shift = Duration::milliseconds(if op == "-" { -ms } else { ms });
            return Value::Date(*d + shift);
        }
    }
    if let (Value::Duration(a), Some(b)) = (&x, duration(&y)) {
        return Value::Duration(if op == "-" { a - b } else { a + b });
    }
    if op == "+" && (matches!(x, Value::Text(_)) || matches!(y, Value::Text(_))) && (x.number().is_none() || y.number().is_none()) {
        return Value::Text(x.display() + &y.display());
    }
    let (Some(a), Some(b)) = (x.number(), y.number()) else { return Value::Null };
    Value::Number(match op {
        "+" => a + b,
        "-" => a - b,
        "*" => a * b,
        "/" if b != 0. => a / b,
        "%" if b != 0. => a % b,
        _ => return Value::Null,
    })
}

fn global_call(name: &str, args: &[Value], row: &Row) -> Result<Value, String> {
    let first = args.first();
    let text = |v: Option<&Value>| v.map(Value::display).unwrap_or_default();
    Ok(match name {
        "today" => row.now.date_naive().and_hms_opt(0, 0, 0).and_then(|n| Local.from_local_datetime(&n).earliest()).map_or(Value::Null, Value::Date),
        "now" => Value::Date(row.now),
        "date" => match first {
            Some(Value::Date(d)) => Value::Date(*d),
            other => parse_date(&text(other)).map_or(Value::Null, Value::Date),
        },
        "duration" => parse_duration(&text(first)).map_or(Value::Null, Value::Duration),
        "number" => first.and_then(Value::number).map_or(Value::Null, Value::Number),
        "list" => match first {
            Some(Value::List(l)) => Value::List(l.clone()),
            Some(v) => Value::List(vec![v.clone()]),
            None => Value::List(Vec::new()),
        },
        "link" => Value::Link(match first {
            Some(Value::File(p)) => p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            other => text(other),
        }, args.get(1).map(Value::display)),
        "file" => row.index.resolve(&text(first), Some(row.path)).map_or(Value::Null, Value::File),
        "image" | "icon" | "html" => first.cloned().unwrap_or(Value::Null),
        "escapeHTML" => Value::Text(crate::core::markdown::escape(&text(first))),
        "max" => args.iter().filter_map(Value::number).reduce(f64::max).map_or(Value::Null, Value::Number),
        "min" => args.iter().filter_map(Value::number).reduce(f64::min).map_or(Value::Null, Value::Number),
        "random" => Value::Number(row.now.timestamp_subsec_nanos() as f64 / 1e9),
        other => return Err(format!("Unbekannte Funktion: {other}")),
    })
}

fn contains(list_or_text: &Value, needle: &Value) -> bool {
    match list_or_text {
        Value::List(l) => l.iter().any(|v| v.equals(needle)),
        other => other.display().to_lowercase().contains(&needle.display().to_lowercase()),
    }
}

fn method_call(base: &Value, method: &str, args: &[Value], row: &Row) -> Result<Value, String> {
    let arg = args.first();
    let arg_text = || arg.map(Value::display).unwrap_or_default();
    let int = |i: usize| args.get(i).and_then(Value::number).map(|n| n as i64);
    match method {
        "isTruthy" => return Ok(Value::Bool(base.truthy())),
        "toString" => return Ok(Value::Text(base.display())),
        "isEmpty" => return Ok(Value::Bool(!base.truthy() && !matches!(base, Value::Number(_) | Value::Bool(_) | Value::Date(_)))),
        "isType" => {
            let kind = match base {
                Value::Null => "null",
                Value::Bool(_) => "boolean",
                Value::Number(_) => "number",
                Value::Text(_) => "string",
                Value::Date(_) => "date",
                Value::Duration(_) => "duration",
                Value::List(_) => "list",
                Value::Link(..) => "link",
                Value::File(_) => "file",
                Value::Regex(_) => "regexp",
            };
            return Ok(Value::Bool(kind == arg_text()));
        }
        _ => {}
    }
    Ok(match base {
        Value::Regex(pattern) if method == "matches" => Value::Bool(regex::Regex::new(pattern).is_ok_and(|r| r.is_match(&arg_text()))),
        Value::File(path) => match method {
            "asLink" => Value::Link(path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), arg.map(Value::display)),
            "hasTag" => {
                let tags = row.index.notes.get(path).map(|f| f.tags.clone()).unwrap_or_default();
                Value::Bool(args.iter().any(|want| {
                    let want = want.display().trim_start_matches('#').to_lowercase();
                    tags.iter().any(|t| {
                        let t = t.to_lowercase();
                        t == want || t.starts_with(&format!("{want}/"))
                    })
                }))
            }
            "hasProperty" => Value::Bool(row.index.notes.get(path).is_some_and(|f| f.properties.iter().any(|p| p.name == arg_text()))),
            "inFolder" => {
                let folder = arg_text();
                let rel = path.strip_prefix(&row.index.root).unwrap_or(path).to_string_lossy().replace('\\', "/");
                Value::Bool(rel.starts_with(&format!("{}/", folder.trim_end_matches('/'))))
            }
            "hasLink" => {
                let target = match arg {
                    Some(Value::File(p)) => Some(p.clone()),
                    other => row.index.resolve(&other.map(Value::display).unwrap_or_default(), Some(path)),
                };
                Value::Bool(target.is_some_and(|t| row.index.notes.get(path).is_some_and(|f| f.links.iter().any(|(l, _, _)| row.index.resolver.resolve(&l.target, Some(path)).as_ref() == Some(&t)))))
            }
            _ => return Err(format!("Unbekannte Funktion: {method}")),
        },
        Value::Link(link, shown) => match method {
            "asFile" => row.index.resolve(link, Some(row.path)).map_or(Value::Null, Value::File),
            "linksTo" => {
                let target = match arg {
                    Some(Value::File(p)) => Some(p.clone()),
                    other => row.index.resolve(&other.map(Value::display).unwrap_or_default(), Some(row.path)),
                };
                Value::Bool(target == row.index.resolve(link, Some(row.path)))
            }
            _ => return method_call(&Value::Text(shown.clone().unwrap_or_else(|| link.clone())), method, args, row),
        },
        Value::Date(d) => match method {
            "date" => d.date_naive().and_hms_opt(0, 0, 0).and_then(|n| Local.from_local_datetime(&n).earliest()).map_or(Value::Null, Value::Date),
            "time" => Value::Text(d.format("%H:%M:%S").to_string()),
            "format" => Value::Text(moment::format(&arg_text(), *d, crate::core::i18n::is_english())),
            "relative" => Value::Text(relative_time((row.now - *d).num_seconds())),
            _ => return Err(format!("Unbekannte Funktion: {method}")),
        },
        Value::Number(n) => match method {
            "abs" => Value::Number(n.abs()),
            "ceil" => Value::Number(n.ceil()),
            "floor" => Value::Number(n.floor()),
            "round" => {
                let factor = 10f64.powi(int(0).unwrap_or(0) as i32);
                Value::Number((n * factor).round() / factor)
            }
            "toFixed" => Value::Text(format!("{:.*}", int(0).unwrap_or(0).max(0) as usize, n)),
            _ => return Err(format!("Unbekannte Funktion: {method}")),
        },
        Value::List(list) => match method {
            "contains" => Value::Bool(arg.is_some_and(|a| list.iter().any(|v| v.equals(a)))),
            "containsAll" => Value::Bool(args.iter().all(|a| list.iter().any(|v| v.equals(a)))),
            "containsAny" => Value::Bool(args.iter().any(|a| list.iter().any(|v| v.equals(a)))),
            "flat" => Value::List(list.iter().flat_map(|v| if let Value::List(i) = v { i.clone() } else { vec![v.clone()] }).collect()),
            "join" => Value::Text(list.iter().map(Value::display).collect::<Vec<_>>().join(&arg_text())),
            "reverse" => Value::List(list.iter().rev().cloned().collect()),
            "slice" => {
                let (start, end) = slice_range(list.len(), int(0), int(1));
                Value::List(list[start..end].to_vec())
            }
            "sort" => {
                let mut l = list.clone();
                l.sort_by(|a, b| a.compare(b).unwrap_or(std::cmp::Ordering::Equal));
                Value::List(l)
            }
            "unique" => {
                let mut out: Vec<Value> = Vec::new();
                for v in list {
                    if !out.iter().any(|x| x.equals(v)) {
                        out.push(v.clone());
                    }
                }
                Value::List(out)
            }
            "mean" | "average" => {
                let n: Vec<f64> = list.iter().filter_map(Value::number).collect();
                if n.is_empty() { Value::Null } else { Value::Number(n.iter().sum::<f64>() / n.len() as f64) }
            }
            _ => return Err(format!("Unbekannte Funktion: {method}")),
        },
        other => {
            let s = other.display();
            match method {
                "contains" => Value::Bool(arg.is_some_and(|a| contains(other, a))),
                "containsAll" => Value::Bool(args.iter().all(|a| contains(other, a))),
                "containsAny" => Value::Bool(args.iter().any(|a| contains(other, a))),
                "endsWith" => Value::Bool(s.ends_with(&arg_text())),
                "startsWith" => Value::Bool(s.starts_with(&arg_text())),
                "lower" => Value::Text(s.to_lowercase()),
                "replace" => match arg {
                    Some(Value::Regex(p)) => Value::Text(regex::Regex::new(p).map_or(s.clone(), |r| r.replace_all(&s, args.get(1).map(Value::display).unwrap_or_default().as_str()).to_string())),
                    _ => Value::Text(s.replace(&arg_text(), &args.get(1).map(Value::display).unwrap_or_default())),
                },
                "repeat" => Value::Text(s.repeat(int(0).unwrap_or(0).max(0) as usize)),
                "reverse" => Value::Text(s.chars().rev().collect()),
                "slice" => {
                    let chars: Vec<char> = s.chars().collect();
                    let (start, end) = slice_range(chars.len(), int(0), int(1));
                    Value::Text(chars[start..end].iter().collect())
                }
                "split" => Value::List(match arg {
                    Some(Value::Regex(p)) => regex::Regex::new(p).map_or(vec![Value::Text(s.clone())], |r| r.split(&s).map(|x| Value::Text(x.to_string())).collect()),
                    _ => s.split(&arg_text()).map(|x| Value::Text(x.to_string())).collect(),
                }),
                "title" => Value::Text(s.split(' ').map(|w| {
                    let mut c = w.chars();
                    c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase())
                }).collect::<Vec<_>>().join(" ")),
                "trim" => Value::Text(s.trim().to_string()),
                _ if matches!(other, Value::Null) => Value::Null,
                _ => return Err(format!("Unbekannte Funktion: {method}")),
            }
        }
    })
}

fn slice_range(len: usize, start: Option<i64>, end: Option<i64>) -> (usize, usize) {
    let at = |v: i64| if v < 0 { (len as i64 + v).max(0) as usize } else { (v as usize).min(len) };
    let s = at(start.unwrap_or(0));
    let e = end.map_or(len, at);
    (s.min(e), e)
}

fn relative_time(seconds: i64) -> String {
    let (n, unit) = match seconds.abs() {
        s if s < 3600 => (s / 60, "min"),
        s if s < 86400 => (s / 3600, "h"),
        s => (s / 86400, "d"),
    };
    if seconds >= 0 { format!("-{n} {unit}") } else { format!("+{n} {unit}") }
}

// MARK: The file format

/// A condition: one statement, or `and`, `or`, `not` over others.
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    Statement(Expr, String),
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Not(Vec<Filter>),
}

impl Filter {
    fn parse(y: &Yaml) -> Result<Self, String> {
        match y {
            Yaml::String(s) => Ok(Filter::Statement(parse(s)?, s.clone())),
            Yaml::Mapping(m) => {
                let list = |key: &str| -> Option<Result<Vec<Filter>, String>> { m.get(key).and_then(Yaml::as_sequence).map(|l| l.iter().map(Filter::parse).collect()) };
                if let Some(l) = list("and") {
                    Ok(Filter::And(l?))
                } else if let Some(l) = list("or") {
                    Ok(Filter::Or(l?))
                } else if let Some(l) = list("not") {
                    Ok(Filter::Not(l?))
                } else {
                    Err("Filter braucht and, or oder not".into())
                }
            }
            _ => Err("Filter nicht lesbar".into()),
        }
    }

    pub fn matches(&self, row: &Row) -> Result<bool, String> {
        Ok(match self {
            Filter::Statement(e, _) => row.eval(e)?.truthy(),
            Filter::And(l) => {
                for f in l {
                    if !f.matches(row)? {
                        return Ok(false);
                    }
                }
                true
            }
            Filter::Or(l) => {
                for f in l {
                    if f.matches(row)? {
                        return Ok(true);
                    }
                }
                false
            }
            Filter::Not(l) => {
                for f in l {
                    if f.matches(row)? {
                        return Ok(false);
                    }
                }
                true
            }
        })
    }
}

/// One way of looking at the data.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct View {
    pub kind: String,
    pub name: String,
    pub filter: Option<Filter>,
    pub order: Vec<String>,
    pub group_by: Option<(String, bool)>,
    pub limit: Option<usize>,
    pub summaries: BTreeMap<String, String>,
}

/// A parsed `.base` file.
#[derive(Debug, Clone, Default)]
pub struct Base {
    pub filter: Option<Filter>,
    pub formulas: HashMap<String, Expr>,
    pub formula_errors: Vec<String>,
    pub display_names: HashMap<String, String>,
    pub summaries: HashMap<String, String>,
    pub views: Vec<View>,
}

impl Base {
    pub fn parse(source: &str) -> Result<Self, String> {
        if source.trim().is_empty() {
            return Ok(Self { views: vec![View { kind: "table".into(), name: "Table".into(), ..Default::default() }], ..Default::default() });
        }
        let root: Yaml = serde_yaml_ng::from_str(source).map_err(|e| e.to_string())?;
        let mut base = Self::default();
        if let Some(f) = root.get("filters") {
            base.filter = Some(Filter::parse(f)?);
        }
        if let Some(m) = root.get("formulas").and_then(Yaml::as_mapping) {
            for (k, v) in m {
                let (Some(name), Some(text)) = (k.as_str(), v.as_str()) else { continue };
                match parse(text) {
                    Ok(e) => {
                        base.formulas.insert(name.to_string(), e);
                    }
                    Err(e) => base.formula_errors.push(format!("{name}: {e}")),
                }
            }
        }
        if let Some(m) = root.get("properties").and_then(Yaml::as_mapping) {
            for (k, v) in m {
                if let (Some(name), Some(shown)) = (k.as_str(), v.get("displayName").and_then(Yaml::as_str)) {
                    base.display_names.insert(name.to_string(), shown.to_string());
                }
            }
        }
        if let Some(m) = root.get("summaries").and_then(Yaml::as_mapping) {
            for (k, v) in m {
                if let (Some(name), Some(text)) = (k.as_str(), v.as_str()) {
                    base.summaries.insert(name.to_string(), text.to_string());
                }
            }
        }
        if let Some(l) = root.get("views").and_then(Yaml::as_sequence) {
            for v in l {
                let text = |k: &str| v.get(k).and_then(Yaml::as_str).map(String::from);
                let mut view = View { kind: text("type").unwrap_or_else(|| "table".into()), name: text("name").unwrap_or_default(), ..Default::default() };
                if let Some(f) = v.get("filters") {
                    view.filter = Some(Filter::parse(f)?);
                }
                view.order = v.get("order").and_then(Yaml::as_sequence).map(|o| o.iter().filter_map(Yaml::as_str).map(String::from).collect()).unwrap_or_default();
                view.limit = v.get("limit").and_then(Yaml::as_u64).map(|n| n as usize);
                if let Some(g) = v.get("groupBy") {
                    let property = g.get("property").and_then(Yaml::as_str).map(String::from);
                    let descending = g.get("direction").and_then(Yaml::as_str).is_some_and(|d| d.eq_ignore_ascii_case("desc"));
                    view.group_by = property.map(|p| (p, descending));
                }
                if let Some(s) = v.get("summaries").and_then(Yaml::as_mapping) {
                    for (k, x) in s {
                        if let (Some(k), Some(x)) = (k.as_str(), x.as_str()) {
                            view.summaries.insert(k.to_string(), x.to_string());
                        }
                    }
                }
                base.views.push(view);
            }
        }
        if base.views.is_empty() {
            base.views.push(View { kind: "table".into(), name: "Table".into(), ..Default::default() });
        }
        Ok(base)
    }

    /// The columns a view shows.
    pub fn columns(&self, view: &View) -> Vec<String> {
        if view.order.is_empty() { vec!["file.name".to_string()] } else { view.order.clone() }
    }

    /// What a column is called.
    pub fn column_label(&self, column: &str) -> String {
        if let Some(shown) = self.display_names.get(column) {
            return shown.clone();
        }
        if column == "file.name" || column == "file.basename" {
            return crate::core::i18n::tr("Name");
        }
        column.strip_prefix("note.").or(column.strip_prefix("formula.")).or(column.strip_prefix("file.")).unwrap_or(column).to_string()
    }

    /// The rows of a view, with their cell values, sorted by file name.
    pub fn rows(&self, view: &View, index: &VaultIndex, this: Option<&Path>, now: DateTime<Local>) -> Result<Vec<ResultRow>, String> {
        let columns: Vec<(String, Expr)> = self.columns(view).into_iter().map(|c| parse(&c).map(|e| (c, e))).collect::<Result<_, _>>()?;
        let group = view.group_by.as_ref().map(|(p, _)| parse(p)).transpose()?;
        let mut files: Vec<&PathBuf> = index.files.iter().filter(|f| !index.config.is_ignored(&f.strip_prefix(&index.root).unwrap_or(f).to_string_lossy())).collect();
        files.sort();
        let mut out = Vec::new();
        for file in files {
            let row = Row { path: file, index, this, formulas: &self.formulas, depth: Default::default(), now };
            if let Some(f) = &self.filter
                && !f.matches(&row)?
            {
                continue;
            }
            if let Some(f) = &view.filter
                && !f.matches(&row)?
            {
                continue;
            }
            let cells = columns.iter().map(|(name, e)| (name.clone(), row.eval(e).unwrap_or(Value::Null))).collect();
            let group_value = group.as_ref().map(|g| row.eval(g).unwrap_or(Value::Null));
            out.push(ResultRow { path: file.clone(), cells, group: group_value });
            if group.is_none() && view.limit.is_some_and(|l| out.len() >= l) {
                break;
            }
        }
        if let Some((_, descending)) = &view.group_by {
            out.sort_by(|a, b| {
                let order = a.group.as_ref().and_then(|x| b.group.as_ref().and_then(|y| x.compare(y))).unwrap_or(std::cmp::Ordering::Equal);
                if *descending { order.reverse() } else { order }
            });
        }
        if let Some(limit) = view.limit {
            out.truncate(limit);
        }
        Ok(out)
    }

    /// The summary of one column.
    pub fn summary(&self, name: &str, values: &[Value], index: &VaultIndex, now: DateTime<Local>) -> Value {
        if let Some(text) = self.summaries.get(name)
            && let Ok(e) = parse(text)
        {
            let formulas = HashMap::new();
            let empty = PathBuf::new();
            let row = Row { path: &empty, index, this: None, formulas: &formulas, depth: Default::default(), now };
            return row.eval(&substitute_values(&e, values)).unwrap_or(Value::Null);
        }
        builtin_summary(name, values)
    }
}

fn substitute_values(e: &Expr, values: &[Value]) -> Expr {
    let go = |x: &Expr| substitute_values(x, values);
    match e {
        Expr::Name(n) if n == "values" => Expr::Literal(Value::List(values.to_vec())),
        Expr::Member(a, b) => Expr::Member(go(a).into(), b.clone()),
        Expr::Index(a, b) => Expr::Index(go(a).into(), go(b).into()),
        Expr::Call(a, args) => Expr::Call(go(a).into(), args.iter().map(go).collect()),
        Expr::Unary(c, a) => Expr::Unary(*c, go(a).into()),
        Expr::Binary(op, a, b) => Expr::Binary(op.clone(), go(a).into(), go(b).into()),
        Expr::ListOf(items) => Expr::ListOf(items.iter().map(go).collect()),
        other => other.clone(),
    }
}

/// Obsidian's default summaries.
pub fn builtin_summary(name: &str, values: &[Value]) -> Value {
    let numbers: Vec<f64> = values.iter().filter_map(Value::number).collect();
    let dates: Vec<DateTime<Local>> = values.iter().filter_map(|v| if let Value::Date(d) = v { Some(*d) } else { None }).collect();
    let sum: f64 = numbers.iter().sum();
    let count = numbers.len() as f64;
    let filled = values.iter().filter(|v| !matches!(v, Value::Null) && !v.display().is_empty()).count();
    match name {
        "Average" if count > 0. => Value::Number(sum / count),
        "Min" if count > 0. => Value::Number(numbers.iter().copied().fold(f64::MAX, f64::min)),
        "Max" if count > 0. => Value::Number(numbers.iter().copied().fold(f64::MIN, f64::max)),
        "Sum" => Value::Number(sum),
        "Range" if !dates.is_empty() => Value::Duration((*dates.iter().max().unwrap() - *dates.iter().min().unwrap()).num_milliseconds()),
        "Range" if count > 0. => Value::Number(numbers.iter().copied().fold(f64::MIN, f64::max) - numbers.iter().copied().fold(f64::MAX, f64::min)),
        "Median" if count > 0. => {
            let mut sorted = numbers.clone();
            sorted.sort_by(f64::total_cmp);
            let mid = sorted.len() / 2;
            Value::Number(if sorted.len() % 2 == 0 { (sorted[mid - 1] + sorted[mid]) / 2. } else { sorted[mid] })
        }
        "Stddev" if count > 0. => {
            let mean = sum / count;
            Value::Number((numbers.iter().map(|n| (n - mean).powi(2)).sum::<f64>() / count).sqrt())
        }
        "Earliest" => dates.iter().min().map_or(Value::Null, |d| Value::Date(*d)),
        "Latest" => dates.iter().max().map_or(Value::Null, |d| Value::Date(*d)),
        "Checked" => Value::Number(values.iter().filter(|v| v.truthy()).count() as f64),
        "Unchecked" => Value::Number(values.iter().filter(|v| !v.truthy()).count() as f64),
        "Empty" => Value::Number((values.len() - filled) as f64),
        "Filled" => Value::Number(filled as f64),
        "Unique" => {
            let mut seen: Vec<String> = values.iter().map(Value::display).collect();
            seen.sort();
            seen.dedup();
            Value::Number(seen.len() as f64)
        }
        _ => Value::Null,
    }
}

/// One line of a view.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultRow {
    pub path: PathBuf,
    pub cells: Vec<(String, Value)>,
    pub group: Option<Value>,
}

/// Orders two cell values the way a table column sorts.
pub fn compare(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Null, _) => std::cmp::Ordering::Greater,
        (_, Value::Null) => std::cmp::Ordering::Less,
        (Value::Number(x), Value::Number(y)) => x.total_cmp(y),
        _ => a.display().to_lowercase().cmp(&b.display().to_lowercase()),
    }
}

fn yaml_of(source: &str) -> Option<serde_yaml_ng::Mapping> {
    match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(source).ok()? {
        serde_yaml_ng::Value::Mapping(m) => Some(m),
        _ => None,
    }
}

/// Replaces one top-level section and leaves every other line as it is.
pub fn replace_section(source: &str, key: &str, value: &serde_yaml_ng::Value) -> String {
    let body = serde_yaml_ng::to_string(&serde_yaml_ng::Mapping::from_iter([(serde_yaml_ng::Value::from(key), value.clone())])).unwrap_or_default();
    let prefix = format!("{key}:");
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let Some(start) = lines.iter().position(|l| l.starts_with(&prefix)) else {
        let mut out = source.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&body);
        return out;
    };
    let end = lines[start + 1..].iter().position(|l| !(l.starts_with(' ') || l.starts_with('\t') || l.starts_with('-') || l.trim().is_empty())).map_or(lines.len(), |i| start + 1 + i);
    let mut out = lines[..start].concat();
    out.push_str(&body);
    out.push_str(&lines[end..].concat());
    out
}

fn with_view(source: &str, index: usize, change: impl FnOnce(&mut serde_yaml_ng::Mapping)) -> Option<String> {
    let root = yaml_of(source)?;
    let mut views = root.get("views")?.as_sequence()?.clone();
    change(views.get_mut(index)?.as_mapping_mut()?);
    Some(replace_section(source, "views", &serde_yaml_ng::Value::Sequence(views)))
}

/// Sets a view's columns.
pub fn set_view_order(source: &str, index: usize, order: &[String]) -> Option<String> {
    with_view(source, index, |view| {
        view.insert("order".into(), serde_yaml_ng::Value::Sequence(order.iter().map(|c| c.clone().into()).collect()));
    })
}

/// The column a view sorts by, and whether it goes upwards.
pub fn view_sort(source: &str, index: usize) -> Option<(String, bool)> {
    let root = yaml_of(source)?;
    let first = root.get("views")?.as_sequence()?.get(index)?.get("sort")?.as_sequence()?.first()?.clone();
    let direction = first.get("direction").and_then(|d| d.as_str()).unwrap_or("ASC");
    Some((first.get("property")?.as_str()?.to_string(), !direction.eq_ignore_ascii_case("DESC")))
}

/// Sets or removes a view's sorting.
pub fn set_view_sort(source: &str, index: usize, sort: Option<&(String, bool)>) -> Option<String> {
    with_view(source, index, |view| match sort {
        Some((column, up)) => {
            let mut entry = serde_yaml_ng::Mapping::new();
            entry.insert("property".into(), column.clone().into());
            entry.insert("direction".into(), if *up { "ASC" } else { "DESC" }.into());
            view.insert("sort".into(), serde_yaml_ng::Value::Sequence(vec![entry.into()]));
        }
        None => {
            view.remove("sort");
        }
    })
}

/// The conditions of a view when they are a plain list; nested filters give `None`.
pub fn view_filter_lines(source: &str, index: usize) -> Option<Vec<String>> {
    let root = yaml_of(source)?;
    let view = root.get("views")?.as_sequence()?.get(index)?;
    let Some(filters) = view.get("filters") else { return Some(Vec::new()) };
    if let Some(line) = filters.as_str() {
        return Some(vec![line.to_string()]);
    }
    filters.get("and")?.as_sequence()?.iter().map(|l| l.as_str().map(String::from)).collect()
}

/// Sets a view's conditions.
pub fn set_view_filter(source: &str, index: usize, lines: &[String]) -> Option<String> {
    with_view(source, index, |view| {
        if lines.is_empty() {
            view.remove("filters");
        } else {
            let mut all = serde_yaml_ng::Mapping::new();
            all.insert("and".into(), serde_yaml_ng::Value::Sequence(lines.iter().map(|l| l.clone().into()).collect()));
            view.insert("filters".into(), all.into());
        }
    })
}

/// Writes an edited table cell into the note's front matter. Only plain note properties can be edited.
pub fn write_cell(text: &str, column: &str, value: &str) -> Option<String> {
    let name = column.strip_prefix("note.").unwrap_or(column);
    if name.contains('.') || name.contains('(') {
        return None;
    }
    let kind = frontmatter::properties(text).and_then(|p| p.into_iter().find(|x| x.name == name).map(|x| x.kind)).unwrap_or(PropertyKind::Text);
    let values: Vec<String> = match kind {
        PropertyKind::List | PropertyKind::Tags => value.split(',').map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect(),
        _ => vec![value.to_string()],
    };
    Some(frontmatter::set(text, name, kind, &values))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::vault::index::{facts_of, rebuild_for_test};

    #[test]
    fn view_edits_only_touch_their_section() {
        let source = "# Kommentar bleibt\nfilters:\n  and:\n    - a == 1\nviews:\n  - type: table\n    name: T\n    order: [file.name]\nsummaries:\n  s: values.length\n";
        let ordered = set_view_order(source, 0, &["x".to_string(), "y".to_string()]).unwrap();
        assert!(ordered.starts_with("# Kommentar bleibt\nfilters:\n  and:\n    - a == 1\n"));
        assert!(ordered.ends_with("summaries:\n  s: values.length\n"));
        assert_eq!(Base::parse(&ordered).unwrap().views[0].order, ["x", "y"]);
        let sorted = set_view_sort(source, 0, Some(&("x".to_string(), false))).unwrap();
        assert_eq!(view_sort(&sorted, 0), Some(("x".to_string(), false)));
        assert_eq!(view_sort(&set_view_sort(&sorted, 0, None).unwrap(), 0), None);
        let filtered = set_view_filter(source, 0, &["b > 2".to_string()]).unwrap();
        assert_eq!(view_filter_lines(&filtered, 0).unwrap(), ["b > 2"]);
        assert!(view_filter_lines(&set_view_filter(&filtered, 0, &[]).unwrap(), 0).unwrap().is_empty());
        assert_eq!(view_filter_lines("views:\n  - filters:\n      or:\n        - a\n", 0), None);
        assert!(set_view_order(source, 3, &[]).is_none());
    }

    fn fixture() -> (PathBuf, VaultIndex) {
        let dir = std::env::temp_dir().join(format!("schrumbi-bases-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Uni")).unwrap();
        let notes = [
            ("Uni/A.md", "---\nstatus: offen\npunkte: 10\nfertig: false\ndatum: 2026-11-03\ntags: [uni, uni/p1]\nlink: \"[[B]]\"\n---\n[[B]] #inline\n"),
            ("Uni/B.md", "---\nstatus: fertig\npunkte: 30\nfertig: true\ndatum: 2026-10-01\ntags: [uni]\n---\nText\n"),
            ("C.md", "---\nstatus: offen\npunkte: 5\n---\nText\n"),
        ];
        for (p, t) in notes {
            std::fs::write(dir.join(p), t).unwrap();
        }
        let dir = dunce::canonicalize(&dir).unwrap();
        let index = rebuild_for_test(&dir, |t| facts_of(t, 0.));
        (dir, index)
    }

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap()
    }

    fn eval_for(dir: &Path, index: &VaultIndex, file: &str, text: &str) -> Value {
        let formulas = HashMap::new();
        let path = dir.join(file);
        Row { path: &path, index, this: None, formulas: &formulas, depth: Default::default(), now: now() }.eval(&parse(text).unwrap()).unwrap()
    }

    #[test]
    fn expressions_cover_operators_and_every_function_family() {
        let (dir, i) = fixture();
        let e = |t: &str| eval_for(&dir, &i, "Uni/A.md", t);
        assert_eq!(e("1 + 2 * 3"), Value::Number(7.));
        assert_eq!(e("(1 + 2) * 3 % 4"), Value::Number(1.));
        assert_eq!(e("punkte > 5 && !fertig"), Value::Bool(true));
        assert_eq!(e("status == \"offen\" || false"), Value::Bool(true));
        assert_eq!(e("note.status.upper"), Value::Null);
        assert_eq!(e("note[\"status\"]"), Value::Text("offen".into()));
        assert_eq!(e("file.name"), Value::Text("A.md".into()));
        assert_eq!(e("file.basename + \"!\""), Value::Text("A!".into()));
        assert_eq!(e("file.folder"), Value::Text("Uni".into()));
        assert_eq!(e("file.hasTag(\"uni\")"), Value::Bool(true));
        assert_eq!(e("file.hasTag(\"p1\")"), Value::Bool(false));
        assert_eq!(e("file.hasTag(\"inline\")"), Value::Bool(true));
        assert_eq!(e("file.inFolder(\"Uni\")"), Value::Bool(true));
        assert_eq!(e("file.hasLink(\"B\")"), Value::Bool(true));
        assert_eq!(e("file.hasProperty(\"punkte\")"), Value::Bool(true));
        assert_eq!(e("link.asFile().name"), Value::Text("B.md".into()));
        assert_eq!(e("link.linksTo(\"B\")"), Value::Bool(true));
        assert_eq!(e("\"Abc\".lower()"), Value::Text("abc".into()));
        assert_eq!(e("\"a,b\".split(\",\").join(\"+\")"), Value::Text("a+b".into()));
        assert_eq!(e("\"hallo welt\".title()"), Value::Text("Hallo Welt".into()));
        assert_eq!(e("\"abc\".slice(1)"), Value::Text("bc".into()));
        assert_eq!(e("\"abc\".reverse()"), Value::Text("cba".into()));
        assert_eq!(e("\"ab\".repeat(2)"), Value::Text("abab".into()));
        assert_eq!(e("\"abc\".replace(/b/, \"x\")"), Value::Text("axc".into()));
        assert_eq!(e("\"abc\".contains(\"B\")"), Value::Bool(true));
        assert_eq!(e("\"abc\".containsAny(\"z\", \"a\")"), Value::Bool(true));
        assert_eq!(e("\" a \".trim().startsWith(\"a\")"), Value::Bool(true));
        assert_eq!(e("/a+/.matches(\"caat\")"), Value::Bool(true));
        assert_eq!(e("1.256.round(1)"), Value::Number(1.3));
        assert_eq!(e("1.5.toFixed(2)"), Value::Text("1.50".into()));
        assert_eq!(e("[1, 2, 2].unique().length"), Value::Number(2.));
        assert_eq!(e("[3, 1].sort()[0]"), Value::Number(1.));
        assert_eq!(e("[1, 2].contains(2)"), Value::Bool(true));
        assert_eq!(e("[1, [2]].flat().length"), Value::Number(2.));
        assert_eq!(e("[1, 2, 3].filter(value > 1).length"), Value::Number(2.));
        assert_eq!(e("[1, 2].map(value * 2)[1]"), Value::Number(4.));
        assert_eq!(e("[1, 2, 3].reduce(acc + value, 0)"), Value::Number(6.));
        assert_eq!(e("max(1, 3)"), Value::Number(3.));
        assert_eq!(e("min(1, 3)"), Value::Number(1.));
        assert_eq!(e("if(punkte > 5, \"viel\", \"wenig\")"), Value::Text("viel".into()));
        assert_eq!(e("number(\"4\") + 1"), Value::Number(5.));
        assert_eq!(e("list(1).length"), Value::Number(1.));
        assert_eq!(e("link(\"X\", \"Name\")"), Value::Link("X".into(), Some("Name".into())));
        assert_eq!(e("null.isEmpty()"), Value::Bool(true));
        assert_eq!(e("\"\".isEmpty()"), Value::Bool(true));
        assert_eq!(e("1.isType(\"number\")"), Value::Bool(true));
        assert_eq!(e("true.isTruthy()"), Value::Bool(true));
    }

    #[test]
    fn dates_and_durations_calculate() {
        let (dir, i) = fixture();
        let e = |t: &str| eval_for(&dir, &i, "Uni/A.md", t);
        assert_eq!(e("datum > today()"), Value::Bool(true));
        assert_eq!(e("datum.format(\"DD.MM.YYYY\")"), Value::Text("03.11.2026".into()));
        assert_eq!(e("datum.year"), Value::Number(2026.));
        assert_eq!(e("(date(\"2026-01-02\") - date(\"2026-01-01\")) == duration(\"1d\")"), Value::Bool(true));
        assert_eq!(e("(date(\"2026-01-01\") + \"1M\").date() == date(\"2026-01-31\")"), Value::Bool(true));
        assert_eq!(parse_duration("-2 hours"), Some(-7_200_000));
        assert_eq!(parse_duration("kaputt"), None);
    }

    #[test]
    fn a_base_file_gives_the_right_rows_groups_and_summaries() {
        let (dir, i) = fixture();
        let base = Base::parse(
            r#"
filters:
  and:
    - file.ext == "md"
    - or:
        - status == "offen"
        - status == "fertig"
formulas:
  doppelt: punkte * 2
properties:
  note.status:
    displayName: Zustand
summaries:
  Spanne: values.length
views:
  - type: table
    name: Alle
    order: [file.name, note.status, formula.doppelt]
    summaries:
      formula.doppelt: Sum
  - type: cards
    name: Uni
    filters: 'file.inFolder("Uni")'
    groupBy:
      property: status
      direction: DESC
  - type: list
    name: Eins
    limit: 1
"#,
        )
        .unwrap();
        let all = &base.views[0];
        let rows = base.rows(all, &i, None, now()).unwrap();
        assert_eq!(rows.iter().map(|r| r.path.file_name().unwrap().to_str().unwrap()).collect::<Vec<_>>(), ["C.md", "A.md", "B.md"]);
        assert_eq!(rows[1].cells[2].1, Value::Number(20.));
        assert_eq!(base.column_label("note.status"), "Zustand");
        assert_eq!(base.column_label("formula.doppelt"), "doppelt");
        let doubled: Vec<Value> = rows.iter().map(|r| r.cells[2].1.clone()).collect();
        assert_eq!(base.summary("Sum", &doubled, &i, now()), Value::Number(90.));
        assert_eq!(base.summary("Spanne", &doubled, &i, now()), Value::Number(3.));
        let uni = base.rows(&base.views[1], &i, None, now()).unwrap();
        assert_eq!(uni.iter().map(|r| r.group.clone().unwrap().display()).collect::<Vec<_>>(), ["offen", "fertig"]);
        assert_eq!(base.rows(&base.views[2], &i, None, now()).unwrap().len(), 1);
        let _ = dir;
    }

    #[test]
    fn this_points_at_the_embedding_file() {
        let (dir, i) = fixture();
        let base = Base::parse("views:\n  - type: table\n    filters: 'file.hasLink(this)'\n").unwrap();
        let rows = base.rows(&base.views[0], &i, Some(&dir.join("Uni/B.md")), now()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, dir.join("Uni/A.md"));
    }

    #[test]
    fn default_summaries_match_obsidian() {
        let n = [Value::Number(1.), Value::Number(2.), Value::Number(6.), Value::Null];
        assert_eq!(builtin_summary("Average", &n), Value::Number(3.));
        assert_eq!(builtin_summary("Min", &n), Value::Number(1.));
        assert_eq!(builtin_summary("Max", &n), Value::Number(6.));
        assert_eq!(builtin_summary("Sum", &n), Value::Number(9.));
        assert_eq!(builtin_summary("Range", &n), Value::Number(5.));
        assert_eq!(builtin_summary("Median", &n), Value::Number(2.));
        assert_eq!(builtin_summary("Empty", &n), Value::Number(1.));
        assert_eq!(builtin_summary("Filled", &n), Value::Number(3.));
        assert_eq!(builtin_summary("Unique", &[Value::Number(1.), Value::Number(1.)]), Value::Number(1.));
        let flags = [Value::Bool(true), Value::Bool(false), Value::Bool(true)];
        assert_eq!(builtin_summary("Checked", &flags), Value::Number(2.));
        assert_eq!(builtin_summary("Unchecked", &flags), Value::Number(1.));
        let a = Local.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let b = Local.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap();
        assert_eq!(builtin_summary("Earliest", &[Value::Date(b), Value::Date(a)]), Value::Date(a));
        assert_eq!(builtin_summary("Latest", &[Value::Date(b), Value::Date(a)]), Value::Date(b));
        assert_eq!(builtin_summary("Range", &[Value::Date(b), Value::Date(a)]), Value::Duration(86_400_000));
    }

    #[test]
    fn broken_input_says_so_instead_of_crashing() {
        assert!(Base::parse("views: [").is_err());
        assert!(parse("1 +").is_err());
        assert!(parse("\"offen").is_err());
        assert!(parse("a b").is_err());
        let base = Base::parse("formulas:\n  kaputt: '1 +'\n  gut: '1'\n").unwrap();
        assert_eq!(base.formula_errors.len(), 1);
        assert!(base.formulas.contains_key("gut"));
        let (dir, i) = fixture();
        let mut formulas = HashMap::new();
        formulas.insert("a".to_string(), parse("formula.a").unwrap());
        let path = dir.join("C.md");
        let row = Row { path: &path, index: &i, this: None, formulas: &formulas, depth: Default::default(), now: now() };
        assert!(row.eval(&parse("formula.a").unwrap()).is_err());
        assert!(row.eval(&parse("gibtsnicht(1)").unwrap()).is_err());
    }

    #[test]
    fn editing_a_cell_writes_only_that_property() {
        let note = "---\nstatus: offen\ntags: [a]\n---\nText\n";
        assert_eq!(write_cell(note, "note.status", "fertig").unwrap(), "---\nstatus: fertig\ntags: [a]\n---\nText\n");
        assert!(write_cell(note, "tags", "x, y").unwrap().contains("tags: [x, y]\n"));
        assert!(write_cell("---\ntags:\n  - a\n---\n", "tags", "a, y").unwrap().contains("  - a\n  - y\n"));
        assert_eq!(write_cell(note, "file.name", "x"), None);
        assert_eq!(write_cell(note, "formula.x", "x"), None);
        assert!(write_cell("Text", "neu", "1").unwrap().starts_with("---\nneu: "));
    }
}
