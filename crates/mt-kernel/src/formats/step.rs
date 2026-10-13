//! A reader and writer helpers for STEP physical files (ISO 10303-21, "Part 21"),
//! the text encoding of IFC files ([`super::ifc`]).
//!
//! - The reader keeps every instance of the DATA sections as a class name (upper
//!   case, as written) and its parameters, by instance number, plus the header's
//!   `FILE_SCHEMA`. It knows no schema: callers pick parameters by position.
//! - Complex instances (`#1=(A(..)B(..));`) keep their first partial's name and the
//!   parameters of all partials in order.
//! - Strings are decoded: `''`, `\\`, `\S\`, `\X\hh` (ISO 8859-1), `\X2\..\X0\`
//!   (UTF-16) and `\X4\..\X0\` (UTF-32); `\P.\` page directives are dropped.
//! - Comments `/* */` are skipped anywhere between tokens.
//!
//! No crate on crates.io fitted: `ruststep` (Apache-2.0) generates schema code at
//! build time and has no IFC schema, `ifc-lite-core` is MPL-2.0, `oxideav-ifc` is a
//! 0.0.x preview with its own framework dependencies.

use std::collections::BTreeMap;

/// A parameter value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `$`: not given.
    Null,
    /// `*`: derived in a subtype.
    Derived,
    Int(i64),
    Real(f64),
    Str(String),
    /// `.NAME.` (booleans are `.T.` / `.F.`).
    Enum(String),
    /// `#n`.
    Ref(u64),
    List(Vec<Value>),
    /// `NAME(value)`, a value of a defined type (`IFCLABEL('x')`).
    Typed(String, Box<Value>),
    /// `"hex"`.
    Binary(String),
}

impl Value {
    pub fn as_ref_id(&self) -> Option<u64> {
        match self {
            Self::Ref(r) => Some(*r),
            Self::Typed(_, v) => v.as_ref_id(),
            _ => None,
        }
    }

    /// A number (integers too, and typed numbers such as `IFCLENGTHMEASURE(2.)`).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Real(x) => Some(*x),
            Self::Int(i) => Some(*i as f64),
            Self::Typed(_, v) => v.as_f64(),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Real(x) if x.fract() == 0.0 => Some(*x as i64),
            Self::Typed(_, v) => v.as_i64(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            Self::Typed(_, v) => v.as_str(),
            _ => None,
        }
    }

    pub fn as_enum(&self) -> Option<&str> {
        match self {
            Self::Enum(s) => Some(s),
            _ => None,
        }
    }

    /// The items of a list; empty for anything else.
    pub fn list(&self) -> &[Value] {
        match self {
            Self::List(v) => v,
            Self::Typed(_, v) => v.list(),
            _ => &[],
        }
    }

    /// The instances a list refers to (or the one a reference names).
    pub fn refs(&self) -> Vec<u64> {
        match self {
            Self::Ref(r) => vec![*r],
            _ => self.list().iter().filter_map(Value::as_ref_id).collect(),
        }
    }

    /// The numbers of a list.
    pub fn numbers(&self) -> Vec<f64> {
        self.list().iter().filter_map(Value::as_f64).collect()
    }
}

/// An entity instance.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// Upper case, e.g. `IFCWALL`.
    pub class: String,
    pub args: Vec<Value>,
}

impl Entity {
    /// Parameter `i`, `Null` when missing.
    pub fn arg(&self, i: usize) -> &Value {
        self.args.get(i).unwrap_or(&Value::Null)
    }
}

/// A parsed file.
#[derive(Debug, Clone, Default)]
pub struct File {
    /// `FILE_SCHEMA` names, upper case (`IFC2X3`, `IFC4`, `IFC4X3_ADD2`).
    pub schema: Vec<String>,
    pub entities: BTreeMap<u64, Entity>,
}

impl File {
    pub fn get(&self, id: u64) -> Option<&Entity> {
        self.entities.get(&id)
    }

    /// The entity a value refers to.
    pub fn deref(&self, v: &Value) -> Option<(u64, &Entity)> {
        let id = v.as_ref_id()?;
        Some((id, self.get(id)?))
    }
}

