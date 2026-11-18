//! `/`-separated paths, the same on every host.

/// The folder of a path.
pub fn dirname(p: &str) -> String {
    match p.rfind('/') {
        Some(i) if i > 0 => p[..i].to_string(),
        _ => "/".to_string(),
    }
}

/// `rel` resolved against the folder `dir` (absolute, `/`-separated), with `.` and `..` segments resolved.
pub fn join_path(dir: &str, rel: &str) -> String {
    let full = if rel.starts_with('/') { rel.to_string() } else { format!("{dir}/{rel}") };
    let mut out: Vec<&str> = Vec::new();
    for s in full.split('/') {
        if s.is_empty() || s == "." {
            continue;
        }
        if s == ".." {
            out.pop();
        } else {
            out.push(s);
        }
    }
    format!("/{}", out.join("/"))
}

/// The relative path from folder `from` to `to`, both absolute `/`-separated paths.
pub fn relative(from: &str, to: &str) -> String {
    let a: Vec<&str> = from.split('/').filter(|s| !s.is_empty()).collect();
    let b: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let mut i = 0;
    while i < a.len() && i < b.len() && a[i] == b[i] {
        i += 1;
    }
    let mut out: Vec<&str> = vec![".."; a.len() - i];
    out.extend(&b[i..]);
    out.join("/")
}

const RESERVED: &str = ";/:@&=+$,";

/// `decodeURI` without its error, for the submachine hrefs of old files: decodes `%XX` escapes (UTF-8), except those
/// of reserved characters (`;/:@&=+$,`; unlike `decodeURI`, `%23` and `%3F` give `#` and `?`), and keeps malformed ones
/// as they are.
pub fn decode_href(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let at = |i: usize| -> i32 {
        if c.get(i) == Some(&'%')
            && let (Some(a), Some(b)) =
                (c.get(i + 1).and_then(|x| x.to_digit(16)), c.get(i + 2).and_then(|x| x.to_digit(16)))
        {
            return (a * 16 + b) as i32;
        }
        -1
    };
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        let b = at(i);
        if b >= 0 {
            let len = match b {
                0..0x80 => 1,
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                _ => 0,
            };
            let mut run = vec![b as u8];
            let mut j = i + 3;
            while run.len() < len && at(j) >= 0 {
                run.push(at(j) as u8);
                j += 3;
            }
            if run.len() == len
                && let Ok(text) = std::str::from_utf8(&run)
            {
                // TextDecoder drops a byte order mark at the start.
                let text = text.strip_prefix('\u{feff}').unwrap_or(text);
                if len == 1 && RESERVED.contains(text) && !text.is_empty() {
                    out.extend(&c[i..j]);
                } else {
                    out.push_str(text);
                }
                i = j;
                continue;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}
