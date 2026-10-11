//! A small XML reader for the simple documents of 3MF packages: start/end tags with
//! attributes (namespace prefixes kept in names), character data, comments,
//! processing instructions and CDATA skipped. Not validating.

/// One XML event.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Event<'a> {
    /// A start tag; `empty` for `<x/>` (no end event follows).
    Start {
        name: &'a str,
        attrs: Vec<(&'a str, String)>,
        empty: bool,
    },
    End(&'a str),
}

impl Event<'_> {
    /// The value of attribute `key` (matched without its namespace prefix).
    pub(crate) fn attr(&self, key: &str) -> Option<&str> {
        match self {
            Event::Start { attrs, .. } => attrs
                .iter()
                .find(|(k, _)| *k == key || local(k) == key)
                .map(|(_, v)| v.as_str()),
            Event::End(_) => None,
        }
    }
}

/// A name without its namespace prefix.
pub(crate) fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Decodes the XML entities and character references of `s`.
pub(crate) fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else { break };
        let ent = &rest[1..end];
        let ch = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "apos" => Some('\''),
            "quot" => Some('"'),
            _ => ent
                .strip_prefix("#x")
                .map(|h| u32::from_str_radix(h, 16).ok())
                .unwrap_or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
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

/// The tag events of `text`, in order.
pub(crate) fn events(text: &str) -> Result<Vec<Event<'_>>, String> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(off) = text[i..].find('<') {
        i += off;
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").ok_or("unterminated comment")? + 3;
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i += rest.find("]]>").ok_or("unterminated CDATA")? + 3;
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            i += rest.find('>').ok_or("unterminated declaration")? + 1;
            continue;
        }
        if let Some(r) = rest.strip_prefix("</") {
            let end = r.find('>').ok_or("unterminated end tag")?;
            out.push(Event::End(r[..end].trim()));
            i += 2 + end + 1;
            continue;
        }
        // Start tag: name, then attributes up to `>` outside quotes.
        let mut j = i + 1;
        while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
            j += 1;
        }
        let name = &text[i + 1..j];
        let mut attrs = Vec::new();
        loop {
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= b.len() {
                return Err("unterminated tag".into());
            }
            if b[j] == b'>' {
                out.push(Event::Start {
                    name,
                    attrs,
                    empty: false,
                });
                j += 1;
                break;
            }
            if b[j] == b'/' {
                if b.get(j + 1) != Some(&b'>') {
                    return Err("bad empty tag".into());
                }
                out.push(Event::Start {
                    name,
                    attrs,
                    empty: true,
                });
                j += 2;
                break;
            }
            let k0 = j;
            while j < b.len() && b[j] != b'=' && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                j += 1;
            }
            let key = &text[k0..j];
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if b.get(j) != Some(&b'=') {
                // Attribute without a value (not XML; tolerated).
                attrs.push((key, String::new()));
                continue;
            }
            j += 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let q = *b.get(j).ok_or("unterminated attribute")?;
            if q != b'"' && q != b'\'' {
                return Err("unquoted attribute".into());
            }
            let v0 = j + 1;
            let len = text[v0..]
                .find(q as char)
                .ok_or("unterminated attribute value")?;
            attrs.push((key, unescape(&text[v0..v0 + len])));
            j = v0 + len + 1;
        }
        i = j;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_attributes() {
        let x = "<?xml version=\"1.0\"?><!-- c --><m:model unit='inch' a=\"x&amp;y&#65;\">\n<vertex x=\"1\" y = \"2\"/></m:model>";
        let ev = events(x).unwrap();
        assert_eq!(ev.len(), 3);
        assert_eq!(ev[0].attr("unit"), Some("inch"));
        assert_eq!(ev[0].attr("a"), Some("x&yA"));
        assert!(matches!(
            ev[1],
            Event::Start {
                name: "vertex",
                empty: true,
                ..
            }
        ));
        assert_eq!(ev[1].attr("y"), Some("2"));
        assert_eq!(ev[2], Event::End("m:model"));
        assert_eq!(local("m:color"), "color");
        assert!(events("<a b=\"1").is_err());
    }
}