struct Lexer<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn err<T>(&self, what: &str) -> Result<T, String> {
        let line = 1 + self.b[..self.i.min(self.b.len())]
            .iter()
            .filter(|&&c| c == b'\n')
            .count();
        Err(format!("STEP file: {what} on line {line}"))
    }

    fn skip_ws(&mut self) {
        loop {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.b.get(self.i..).is_some_and(|r| r.starts_with(b"/*")) {
                match find(&self.b[self.i + 2..], b"*/") {
                    Some(k) => self.i += k + 4,
                    None => self.i = self.b.len(),
                }
            } else {
                return;
            }
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.b.get(self.i).copied()
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            self.err(&format!("expected '{}'", c as char))
        }
    }

    fn keyword(&mut self) -> String {
        self.skip_ws();
        let start = self.i;
        while self.i < self.b.len()
            && (self.b[self.i].is_ascii_alphanumeric() || matches!(self.b[self.i], b'_' | b'-'))
        {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.b[start..self.i]).to_ascii_uppercase()
    }

    fn digits(&mut self) -> &'a [u8] {
        let start = self.i;
        while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
            self.i += 1;
        }
        &self.b[start..self.i]
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > 64 {
            return self.err("nesting too deep");
        }
        let Some(c) = self.peek() else {
            return self.err("unexpected end");
        };
        Ok(match c {
            b'$' => {
                self.i += 1;
                Value::Null
            }
            b'*' => {
                self.i += 1;
                Value::Derived
            }
            b'#' => {
                self.i += 1;
                let d = self.digits();
                match std::str::from_utf8(d).ok().and_then(|s| s.parse().ok()) {
                    Some(n) => Value::Ref(n),
                    None => return self.err("bad reference"),
                }
            }
            b'\'' => Value::Str(self.string()?),
            b'"' => {
                self.i += 1;
                let start = self.i;
                while self.i < self.b.len() && self.b[self.i] != b'"' {
                    self.i += 1;
                }
                if self.i >= self.b.len() {
                    return self.err("unterminated binary value");
                }
                let s = String::from_utf8_lossy(&self.b[start..self.i]).into_owned();
                self.i += 1;
                Value::Binary(s)
            }
            b'.' if self
                .b
                .get(self.i + 1)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') =>
            {
                self.i += 1;
                let name = self.keyword();
                self.expect(b'.')?;
                Value::Enum(name)
            }
            b'(' => {
                self.i += 1;
                Value::List(self.args(depth + 1)?)
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.number()?,
            c if c.is_ascii_alphabetic() => {
                let name = self.keyword();
                self.expect(b'(')?;
                let mut args = self.args(depth + 1)?;
                let inner = if args.len() == 1 {
                    args.pop().unwrap_or(Value::Null)
                } else {
                    Value::List(args)
                };
                Value::Typed(name, Box::new(inner))
            }
            _ => return self.err(&format!("unexpected '{}'", c as char)),
        })
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        if matches!(self.b[self.i], b'+' | b'-') {
            self.i += 1;
        }
        let mut real = false;
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'0'..=b'9' => {}
                b'.' => real = true,
                b'e' | b'E' => {
                    real = true;
                    if matches!(self.b.get(self.i + 1), Some(b'+' | b'-')) {
                        self.i += 1;
                    }
                }
                _ => break,
            }
            self.i += 1;
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).unwrap_or("");
        let parsed = if real {
            // "1.E-5": Rust wants a digit after the point only before the exponent.
            text.replace(".E", ".0E")
                .replace(".e", ".0e")
                .parse::<f64>()
                .ok()
                .map(Value::Real)
        } else {
            text.parse::<i64>()
                .ok()
                .map(Value::Int)
                .or_else(|| text.parse::<f64>().ok().map(Value::Real))
        };
        match parsed {
            Some(v) => Ok(v),
            None => self.err(&format!("bad number '{text}'")),
        }
    }

    /// Values up to the closing parenthesis (the opening one already read).
    fn args(&mut self, depth: usize) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        if self.peek() == Some(b')') {
            self.i += 1;
            return Ok(out);
        }
        loop {
            out.push(self.value(depth)?);
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b')') => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => return self.err("expected ',' or ')'"),
            }
        }
    }

    /// A quoted string, decoded.
    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut raw = Vec::new();
        loop {
            match self.b.get(self.i) {
                None => return self.err("unterminated string"),
                Some(b'\'') if self.b.get(self.i + 1) == Some(&b'\'') => {
                    raw.push(b'\'');
                    self.i += 2;
                }
                Some(b'\'') => {
                    self.i += 1;
                    break;
                }
                Some(&c) => {
                    raw.push(c);
                    self.i += 1;
                }
            }
        }
        Ok(decode_string(&raw))
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn hex_val(s: &[u8]) -> Option<u32> {
    u32::from_str_radix(std::str::from_utf8(s).ok()?, 16).ok()
}

