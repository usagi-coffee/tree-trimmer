use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

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
    let source = prepare(source)?;
    let mut lexer = Lexer::new(&source);
    let mut out = Vec::new();
    while let Some(t) = lexer.next()? {
        out.push(Token {
            text: t.text.into(),
            directive: t.directive,
        });
    }
    Ok(out)
}

fn prepare(source: &str) -> Result<Cow<'_, str>, String> {
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
    if source.contains("\\\n") || source.contains("\\\r\n") {
        Ok(Cow::Owned(source.replace("\\\r\n", "").replace("\\\n", "")))
    } else {
        Ok(Cow::Borrowed(source))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct TokenRef<'a> {
    text: &'a str,
    directive: bool,
}

struct Lexer<'a> {
    source: &'a str,
    offset: usize,
    line_start: bool,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            offset: 0,
            line_start: true,
        }
    }

    fn next(&mut self) -> Result<Option<TokenRef<'a>>, String> {
        let source = self.source;
        let b = source.as_bytes();
        let mut i = self.offset;
        let mut line_start = self.line_start;
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
                self.offset = i;
                self.line_start = line_start;
                return Ok(Some(TokenRef {
                    text: text.trim_end(),
                    directive: true,
                }));
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
            self.offset = i;
            self.line_start = line_start;
            return Ok(Some(TokenRef {
                text: &source[start..i],
                directive: false,
            }));
        }
        self.offset = i;
        Ok(None)
    }
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
    render_iter(tokens.iter())
}

fn render_iter<'a>(tokens: impl IntoIterator<Item = &'a Token>) -> String {
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
    compress_with_reserved(source, &[reserved], rounds)
}

