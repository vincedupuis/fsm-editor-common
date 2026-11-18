//! A minimal XML reader, enough for XMI documents (the files of earlier versions): elements, attributes, text, CDATA,
//! comments, processing instructions and namespaces. Element and attribute names are rewritten to canonical prefixes
//! for known namespace URIs, so documents using other prefixes read the same.

use std::collections::HashMap;

use fsm_cli::json;
use fsm_cli::model::js_space;

#[derive(Debug, Clone, PartialEq)]
pub struct XmlElement {
    /// Qualified name using the canonical prefix when the namespace is known, e.g. `uml:Model`.
    pub name: String,
    /// Namespace URI of the element ('' when none).
    pub ns: String,
    /// Local part of the name.
    pub local: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlElement>,
    pub text: String,
    pub line: usize,
}

impl XmlElement {
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
}

fn set(attrs: &mut Vec<(String, String)>, k: String, v: String) {
    match attrs.iter_mut().find(|(n, _)| *n == k) {
        Some(a) => a.1 = v,
        None => attrs.push((k, v)),
    }
}

fn xml_error(message: &str, line: usize) -> String {
    format!("Line {line}: {message}")
}

fn entity(e: &str) -> Option<char> {
    Some(match e {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "quot" => '"',
        "apos" => '\'',
        _ => return None,
    })
}

