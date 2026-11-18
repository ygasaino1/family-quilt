//! Streaming GEDCOM line parser: bytes -> one record tree at a time.
//! Handles BOM, LF/CRLF/CR line ends, `CONC`/`CONT`, `@@` escapes, size limits, and reports
//! the line number of every syntax problem.

use crate::GedError;
use std::io::BufRead;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_bytes: u64,
    pub max_line_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_bytes: 128 * 1024 * 1024, max_line_bytes: 64 * 1024 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Node {
    pub level: usize,
    pub xref: Option<String>,
    pub tag: String,
    pub value: String,
    pub line: usize,
    pub children: Vec<Node>,
}

impl Node {
    pub fn child(&self, tag: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.tag == tag)
    }
    pub fn all<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.tag == tag)
    }
    pub fn text(&self, tag: &str) -> Option<&str> {
        self.child(tag).map(|c| c.value.trim()).filter(|v| !v.is_empty())
    }
}

/// `@I12@` -> `I12`; anything else is not a pointer.
pub fn pointer(v: &str) -> Option<&str> {
    let v = v.trim();
    (v.len() > 2 && v.starts_with('@') && v.ends_with('@') && !v[1..v.len() - 1].contains('@')).then(|| &v[1..v.len() - 1])
}

fn syntax(line: usize, message: impl Into<String>) -> GedError {
    GedError::Syntax { line, message: message.into() }
}

fn parse_line(text: &str, line: usize) -> Result<Node, GedError> {
    let t = text.trim_start();
    let (lvl, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
    if lvl.is_empty() || lvl.len() > 2 || !lvl.bytes().all(|b| b.is_ascii_digit()) {
        return Err(syntax(line, format!("a line must start with a level number, found '{}'", lvl.chars().take(20).collect::<String>())));
    }
    let level: usize = lvl.parse().unwrap();
    let rest = rest.trim_start();
    let (xref, rest) = if rest.starts_with('@') {
        let (x, r) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let id = pointer(x).ok_or_else(|| syntax(line, format!("bad record id '{x}'")))?;
        (Some(id.to_string()), r.trim_start())
    } else {
        (None, rest)
    };
    let (tag, value) = rest.split_once(' ').unwrap_or((rest, ""));
    if tag.is_empty() || !tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(syntax(line, format!("missing or invalid tag '{}'", tag.chars().take(20).collect::<String>())));
    }
    // "@@" is an escaped "@" (but a lone pointer value like "@I1@" must stay intact)
    let value = if pointer(value).is_some() { value.to_string() } else { value.replace("@@", "@") };
    Ok(Node { level, xref, tag: tag.to_ascii_uppercase(), value, line, children: Vec::new() })
}

/// Read every level-0 record and hand it to `on_record`. Memory use is one record at a time.
pub fn read_records(mut r: impl BufRead, limits: &Limits, mut on_record: impl FnMut(Node) -> Result<(), GedError>) -> Result<(), GedError> {
    let mut stack: Vec<Node> = Vec::new();
    let mut line_no = 0usize;
    let mut total = 0u64;
    let mut buf = Vec::new();
    let mut first = true;
    loop {
        buf.clear();
        let n = std::io::Read::take(&mut r, limits.max_line_bytes as u64 + 1).read_until(b'\n', &mut buf).map_err(GedError::Io)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > limits.max_bytes {
            return Err(GedError::TooBig { limit_bytes: limits.max_bytes });
        }
        if buf.len() > limits.max_line_bytes {
            return Err(syntax(line_no + 1, format!("line is longer than {} bytes", limits.max_line_bytes)));
        }
        if first {
            first = false;
            if buf.starts_with(&[0xEF, 0xBB, 0xBF]) {
                buf.drain(..3);
            } else if buf.starts_with(&[0xFF, 0xFE]) || buf.starts_with(&[0xFE, 0xFF]) {
                return Err(syntax(1, "UTF-16 files are not supported; save the file as UTF-8"));
            }
        }
        // CR-only files arrive as one "line" with embedded CRs
        let pieces: Vec<&[u8]> = buf.split(|b| *b == b'\r' || *b == b'\n').filter(|p| !p.iter().all(u8::is_ascii_whitespace)).collect();
        if buf.iter().all(u8::is_ascii_whitespace) {
            line_no += 1;
            continue;
        }
        for piece in pieces {
            line_no += 1;
            let text = std::str::from_utf8(piece).map_err(|_| syntax(line_no, "the text is not valid UTF-8 (ANSEL and other old encodings must be converted first)"))?;
            let node = parse_line(text, line_no)?;
            while stack.len() > node.level {
                let done = stack.pop().unwrap();
                match stack.last_mut() {
                    Some(p) => p.children.push(done),
                    None => on_record(done)?,
                }
            }
            if node.level != stack.len() {
                return Err(syntax(line_no, format!("level {} follows level {}: levels may only go one deeper at a time", node.level, stack.len().saturating_sub(1))));
            }
            if node.level > 0 && (node.tag == "CONC" || node.tag == "CONT") {
                let parent = stack.last_mut().unwrap();
                if node.tag == "CONT" {
                    parent.value.push('\n');
                }
                parent.value.push_str(&node.value);
                continue;
            }
            stack.push(node);
        }
    }
    while let Some(done) = stack.pop() {
        match stack.last_mut() {
            Some(p) => p.children.push(done),
            None => on_record(done)?,
        }
    }
    Ok(())
}