/// Accept reserved text in separate buffers so callers need not concatenate
/// potentially hundreds of megabytes of preprocessed C.
pub fn compress_with_reserved(
    source: &str,
    reserved: &[&str],
    rounds: usize,
) -> Result<Compressed, String> {
    let source = prepare(source)?;
    let occupied: HashSet<&str> = std::iter::once(source.as_ref())
        .chain(reserved.iter().copied())
        .flat_map(|s| s.split(|c: char| !c.is_ascii_alphanumeric() && c != '_'))
        .filter(|s| !s.is_empty())
        .collect();
    let mut vocabulary = Vec::<Token>::new();
    let mut intern = HashMap::<(&str, bool), u32>::new();
    let mut body = Vec::new();
    let mut lexer = Lexer::new(&source);
    while let Some(t) = lexer.next()? {
        let next = vocabulary.len() as u32;
        let id = *intern.entry((t.text, t.directive)).or_insert_with(|| {
            vocabulary.push(Token {
                text: t.text.into(),
                directive: t.directive,
            });
            next
        });
        body.push(id);
    }
    drop(intern);
    let whitespace = render_iter(body.iter().map(|&id| &vocabulary[id as usize]));
    let whitespace_bytes = whitespace.len();
    let mut definitions: Vec<(String, Vec<u32>)> = Vec::new();
    let mut serial = 0;
    for _ in 0..rounds {
        let candidates = candidates(&body, &vocabulary);
        if candidates.is_empty() {
            break;
        }
        let changed = replace_candidates(
            &mut body,
            &mut vocabulary,
            candidates,
            &occupied,
            &mut serial,
            &mut definitions,
        );
        if !changed {
            break;
        }
    }
    let insert = body
        .iter()
        .position(|&id| !vocabulary[id as usize].directive)
        .unwrap_or(body.len());
    let defs: Vec<_> = definitions
        .iter()
        .map(|(alias, seq)| Token {
            text: format!(
                "#define {alias} {}",
                render_iter(seq.iter().map(|&id| &vocabulary[id as usize])).trim_end()
            ),
            directive: true,
        })
        .collect();
    let undefs: Vec<_> = definitions
        .iter()
        .map(|(alias, _)| Token {
            text: format!("#undef {alias}"),
            directive: true,
        })
        .collect();
    let compressed = render_iter(
        body[..insert]
            .iter()
            .map(|&id| &vocabulary[id as usize])
            .chain(defs.iter())
            .chain(body[insert..].iter().map(|&id| &vocabulary[id as usize]))
            .chain(undefs.iter()),
    );
    if compressed.len() >= whitespace_bytes {
        return Ok(Compressed {
            source: whitespace,
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

fn replace_candidates(
    body: &mut Vec<u32>,
    vocabulary: &mut Vec<Token>,
    candidates: Vec<Vec<u32>>,
    occupied: &HashSet<&str>,
    serial: &mut usize,
    definitions: &mut Vec<(String, Vec<u32>)>,
) -> bool {
    let mut frequencies = vec![0usize; vocabulary.len()];
    for &id in body.iter() {
        frequencies[id as usize] += 1;
    }
    // Index the rarest token of each candidate, not necessarily its first.
    // Most generated table entries start with the same punctuation.
    let anchors: Vec<usize> = candidates
        .iter()
        .map(|seq| {
            (0..seq.len())
                .min_by_key(|&i| frequencies[seq[i] as usize])
                .unwrap()
        })
        .collect();
    let mut wanted = vec![false; vocabulary.len()];
    for (seq, &anchor) in candidates.iter().zip(&anchors) {
        wanted[seq[anchor] as usize] = true;
    }
    let mut positions: Vec<Vec<usize>> = frequencies
        .iter()
        .zip(&wanted)
        .map(|(&count, &want)| {
            if want {
                Vec::with_capacity(count)
            } else {
                Vec::new()
            }
        })
        .collect();
    for (pos, &id) in body.iter().enumerate() {
        if wanted[id as usize] {
            positions[id as usize].push(pos);
        }
    }
    let mut matches = Vec::new();
    let mut changed = false;
    for (seq, anchor) in candidates.into_iter().zip(anchors) {
        matches.clear();
        let mut end = 0;
        for &position in &positions[seq[anchor] as usize] {
            let Some(start) = position.checked_sub(anchor) else {
                continue;
            };
            if start >= end && body.get(start..start + seq.len()) == Some(seq.as_slice()) {
                matches.push(start);
                end = start + seq.len();
            }
        }
        let count = matches.len();
        if count < 2 {
            continue;
        }
        let alias = loop {
            let s = name(*serial);
            if !occupied.contains(s.as_str()) {
                break s;
            }
            *serial += 1;
        };
        let len = sequence_len(&seq, vocabulary);
        let cost = len + 2 * alias.len() + 19;
        if count * len <= count * (alias.len() + 2) + cost {
            continue;
        }
        // Names are allocated monotonically, so a generated name cannot recur.
        *serial += 1;
        let id = vocabulary.len() as u32;
        vocabulary.push(Token {
            text: alias.clone(),
            directive: false,
        });
        for &start in &matches {
            body[start] = id;
            body[start + 1..start + seq.len()].fill(u32::MAX);
        }
        definitions.push((alias, seq));
        changed = true;
    }
    // A fresh alias never appears in this round's candidates. Therefore
    // replacing a match cannot create new matches this round. Tombstones
    // keep every index valid until the single compaction below.
    body.retain(|&id| id != u32::MAX);
    changed
}

fn sequence_len(seq: &[u32], vocabulary: &[Token]) -> usize {
    // Candidate sequences contain no directives and at most 24 tokens.
    // Use the same renderer, but borrow tokens instead of cloning their strings.
    render_iter(seq.iter().map(|&id| &vocabulary[id as usize]))
        .trim_end()
        .len()
}

fn candidates(body: &[u32], vocabulary: &[Token]) -> Vec<Vec<u32>> {
    let mut counts: HashMap<&[u32], (usize, usize)> = HashMap::new();
    for start in 0..body.len() {
        if vocabulary[body[start] as usize].text == "(" {
            continue;
        }
        let mut stack = [""; 24];
        let mut depth = 0;
        let mut parentheses = 0;
        for end in start..body.len().min(start + 24) {
            let token = &vocabulary[body[end] as usize];
            if token.directive {
                break;
            }
            match token.text.as_str() {
                "#" | "##" | "%:" | "%:%:" => break,
                "(" | "[" | "{" => {
                    stack[depth] = token.text.as_str();
                    depth += 1;
                    if token.text == "(" {
                        parentheses += 1;
                    }
                }
                ")" | "]" | "}" => {
                    let expected = match token.text.as_str() {
                        ")" => "(",
                        "]" => "[",
                        _ => "{",
                    };
                    if depth == 0 || stack[depth - 1] != expected {
                        break;
                    }
                    depth -= 1;
                    if expected == "(" {
                        parentheses -= 1;
                    }
                }
                "," if parentheses == 0 => break,
                _ => (),
            }
            if depth != 0 {
                continue;
            }
            if end + 1 < body.len() && vocabulary[body[end + 1] as usize].text == "(" {
                continue;
            }
            let entry = counts.entry(&body[start..=end]).or_insert((0, 0));
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
            let len = sequence_len(seq, vocabulary);
            let savings = (count as isize - 1) * len as isize - count as isize * 4 - 26;
            (savings > 0).then_some((savings, seq))
        })
        .collect();
    let order = |a: &(isize, &[u32]), b: &(isize, &[u32])| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1));
    if candidates.len() > 256 {
        candidates.select_nth_unstable_by(256, order);
        candidates.truncate(256);
    }
    candidates.sort_unstable_by(order);
    candidates
        .into_iter()
        .map(|(_, seq)| seq.to_vec())
        .collect()
}

/// Compare every preprocessing token without allocating token strings or vectors.
/// Directives (e.g. pragmas) remain significant.
pub fn equivalent(a: &str, b: &str) -> Result<(), String> {
    let a = prepare(a)?;
    let b = prepare(b)?;
    let mut a = Lexer::new(&a);
    let mut b = Lexer::new(&b);
    let mut at = 0;
    loop {
        let left = a.next()?;
        let right = b.next()?;
        if left != right {
            return Err(format!(
                "preprocessor equivalence failed at token {at}: {left:?} != {right:?}; output was not written"
            ));
        }
        if left.is_none() {
            return Ok(());
        }
        at += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_replacement_matches_full_scan_with_overlapping_candidates() {
        let mut random = 42u32;
        for _ in 0..24 {
            let mut body = Vec::new();
            for _ in 0..600 {
                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                body.extend([0, 1, 0, 1, 0, (random >> 16) % 8]);
            }
            let mut vocabulary: Vec<_> = (0..8)
                .map(|n| Token {
                    text: format!("long_generated_symbol_identifier_{n}"),
                    directive: false,
                })
                .collect();
            let mut candidates = vec![vec![0, 1, 0], vec![0, 1], vec![1, 0, 1, 0]];
            for _ in 0..80 {
                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                let start = random as usize % (body.len() - 6);
                let len = (random >> 16) as usize % 6 + 1;
                candidates.push(body[start..start + len].to_vec());
            }
            // Vary candidate order to exercise different overlap and anchor choices.
            let rotate = random as usize % candidates.len();
            candidates.rotate_left(rotate);
            let mut expected = body.clone();
            let mut expected_vocabulary = vocabulary.clone();
            let mut expected_definitions = Vec::new();
            let mut serial = 0;
            let occupied: HashSet<&str> = ["a", "c", "f"].into_iter().collect();
            for seq in &candidates {
                let mut count = 0;
                let mut i = 0;
                while i < expected.len() {
                    if expected[i..].starts_with(seq) {
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
                    let alias = name(serial);
                    if !occupied.contains(alias.as_str()) {
                        break alias;
                    }
                    serial += 1;
                };
                let len = sequence_len(seq, &expected_vocabulary);
                if count * len <= count * (alias.len() + 2) + len + 2 * alias.len() + 19 {
                    continue;
                }
                serial += 1;
                let id = expected_vocabulary.len() as u32;
                expected_vocabulary.push(Token {
                    text: alias.clone(),
                    directive: false,
                });
                expected_definitions.push((alias, seq.clone()));
                let mut next = Vec::new();
                i = 0;
                while i < expected.len() {
                    if expected[i..].starts_with(seq) {
                        next.push(id);
                        i += seq.len();
                    } else {
                        next.push(expected[i]);
                        i += 1;
                    }
                }
                expected = next;
            }
            let mut definitions = Vec::new();
            let mut actual_serial = 0;
            replace_candidates(
                &mut body,
                &mut vocabulary,
                candidates,
                &occupied,
                &mut actual_serial,
                &mut definitions,
            );
            assert_eq!(body, expected);
            assert_eq!(vocabulary, expected_vocabulary);
            assert_eq!(definitions, expected_definitions);
            assert_eq!(actual_serial, serial);
        }
    }
}