/// Replaces the character and entity references of a text.
fn decode(s: &[char], line: usize) -> Result<String, String> {
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        if s[i] == '&' {
            // &(#x[0-9a-fA-F]+|#[0-9]+|[a-zA-Z]+);
            let mut j = i + 1;
            let kind;
            if s.get(j) == Some(&'#') && s.get(j + 1) == Some(&'x') {
                j += 2;
                kind = 'x';
                while j < s.len() && s[j].is_ascii_hexdigit() {
                    j += 1;
                }
                if j == i + 3 {
                    j = usize::MAX;
                }
            } else if s.get(j) == Some(&'#') {
                j += 1;
                kind = '#';
                while j < s.len() && s[j].is_ascii_digit() {
                    j += 1;
                }
                if j == i + 2 {
                    j = usize::MAX;
                }
            } else {
                kind = 'a';
                while j < s.len() && s[j].is_ascii_alphabetic() {
                    j += 1;
                }
                if j == i + 1 {
                    j = usize::MAX;
                }
            }
            if j != usize::MAX && s.get(j) == Some(&';') {
                let body: String = s[i + 1..j].iter().collect();
                match kind {
                    'x' | '#' => {
                        let code = if kind == 'x' {
                            body[2..].chars().fold(0f64, |n, c| n * 16.0 + c.to_digit(16).unwrap() as f64)
                        } else {
                            body[1..].chars().fold(0f64, |n, c| n * 10.0 + c.to_digit(10).unwrap() as f64)
                        };
                        if code > 0x10ffff as f64 {
                            return Err(format!("Invalid code point {}", json::number(code)));
                        }
                        out.push(char::from_u32(code as u32).unwrap_or('\u{fffd}'));
                    }
                    _ => match entity(&body) {
                        Some(c) => out.push(c),
                        None => return Err(xml_error(&format!("Unknown entity &{body};."), line)),
                    },
                }
                i = j + 1;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    Ok(out)
}

struct Raw {
    qname: String,
    attrs: Vec<(String, String)>,
    children: Vec<Raw>,
    text: String,
    line: usize,
}

struct Parser {
    t: Vec<char>,
    i: usize,
    line: usize,
}

impl Parser {
    fn advance(&mut self, to: usize) {
        for k in self.i..to.min(self.t.len()) {
            if self.t[k] == '\n' {
                self.line += 1;
            }
        }
        self.i = to;
    }

    fn starts(&self, s: &str) -> bool {
        (self.i..).zip(s.chars()).all(|(k, c)| self.t.get(k) == Some(&c))
    }

    fn find(&self, pat: &str, from: usize) -> Option<usize> {
        let p: Vec<char> = pat.chars().collect();
        if p.len() > self.t.len() {
            return None;
        }
        (from..=self.t.len() - p.len()).find(|&k| self.t[k..k + p.len()] == p[..])
    }

    fn expect(&mut self, s: &str) -> Result<(), String> {
        if !self.starts(s) {
            return Err(xml_error(&format!("Expected '{s}'."), self.line));
        }
        self.advance(self.i + s.chars().count());
        Ok(())
    }

    fn skip_until(&mut self, end: &str, what: &str) -> Result<Vec<char>, String> {
        let Some(j) = self.find(end, self.i) else {
            return Err(xml_error(&format!("Unterminated {what}."), self.line));
        };
        let content = self.t[self.i..j].to_vec();
        self.advance(j + end.chars().count());
        Ok(content)
    }

    fn skip_space(&mut self) {
        let mut j = self.i;
        while j < self.t.len() && js_space(self.t[j]) {
            j += 1;
        }
        self.advance(j);
    }

    fn name_at(&mut self) -> Result<String, String> {
        let first = self.t.get(self.i);
        if !first.is_some_and(|c| c.is_ascii_alphabetic() || *c == '_') {
            return Err(xml_error("Expected a name.", self.line));
        }
        let limit = (self.i + 256).min(self.t.len());
        let mut j = self.i + 1;
        while j < limit && (self.t[j].is_ascii_alphanumeric() || matches!(self.t[j], '_' | '.' | '-' | ':')) {
            j += 1;
        }
        let name: String = self.t[self.i..j].iter().collect();
        self.advance(j);
        Ok(name)
    }

    fn element(&mut self) -> Result<Raw, String> {
        let start = self.line;
        self.expect("<")?;
        let qname = self.name_at()?;
        let mut attrs = Vec::new();
        loop {
            self.skip_space();
            if self.starts("/>") {
                self.advance(self.i + 2);
                return Ok(Raw { qname, attrs, children: Vec::new(), text: String::new(), line: start });
            }
            if self.t.get(self.i) == Some(&'>') {
                self.advance(self.i + 1);
                break;
            }
            let an = self.name_at()?;
            self.skip_space();
            self.expect("=")?;
            self.skip_space();
            let q = self.t.get(self.i).copied();
            if q != Some('"') && q != Some('\'') {
                return Err(xml_error(&format!("Attribute {an} must be quoted."), self.line));
            }
            self.advance(self.i + 1);
            let at = self.line;
            let raw = self.skip_until(&q.unwrap().to_string(), &format!("attribute {an}"))?;
            if raw.contains(&'<') {
                return Err(xml_error(&format!("'<' is not allowed in attribute {an}."), at));
            }
            attrs.push((an, decode(&raw, at)?));
        }
        let mut children = Vec::new();
        let mut content = String::new();
        loop {
            if self.i >= self.t.len() {
                return Err(xml_error(&format!("Element <{qname}> is not closed."), start));
            }
            if self.starts("</") {
                self.advance(self.i + 2);
                let close = self.name_at()?;
                if close != qname {
                    return Err(xml_error(&format!("Expected </{qname}>, found </{close}>."), self.line));
                }
                self.skip_space();
                self.expect(">")?;
                return Ok(Raw { qname, attrs, children, text: content, line: start });
            }
            if self.starts("<!--") {
                self.advance(self.i + 4);
                self.skip_until("-->", "comment")?;
            } else if self.starts("<![CDATA[") {
                self.advance(self.i + 9);
                content.extend(self.skip_until("]]>", "CDATA section")?);
            } else if self.starts("<?") {
                self.advance(self.i + 2);
                self.skip_until("?>", "processing instruction")?;
            } else if self.t[self.i] == '<' {
                children.push(self.element()?);
            } else {
                let j = self.find("<", self.i).unwrap_or(self.t.len());
                let at = self.line;
                let chunk = self.t[self.i..j].to_vec();
                self.advance(j);
                content.push_str(&decode(&chunk, at)?);
            }
        }
    }
}

fn resolve(raw: &Raw, scope: &HashMap<String, String>, canonical: &HashMap<&str, &str>) -> XmlElement {
    let mut s = scope.clone();
    for (k, v) in &raw.attrs {
        if k == "xmlns" {
            s.insert(String::new(), v.clone());
        } else if let Some(p) = k.strip_prefix("xmlns:") {
            s.insert(p.to_string(), v.clone());
        }
    }
    let qualify = |qname: &str, is_attr: bool| -> (String, String, String) {
        let (prefix, local) = match qname.find(':') {
            Some(c) => (&qname[..c], &qname[c + 1..]),
            None => ("", qname),
        };
        if is_attr && !qname.contains(':') {
            return (local.to_string(), String::new(), local.to_string());
        }
        let ns = s.get(prefix).cloned().unwrap_or_default();
        let p = canonical.get(ns.as_str()).copied().unwrap_or(prefix);
        let name = if p.is_empty() { local.to_string() } else { format!("{p}:{local}") };
        (name, ns, local.to_string())
    };
    let (name, ns, local) = qualify(&raw.qname, false);
    let mut attrs: Vec<(String, String)> = Vec::new();
    for (k, v) in &raw.attrs {
        if k == "xmlns" || k.starts_with("xmlns:") {
            continue;
        }
        set(&mut attrs, qualify(k, true).0, v.clone());
    }
    // Qualified values (xmi:type="uml:State") follow the same prefix mapping.
    for a in attrs.iter_mut() {
        if a.0.ends_with(":type") {
            a.1 = qualify(&a.1, false).0;
        }
    }
    XmlElement {
        name,
        ns,
        local,
        attrs,
        children: raw.children.iter().map(|c| resolve(c, &s, canonical)).collect(),
        text: raw.text.clone(),
        line: raw.line,
    }
}

/// Parses `text` and returns the document element. `canonical` maps namespace URIs to the prefix used in the returned
/// names. The error is the message to show.
pub fn parse_xml(text: &str, canonical: &HashMap<&str, &str>) -> Result<XmlElement, String> {
    // Line ends as XML 1.0 (2.11) has them read: \r\n and a lone \r become \n.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut p = Parser { t: text.chars().collect(), i: 0, line: 1 };
    // Prolog
    loop {
        p.skip_space();
        if p.starts("<?") {
            p.advance(p.i + 2);
            p.skip_until("?>", "processing instruction")?;
        } else if p.starts("<!--") {
            p.advance(p.i + 4);
            p.skip_until("-->", "comment")?;
        } else if p.starts("<!DOCTYPE") {
            p.skip_until(">", "DOCTYPE")?;
        } else {
            break;
        }
    }
    if p.t.get(p.i) != Some(&'<') {
        return Err(xml_error("The document does not start with an element.", p.line));
    }
    let root = p.element()?;
    p.skip_space();
    while p.starts("<!--") {
        p.advance(p.i + 4);
        p.skip_until("-->", "comment")?;
        p.skip_space();
    }
    if p.i < p.t.len() {
        return Err(xml_error("Content after the document element.", p.line));
    }
    Ok(resolve(&root, &HashMap::new(), canonical))
}
