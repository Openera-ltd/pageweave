//! Minimal content-stream tokenizer that keeps byte positions, so a stream can be edited by
//! dropping or replacing whole operations while every other byte (inline image data included) is
//! copied verbatim. lopdf's decode/encode round trip cannot do that: it rewrites inline images as
//! stream objects and silently skips ones it cannot parse.

/// One operation: operand tokens followed by the operator, spanning `start..end` of the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawOp<'a> {
    pub start: usize,
    pub end: usize,
    pub operator: &'a [u8],
    pub operands: Vec<&'a [u8]>,
}

/// Why a stream could not be tokenized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unreadable;

/// Nesting limit for arrays/dictionaries in operands (hostile files).
const MAX_DEPTH: usize = 64;

fn is_space(c: u8) -> bool {
    matches!(c, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

fn is_delim(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

struct Lexer<'a> {
    d: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn skip_space(&mut self) {
        while self.i < self.d.len() {
            let c = self.d[self.i];
            if is_space(c) {
                self.i += 1;
            } else if c == b'%' {
                while self.i < self.d.len() && !matches!(self.d[self.i], b'\r' | b'\n') {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }

    fn regular(&mut self) {
        while self.i < self.d.len() && !is_space(self.d[self.i]) && !is_delim(self.d[self.i]) {
            self.i += 1;
        }
    }

    fn literal_string(&mut self) -> Result<(), Unreadable> {
        // At '('.
        self.i += 1;
        let mut depth = 1usize;
        while self.i < self.d.len() {
            match self.d[self.i] {
                b'\\' => self.i += 2,
                b'(' => {
                    depth += 1;
                    self.i += 1;
                }
                b')' => {
                    depth -= 1;
                    self.i += 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                _ => self.i += 1,
            }
        }
        Err(Unreadable)
    }

    /// Skip one object starting at the current (non-space) byte.
    fn object(&mut self, depth: usize) -> Result<(), Unreadable> {
        if depth > MAX_DEPTH {
            return Err(Unreadable);
        }
        let c = *self.d.get(self.i).ok_or(Unreadable)?;
        match c {
            b'(' => self.literal_string(),
            b'<' if self.d.get(self.i + 1) == Some(&b'<') => {
                self.i += 2;
                loop {
                    self.skip_space();
                    if self.d.get(self.i..self.i + 2) == Some(b">>") {
                        self.i += 2;
                        return Ok(());
                    }
                    if self.i >= self.d.len() {
                        return Err(Unreadable);
                    }
                    self.object(depth + 1)?;
                }
            }
            b'<' => {
                let end = self.d[self.i..].iter().position(|&b| b == b'>').ok_or(Unreadable)?;
                self.i += end + 1;
                Ok(())
            }
            b'[' => {
                self.i += 1;
                loop {
                    self.skip_space();
                    match self.d.get(self.i) {
                        Some(b']') => {
                            self.i += 1;
                            return Ok(());
                        }
                        None => return Err(Unreadable),
                        _ => self.object(depth + 1)?,
                    }
                }
            }
            b'/' => {
                self.i += 1;
                self.regular();
                Ok(())
            }
            b')' | b'>' | b']' => Err(Unreadable),
            b'{' | b'}' => {
                self.i += 1;
                Ok(())
            }
            _ => {
                self.regular();
                Ok(())
            }
        }
    }
}

fn is_operand_start(c: u8, word: &[u8]) -> bool {
    matches!(c, b'(' | b'<' | b'[' | b'/' | b'{' | b'}' | b'+' | b'-' | b'.' | b'0'..=b'9')
        || matches!(word, b"true" | b"false" | b"null")
}

/// End of inline image data that starts at `from`: the "EI" that follows whitespace and is
/// followed by whitespace, a delimiter or the end of the stream.
fn inline_image_end(d: &[u8], from: usize) -> Option<usize> {
    let mut j = from;
    while j + 2 <= d.len() {
        if d[j] == b'E'
            && d[j + 1] == b'I'
            && j > from
            && is_space(d[j - 1])
            && d.get(j + 2).is_none_or(|&c| is_space(c) || is_delim(c))
        {
            return Some(j + 2);
        }
        j += 1;
    }
    None
}

/// Split a content stream into operations with byte spans.
pub(crate) fn tokenize(d: &[u8]) -> Result<Vec<RawOp<'_>>, Unreadable> {
    let mut lx = Lexer { d, i: 0 };
    let mut ops = Vec::new();
    let mut operands: Vec<&[u8]> = Vec::new();
    let mut op_start: Option<usize> = None;
    loop {
        lx.skip_space();
        if lx.i >= d.len() {
            break;
        }
        let start = lx.i;
        let c = d[start];
        let word_end = {
            let mut j = start;
            while j < d.len() && !is_space(d[j]) && !is_delim(d[j]) {
                j += 1;
            }
            j
        };
        if is_operand_start(c, &d[start..word_end]) {
            lx.object(0)?;
            operands.push(&d[start..lx.i]);
            op_start.get_or_insert(start);
            continue;
        }
        if is_delim(c) {
            return Err(Unreadable);
        }
        lx.i = word_end;
        let operator = &d[start..word_end];
        let mut end = word_end;
        if operator == b"BI" {
            // Key/value pairs up to ID, one whitespace byte, then binary data up to EI.
            loop {
                lx.skip_space();
                if d.get(lx.i..lx.i + 2) == Some(b"ID") && d.get(lx.i + 2).is_none_or(|&c| is_space(c)) {
                    break;
                }
                if lx.i >= d.len() {
                    return Err(Unreadable);
                }
                lx.object(0)?;
            }
            let data = lx.i + 3;
            end = inline_image_end(d, data.min(d.len())).ok_or(Unreadable)?;
            lx.i = end;
        }
        ops.push(RawOp {
            start: op_start.take().unwrap_or(start),
            end,
            operator,
            operands: std::mem::take(&mut operands),
        });
    }
    Ok(ops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_cover_operands_strings_and_inline_images() {
        let src = b"q 1 0 0 1 72 500 cm /OC /L1 BDC BT /F1 12 Tf (a \\) (b) c) Tj [(x) -40 (y)] TJ ET EMC\n\
                    % comment Tj\nBI /W 2 /H 1 /CS /G /BPC 8 ID \x01EI EI Q <</A [1 2]>> BDC EMC";
        let ops = tokenize(src).expect("ok");
        let names: Vec<&[u8]> = ops.iter().map(|o| o.operator).collect();
        assert_eq!(
            names,
            vec![&b"q"[..], b"cm", b"BDC", b"BT", b"Tf", b"Tj", b"TJ", b"ET", b"EMC", b"BI", b"Q", b"BDC", b"EMC"]
        );
        let tj = &ops[5];
        assert_eq!(&src[tj.start..tj.end], b"(a \\) (b) c) Tj");
        let bdc = &ops[2];
        assert_eq!(bdc.operands, vec![&b"/OC"[..], b"/L1"]);
        let bi = &ops[9];
        assert!(src[bi.start..bi.end].starts_with(b"BI /W 2"));
        assert!(src[bi.start..bi.end].ends_with(b"EI"), "binary data containing 'EI' is kept");
        assert_eq!(ops[11].operands, vec![&b"<</A [1 2]>>"[..]]);
    }

    #[test]
    fn malformed_streams_are_reported() {
        assert_eq!(tokenize(b"BT (unterminated Tj ET"), Err(Unreadable));
        assert_eq!(tokenize(b"BI /W 1 ID xx"), Err(Unreadable));
        assert_eq!(tokenize(b"] Tj"), Err(Unreadable));
        let deep = "[".repeat(100) + &"]".repeat(100) + " Tj";
        assert_eq!(tokenize(deep.as_bytes()), Err(Unreadable));
        assert!(tokenize(b"").expect("empty").is_empty());
    }
}
