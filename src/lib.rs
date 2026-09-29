use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    pub directive: bool,
}

fn ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

const PUNCT: &[&str] = &[
    "%:%:", ">>=", "<<=", "...", "->", "++", "--", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||",
    "*=", "/=", "%=", "+=", "-=", "&=", "^=", "|=", "##", "<:", ":>", "<%", "%>", "%:",
];

/// Tokenize C preprocessing tokens; preserve directives as opaque logical lines.
/// Unsupported lexical constructs are rejected instead of guessed at.
pub fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    if !source.is_ascii() {
        return Err("non-ASCII source is unsupported (use generated C escapes)".into());
    }
    if source
        .as_bytes()
        .windows(3)
        .any(|w| w[0..2] == *b"??" && b"=/'()!<>-".contains(&w[2]))
    {
        return Err("C trigraphs are unsupported".into());
    }
    let source = source.replace("\\\r\n", "").replace("\\\n", "");
    let b = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line_start = true;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            if b[i] == b'\n' {
                line_start = true;
            }
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            let end = source[i + 2..].find("*/").ok_or("unterminated comment")? + i + 4;
            if b[i..end].contains(&b'\n') {
                line_start = true;
            }
            i = end;
            continue;
        }
        let start = i;
        if line_start && b[i] == b'#' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            // Block comments crossing directive lines would need a full directive parser.
            let text = &source[start..i];
            if text.matches("/*").count() != text.matches("*/").count() {
                return Err("multiline comments inside directives are unsupported".into());
            }
            out.push(Token {
                text: text.trim_end().into(),
                directive: true,
            });
            continue;
        }
        line_start = false;
        let prefix = ["u8", "u", "U", "L"].iter().find(|p| {
            source[i..].starts_with(**p)
                && b.get(i + p.len())
                    .is_some_and(|c| *c == b'\'' || *c == b'"')
        });
        let quote_at = i + prefix.map_or(0, |p| p.len());
        if b[quote_at] == b'\'' || b[quote_at] == b'"' {
            let quote = b[quote_at];
            i = quote_at + 1;
            loop {
                if i >= b.len() || b[i] == b'\n' {
                    return Err("unterminated literal".into());
                }
                if b[i] == quote {
                    i += 1;
                    break;
                }
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
        } else if b[i].is_ascii_digit()
            || (b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            i += 1;
            while i < b.len()
                && (ident(b[i])
                    || b[i] == b'.'
                    || ((b[i] == b'+' || b[i] == b'-') && b"eEpP".contains(&b[i - 1])))
            {
                i += 1;
            }
        } else if ident(b[i]) {
            i += 1;
            while i < b.len() && ident(b[i]) {
                i += 1;
            }
        } else if let Some(p) = PUNCT.iter().find(|p| source[i..].starts_with(**p)) {
            i += p.len();
        } else {
            if b[i] == b'\\' {
                return Err("universal character names are unsupported".into());
            }
            i += 1;
        }
        out.push(Token {
            text: source[start..i].into(),
            directive: false,
        });
    }
    Ok(out)
}

fn space(a: &str, b: &str) -> bool {
    let x = *a.as_bytes().last().unwrap();
    let y = b.as_bytes()[0];
    (ident(x) && ident(y))
        || (ident(x) && (y == b'\'' || y == b'"'))
        || (x == b'.' && (y == b'.' || y.is_ascii_digit()))
        || ((a.as_bytes()[0].is_ascii_digit()
            || (a.starts_with('.') && a.as_bytes().get(1).is_some_and(u8::is_ascii_digit)))
            && (y == b'.' || y == b'+' || y == b'-'))
        || (x == b'/' && (y == b'/' || y == b'*'))
        || (x == b'?' && y == b'?')
        || PUNCT
            .iter()
            .any(|p| p.len() > 1 && p.as_bytes()[0] == x && p.as_bytes()[1] == y)
}

