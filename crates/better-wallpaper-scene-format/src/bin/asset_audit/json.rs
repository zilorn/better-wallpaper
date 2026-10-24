//! Audit-only JSON extensions. Playback parsers retain their existing contracts.
use anyhow::{Result, bail};
use serde_json::Value;

/// Accept JSON plus // comments, non-nested /* comments */, and trailing commas.
/// The caller applies the 2 MiB document limit before allocating this buffer.
/// Replacing syntax with spaces preserves original error line/byte columns and
/// never joins tokens separated by a comment. Strings remain byte-for-byte intact.
pub(super) fn parse(bytes: &[u8]) -> Result<(Value, bool)> {
    match serde_json::from_slice(bytes) {
        Ok(value) => Ok((value, false)),
        Err(strict_error) => {
            // Do not hide invalid encoding inside a comment during normalization.
            std::str::from_utf8(bytes)?;
            let mut normalized = bytes.to_vec();
            let mut index = 0;
            let mut changed = false;
            while index < bytes.len() {
                match bytes[index] {
                    b'"' => index = string_end(bytes, index),
                    b'/' if bytes.get(index + 1) == Some(&b'/') => {
                        let start = index;
                        index += 2;
                        while index < bytes.len() && !matches!(bytes[index], b'\r' | b'\n') {
                            index += 1;
                        }
                        erase(&mut normalized[start..index]);
                        changed = true;
                    }
                    b'/' if bytes.get(index + 1) == Some(&b'*') => {
                        let start = index;
                        index += 2;
                        while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
                            index += 1;
                        }
                        if index + 1 >= bytes.len() {
                            let line = bytes[..start].iter().filter(|&&b| b == b'\n').count() + 1;
                            let column = bytes[..start]
                                .iter()
                                .rposition(|&b| b == b'\n')
                                .map_or(start + 1, |newline| start - newline);
                            bail!("unterminated block comment at line {line} column {column}");
                        }
                        index += 2;
                        erase(&mut normalized[start..index]);
                        changed = true;
                    }
                    _ => index += 1,
                }
            }

            // Comments are already whitespace. Only a comma after a possible
            // value and before a closing bracket/brace can be trailing. The
            // strict parser still validates nesting, values and separators.
            index = 0;
            let mut previous = None;
            while index < normalized.len() {
                let byte = normalized[index];
                if byte == b'"' {
                    index = string_end(&normalized, index);
                    previous = Some(b'"');
                    continue;
                }
                if byte == b',' && previous.is_some_and(|b| !b"[{,:".contains(&b)) {
                    let next = normalized[index + 1..].iter().find(|&&b| !whitespace(b));
                    if matches!(next, Some(b']' | b'}')) {
                        normalized[index] = b' ';
                        changed = true;
                    }
                }
                if !whitespace(byte) {
                    previous = Some(byte);
                }
                index += 1;
            }
            if !changed {
                return Err(strict_error.into());
            }
            Ok((serde_json::from_slice(&normalized)?, true))
        }
    }
}

fn whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn erase(bytes: &mut [u8]) {
    for byte in bytes {
        if !matches!(*byte, b'\r' | b'\n') {
            *byte = b' ';
        }
    }
}

fn string_end(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => return index + 1,
            b'\\' => index += 2,
            _ => index += 1,
        }
    }
    bytes.len()
}