/// Decodes Part 21 string escapes (the quotes already removed and `''` undone).
pub fn decode_string(raw: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        if rest.starts_with(b"\\\\") {
            out.push('\\');
            i += 2;
        } else if rest.starts_with(b"\\X2\\") || rest.starts_with(b"\\X4\\") {
            let wide = rest[2] == b'4';
            let body_start = i + 4;
            let end = find(&raw[body_start..], b"\\X0\\").map_or(raw.len(), |k| body_start + k);
            let body = &raw[body_start..end];
            let step = if wide { 8 } else { 4 };
            let units: Vec<u32> = body.chunks(step).filter_map(hex_val).collect();
            if wide {
                out.extend(units.into_iter().filter_map(char::from_u32));
            } else {
                let u16s: Vec<u16> = units.into_iter().map(|u| u as u16).collect();
                out.extend(char::decode_utf16(u16s).map(|c| c.unwrap_or('\u{fffd}')));
            }
            i = (end + 4).min(raw.len());
        } else if rest.starts_with(b"\\X\\") && rest.len() >= 5 {
            if let Some(c) = hex_val(&rest[3..5]).and_then(char::from_u32) {
                out.push(c);
            }
            i += 5;
        } else if rest.starts_with(b"\\S\\") && rest.len() >= 4 {
            if let Some(c) = char::from_u32(u32::from(rest[3]) + 128) {
                out.push(c);
            }
            i += 4;
        } else if rest.starts_with(b"\\P") && rest.len() >= 4 && rest[3] == b'\\' {
            i += 4;
        } else {
            // Plain bytes; files written as UTF-8 come through too.
            let end = raw[i + 1..]
                .iter()
                .position(|&c| c == b'\\')
                .map_or(raw.len(), |k| i + 1 + k);
            out.push_str(&String::from_utf8_lossy(&raw[i..end]));
            i = end;
        }
    }
    out
}

/// Parses a Part 21 file.
pub fn parse(bytes: &[u8]) -> Result<File, String> {
    let mut lx = Lexer { b: bytes, i: 0 };
    let mut file = File::default();
    let magic = lx.keyword();
    if magic != "ISO-10303-21" {
        return Err("not a STEP (ISO 10303-21) file".into());
    }
    lx.expect(b';')?;
    loop {
        if lx.peek().is_none() {
            break;
        }
        let section = lx.keyword();
        match section.as_str() {
            "HEADER" => {
                lx.expect(b';')?;
                loop {
                    let name = lx.keyword();
                    if name == "ENDSEC" || name.is_empty() {
                        lx.expect(b';')?;
                        break;
                    }
                    lx.expect(b'(')?;
                    let args = lx.args(0)?;
                    lx.expect(b';')?;
                    if name == "FILE_SCHEMA" {
                        file.schema = args
                            .first()
                            .map(|v| {
                                v.list()
                                    .iter()
                                    .filter_map(|s| s.as_str())
                                    .map(|s| s.trim().to_ascii_uppercase())
                                    .collect()
                            })
                            .unwrap_or_default();
                    }
                }
            }
            "DATA" => {
                // DATA or DATA(name, schemas) in multi-section files.
                if lx.peek() == Some(b'(') {
                    lx.i += 1;
                    lx.args(0)?;
                }
                lx.expect(b';')?;
                loop {
                    match lx.peek() {
                        Some(b'#') => {}
                        Some(_) => {
                            let k = lx.keyword();
                            if k != "ENDSEC" {
                                return lx.err(&format!("unexpected '{k}' in DATA"));
                            }
                            lx.expect(b';')?;
                            break;
                        }
                        None => return lx.err("missing ENDSEC"),
                    }
                    lx.i += 1;
                    let id = std::str::from_utf8(lx.digits())
                        .ok()
                        .and_then(|s| s.parse::<u64>().ok());
                    let Some(id) = id else {
                        return lx.err("bad instance number");
                    };
                    lx.expect(b'=')?;
                    let entity = if lx.peek() == Some(b'(') {
                        // Complex instance: partials in a row.
                        lx.i += 1;
                        let mut class = String::new();
                        let mut args = Vec::new();
                        while lx.peek() != Some(b')') {
                            let name = lx.keyword();
                            if name.is_empty() {
                                return lx.err("bad complex instance");
                            }
                            lx.expect(b'(')?;
                            args.extend(lx.args(1)?);
                            if class.is_empty() {
                                class = name;
                            }
                        }
                        lx.i += 1;
                        Entity { class, args }
                    } else {
                        let class = lx.keyword();
                        if class.is_empty() {
                            return lx.err("missing class name");
                        }
                        lx.expect(b'(')?;
                        Entity {
                            class,
                            args: lx.args(1)?,
                        }
                    };
                    lx.expect(b';')?;
                    file.entities.insert(id, entity);
                }
            }
            "END-ISO-10303-21" => break,
            "" => return lx.err("unexpected character"),
            other => return lx.err(&format!("unknown section '{other}'")),
        }
    }
    Ok(file)
}