pub fn render(tokens: &[Token]) -> String {
    let mut out = String::new();
    let mut previous = "";
    let mut column = 0;
    for t in tokens {
        if t.directive {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&t.text);
            out.push('\n');
            previous = "";
            column = 0;
        } else {
            // Stay below C's minimum supported logical source line length.
            if column + t.text.len() + 1 > 4000 {
                out.push('\n');
                previous = "";
                column = 0;
            }
            if !previous.is_empty() && space(previous, &t.text) {
                out.push(' ');
                column += 1;
            }
            out.push_str(&t.text);
            column += t.text.len();
            previous = &t.text;
        }
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[derive(Debug)]
pub struct Compressed {
    pub source: String,
    pub whitespace_bytes: usize,
    pub macros: usize,
}

fn name(mut n: usize) -> String {
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut s = String::new();
    loop {
        s.push(LETTERS[n % LETTERS.len()] as char);
        n /= LETTERS.len();
        if n == 0 {
            return s;
        }
        n -= 1;
    }
}

/// Compress only by reversible object-like macros. `reserved` must include the
/// compiler's expanded source and macro definitions, including header bodies.
pub fn compress(source: &str, reserved: &str, rounds: usize) -> Result<Compressed, String> {
    let tokens = tokenize(source)?;
    let whitespace_bytes = render(&tokens).len();
    let mut occupied: HashSet<String> = source
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .chain(reserved.split(|c: char| !c.is_ascii_alphanumeric() && c != '_'))
        .map(str::to_owned)
        .collect();
    let mut vocabulary = Vec::<Token>::new();
    let mut intern = HashMap::<(String, bool), u32>::new();
    let mut body = Vec::new();
    for t in &tokens {
        let next = vocabulary.len() as u32;
        let id = *intern
            .entry((t.text.clone(), t.directive))
            .or_insert_with(|| {
                vocabulary.push(t.clone());
                next
            });
        body.push(id);
    }
    let mut definitions: Vec<(String, Vec<u32>)> = Vec::new();
    let mut serial = 0;
    for _ in 0..rounds {
        let mut counts: HashMap<Vec<u32>, (usize, usize)> = HashMap::new();
        for start in 0..body.len() {
            // Replacing a call's argument parentheses hides them from its macro.
            if vocabulary[body[start] as usize].text == "(" {
                continue;
            }
            let mut stack = Vec::new();
            for end in start..body.len().min(start + 24) {
                let token = &vocabulary[body[end] as usize];
                if token.directive {
                    break;
                }
                match token.text.as_str() {
                    "#" | "##" | "%:" | "%:%:" => break,
                    "(" | "[" | "{" => stack.push(token.text.as_str()),
                    ")" | "]" | "}" => {
                        let expected = match token.text.as_str() {
                            ")" => "(",
                            "]" => "[",
                            _ => "{",
                        };
                        if stack.pop() != Some(expected) {
                            break;
                        }
                    }
                    "," if !stack.contains(&"(") => break,
                    _ => (),
                }
                if !stack.is_empty() {
                    continue;
                }
                // Include a function-like invocation as a unit, not its prefix.
                if end + 1 < body.len() && vocabulary[body[end + 1] as usize].text == "(" {
                    continue;
                }
                let seq = &body[start..=end];
                let entry = counts.entry(seq.to_vec()).or_insert((0, 0));
                if start >= entry.1 {
                    entry.0 += 1;
                    entry.1 = end + 1;
                }
            }
        }
        let mut candidates: Vec<_> = counts
            .into_iter()
            .filter_map(|(seq, (count, _))| {
                if count < 2 {
                    return None;
                }
                let ts: Vec<_> = seq
                    .iter()
                    .map(|&id| vocabulary[id as usize].clone())
                    .collect();
                let len = render(&ts).trim_end().len();
                let savings = (count as isize - 1) * len as isize - count as isize * 4 - 26;
                (savings > 0).then_some((savings, seq))
            })
            .collect();
        candidates.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let mut changed = false;
        // Recount after each replacement, so overlapping candidates never grow output.
        for (_, seq) in candidates.into_iter().take(256) {
            let mut count = 0;
            let mut i = 0;
            while i + seq.len() <= body.len() {
                if body[i..i + seq.len()] == seq {
                    count += 1;
                    i += seq.len();
                } else {
                    i += 1;
                }
            }
            if count < 2 {
                continue;
            }
            let alias = loop {
                let s = name(serial);
                if !occupied.contains(&s) {
                    break s;
                }
                serial += 1;
            };
            let ts: Vec<_> = seq
                .iter()
                .map(|&id| vocabulary[id as usize].clone())
                .collect();
            let len = render(&ts).trim_end().len();
            let cost = len + 2 * alias.len() + 19; // #define and #undef
            if count * len <= count * (alias.len() + 2) + cost {
                continue;
            }
            occupied.insert(alias.clone());
            serial += 1;
            let id = vocabulary.len() as u32;
            vocabulary.push(Token {
                text: alias.clone(),
                directive: false,
            });
            definitions.push((alias, seq.clone()));
            let mut replaced = Vec::with_capacity(body.len());
            i = 0;
            while i < body.len() {
                if body[i..].starts_with(&seq) {
                    replaced.push(id);
                    i += seq.len();
                } else {
                    replaced.push(body[i]);
                    i += 1;
                }
            }
            body = replaced;
            changed = true;
        }
        if !changed {
            break;
        }
    }
    let mut output: Vec<Token> = body
        .into_iter()
        .map(|id| vocabulary[id as usize].clone())
        .collect();
    // Headers and initial configuration stay ahead of our macros.
    let insert = output
        .iter()
        .position(|t| !t.directive)
        .unwrap_or(output.len());
    let defs = definitions.iter().map(|(alias, seq)| {
        let ts: Vec<_> = seq
            .iter()
            .map(|&id| vocabulary[id as usize].clone())
            .collect();
        Token {
            text: format!("#define {alias} {}", render(&ts).trim_end()),
            directive: true,
        }
    });
    output.splice(insert..insert, defs);
    output.extend(definitions.iter().map(|(alias, _)| Token {
        text: format!("#undef {alias}"),
        directive: true,
    }));
    let compressed = render(&output);
    if compressed.len() >= whitespace_bytes {
        return Ok(Compressed {
            source: render(&tokens),
            whitespace_bytes,
            macros: 0,
        });
    }
    Ok(Compressed {
        source: compressed,
        whitespace_bytes,
        macros: definitions.len(),
    })
}

/// Compare preprocessing tokens, not whitespace. Directives (e.g. pragmas) remain significant.
pub fn equivalent(a: &str, b: &str) -> Result<(), String> {
    let a = tokenize(a)?;
    let b = tokenize(b)?;
    if a == b {
        return Ok(());
    }
    let at = a
        .iter()
        .zip(&b)
        .position(|(a, b)| a != b)
        .unwrap_or(a.len().min(b.len()));
    Err(format!(
        "preprocessor equivalence failed at token {at}: {:?} != {:?}; output was not written",
        a.get(at),
        b.get(at)
    ))
}