/// A string as a Part 21 literal: quotes doubled, backslashes escaped, non-ASCII as
/// `\X2\` UTF-16 hex.
pub fn quote(s: &str) -> String {
    let mut out = String::from("'");
    let mut wide = String::new();
    let flush = |out: &mut String, wide: &mut String| {
        if !wide.is_empty() {
            out.push_str("\\X2\\");
            out.push_str(wide);
            out.push_str("\\X0\\");
            wide.clear();
        }
    };
    for c in s.chars() {
        if c.is_ascii() && !c.is_ascii_control() {
            flush(&mut out, &mut wide);
            match c {
                '\'' => out.push_str("''"),
                '\\' => out.push_str("\\\\"),
                c => out.push(c),
            }
        } else if c.is_ascii_control() {
            flush(&mut out, &mut wide);
            out.push(' ');
        } else {
            let mut buf = [0u16; 2];
            for u in c.encode_utf16(&mut buf) {
                wide.push_str(&format!("{u:04X}"));
            }
        }
    }
    flush(&mut out, &mut wide);
    out.push('\'');
    out
}

/// A REAL literal: always with a decimal point (`0.`, `2.5`, `1.E-05`).
pub fn real(x: f64) -> String {
    let x = if x.is_finite() { x } else { 0.0 };
    let s = super::num(x);
    if s.contains('.') {
        s
    } else if s == "0" && x != 0.0 {
        // Below the 6 decimals of `num`.
        let e = format!("{x:E}");
        match e.split_once('E') {
            Some((m, exp)) if !m.contains('.') => format!("{m}.E{exp}"),
            _ => e,
        }
    } else {
        format!("{s}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_instances_and_values() {
        let text = b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('ViewDefinition'),'2;1');\n\
            FILE_NAME('a.ifc','2020-01-01T00:00:00',(''),(''),'x','y','');\n\
            FILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
            /* a comment */\n\
            #1= IFCCARTESIANPOINT((0.,1.E-05,-2.5E2));\n\
            #2=IFCWALL('2O2Fr$t4X7Zf8NOew3FLOH',$,'It''s \\X2\\00E9\\X0\\',*,.T.,#1,(#1,#3),IFCLABEL('x'),12);\n\
            #3=(IFCA(1)IFCB(2,'b'));\n\
            ENDSEC;\nEND-ISO-10303-21;\n";
        let f = parse(text).unwrap();
        assert_eq!(f.schema, ["IFC4"]);
        assert_eq!(f.entities.len(), 3);
        let p = f.get(1).unwrap();
        assert_eq!(p.class, "IFCCARTESIANPOINT");
        assert_eq!(p.arg(0).numbers(), [0.0, 1e-5, -250.0]);
        let w = f.get(2).unwrap();
        assert_eq!(w.arg(2).as_str(), Some("It's \u{e9}"));
        assert_eq!(w.arg(3), &Value::Derived);
        assert_eq!(w.arg(4).as_enum(), Some("T"));
        assert_eq!(w.arg(6).refs(), [1, 3]);
        assert_eq!(w.arg(7).as_str(), Some("x"));
        assert_eq!(w.arg(8).as_i64(), Some(12));
        assert_eq!(w.arg(20), &Value::Null);
        let c = f.get(3).unwrap();
        assert_eq!(c.class, "IFCA");
        assert_eq!(c.args.len(), 3);
        assert!(parse(b"hello").is_err());
        assert!(parse(b"ISO-10303-21;DATA;#1=X(1;ENDSEC;").is_err());
    }

    #[test]
    fn strings_round_trip() {
        for s in ["plain", "it's", "back\\slash", "\u{e9}t\u{e9} \u{1F600}"] {
            let q = quote(s);
            assert!(q.is_ascii());
            let inner = &q[1..q.len() - 1];
            assert_eq!(decode_string(inner.replace("''", "'").as_bytes()), s);
        }
        assert_eq!(decode_string(b"\\S\\D"), "\u{c4}");
        assert_eq!(decode_string(b"\\X\\E9t\\PA\\"), "\u{e9}t");
        assert_eq!(real(0.0), "0.");
        assert_eq!(real(2.5), "2.5");
        assert_eq!(real(1e-9), "1.E-9");
        assert_eq!(real(-3.0), "-3.");
    }
}
