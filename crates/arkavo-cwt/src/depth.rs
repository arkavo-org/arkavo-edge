//! A bound on CBOR nesting, applied before any decoder sees the bytes.
//!
//! `coset` hands untrusted input to ciborium, which decodes recursively and
//! imposes no depth limit of its own, so a token that packs thousands of
//! nested arrays or maps into its headers can exhaust a worker thread's
//! stack before its signature is ever checked. The size cap does not help:
//! one byte is enough for one level. This module walks the encoding
//! iteratively — no recursion, one pass, no allocation beyond a stack of
//! container counters — and refuses anything nested deeper than
//! [`MAX_NESTING_DEPTH`].
//!
//! COSE also carries CBOR *inside* byte strings: the protected header and
//! the CWT payload are both `bstr` items that a decoder parses separately.
//! Those are followed too, each against its own depth budget, because each
//! is its own recursive decode. A byte string that is not itself exactly one
//! well-formed CBOR item — a signature, a key coordinate — is left alone:
//! its bytes are never decoded, so its accidental resemblance to CBOR must
//! not refuse a valid token.

use crate::CwtError;
use std::ops::Range;

/// The deepest CBOR nesting a token may carry. CWTs are shallow structures;
/// this is far above anything the schema needs and far below the recursion a
/// decoder can survive.
pub const MAX_NESTING_DEPTH: usize = 16;

/// How many levels of CBOR-inside-a-byte-string the scan follows. COSE nests
/// it twice (the protected header, and claims inside the payload); four
/// leaves room and bounds the scan's own work.
const MAX_EMBEDDING_LEVELS: usize = 4;

/// Refuse `bytes` if its CBOR — or the CBOR embedded in its byte strings —
/// nests deeper than [`MAX_NESTING_DEPTH`].
pub fn check(bytes: &[u8]) -> Result<(), CwtError> {
    let outer = scan(bytes);
    if outer.depth > MAX_NESTING_DEPTH {
        return Err(too_deep());
    }
    let mut frontier: Vec<&[u8]> = outer.embedded.into_iter().map(|r| &bytes[r]).collect();
    for _ in 0..MAX_EMBEDDING_LEVELS {
        if frontier.is_empty() {
            break;
        }
        let mut next: Vec<&[u8]> = Vec::new();
        for slice in frontier {
            let inner = scan(slice);
            // Only a byte string that is exactly one complete CBOR item is
            // embedded CBOR. Anything else is opaque bytes no decoder will
            // walk, so its shape cannot be a reason to refuse the token.
            if !inner.complete {
                continue;
            }
            if inner.depth > MAX_NESTING_DEPTH {
                return Err(too_deep());
            }
            next.extend(inner.embedded.into_iter().map(|r| &slice[r]));
        }
        frontier = next;
    }
    Ok(())
}

fn too_deep() -> CwtError {
    CwtError::Cose(format!("nesting depth exceeds {MAX_NESTING_DEPTH}"))
}

/// What one pass over a CBOR byte stream found.
struct Scan {
    /// The deepest container nesting seen.
    depth: usize,
    /// Byte ranges of the definite-length byte strings encountered, the
    /// places CBOR can hide inside CBOR.
    embedded: Vec<Range<usize>>,
    /// The slice held exactly one well-formed CBOR item and nothing else.
    complete: bool,
}

/// Walk `bytes` as CBOR without decoding any of it.
///
/// Malformed input is not an error here: the scan stops and reports what it
/// saw, leaving the real diagnosis to the decoder that follows.
fn scan(bytes: &[u8]) -> Scan {
    // One entry per open container: `Some(n)` counts the items still owed by
    // a definite-length container, `None` marks an indefinite-length one that
    // ends at a break byte.
    let mut stack: Vec<Option<u64>> = Vec::new();
    let mut embedded: Vec<Range<usize>> = Vec::new();
    let mut depth = 0usize;
    let mut pos = 0usize;
    let mut started = false;

    loop {
        while matches!(stack.last(), Some(Some(0))) {
            stack.pop();
        }
        if started && stack.is_empty() {
            return Scan {
                depth,
                embedded,
                complete: pos == bytes.len(),
            };
        }
        let Some(&initial) = bytes.get(pos) else {
            return Scan {
                depth,
                embedded,
                complete: false,
            };
        };
        pos += 1;
        started = true;

        // A break byte closes the innermost indefinite-length container and
        // is not itself an item of that container.
        if initial == 0xff {
            if matches!(stack.last(), Some(None)) {
                stack.pop();
                continue;
            }
            return Scan {
                depth,
                embedded,
                complete: false,
            };
        }

        let major = initial >> 5;
        let additional = initial & 0x1f;
        let argument = match additional {
            0..=23 => Some(u64::from(additional)),
            24..=27 => {
                let width = 1usize << (additional - 24);
                let Some(slice) = bytes.get(pos..pos + width) else {
                    return Scan {
                        depth,
                        embedded,
                        complete: false,
                    };
                };
                pos += width;
                Some(
                    slice
                        .iter()
                        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte)),
                )
            }
            31 => None,
            // 28..=30 are reserved: the encoding is malformed.
            _ => {
                return Scan {
                    depth,
                    embedded,
                    complete: false,
                };
            }
        };

        // Every item fills one slot of the container holding it.
        if let Some(Some(remaining)) = stack.last_mut() {
            *remaining = remaining.saturating_sub(1);
        }

        let opened = match major {
            // Byte and text strings: definite ones are skipped whole,
            // indefinite ones are containers of chunks.
            2 | 3 => match argument {
                Some(length) => {
                    let Ok(length) = usize::try_from(length) else {
                        return Scan {
                            depth,
                            embedded,
                            complete: false,
                        };
                    };
                    let Some(end) = pos.checked_add(length).filter(|end| *end <= bytes.len())
                    else {
                        return Scan {
                            depth,
                            embedded,
                            complete: false,
                        };
                    };
                    if major == 2 && length >= 2 {
                        embedded.push(pos..end);
                    }
                    pos = end;
                    None
                }
                None => Some(None),
            },
            4 => Some(argument),
            // A map owes two items — a key and a value — per pair.
            5 => Some(argument.map(|pairs| pairs.saturating_mul(2))),
            // A tag wraps exactly one item; `31` is not a legal tag argument.
            6 => {
                if argument.is_none() {
                    return Scan {
                        depth,
                        embedded,
                        complete: false,
                    };
                }
                Some(Some(1))
            }
            // Majors 0, 1 and 7 are atomic.
            _ => None,
        };

        if let Some(frame) = opened {
            stack.push(frame);
            depth = depth.max(stack.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[[[...0...]]]` nested `levels` deep, as definite-length arrays.
    fn nested_arrays(levels: usize) -> Vec<u8> {
        let mut bytes = vec![0x81; levels];
        bytes.push(0x00);
        bytes
    }

    #[test]
    fn shallow_cbor_passes() {
        // {1: -8, 4: h'6b31'} — the shape of a COSE protected header.
        let header = [0xa2, 0x01, 0x27, 0x04, 0x42, 0x6b, 0x31];
        assert!(check(&header).is_ok());
        assert!(check(&nested_arrays(MAX_NESTING_DEPTH)).is_ok());
    }

    #[test]
    fn deeper_than_the_bound_is_refused() {
        assert!(matches!(
            check(&nested_arrays(MAX_NESTING_DEPTH + 1)),
            Err(CwtError::Cose(message)) if message.contains("nesting depth")
        ));
        assert!(check(&nested_arrays(200)).is_err());
        // Indefinite-length containers count the same.
        let mut indefinite = vec![0x9f; 200];
        indefinite.extend(std::iter::repeat_n(0xffu8, 200));
        assert!(check(&indefinite).is_err());
        // Maps too.
        assert!(check(&[0xa1; 200]).is_err());
    }

    #[test]
    fn breadth_is_not_depth() {
        // A flat array of 1000 integers is one level deep, whatever its size.
        let mut wide = vec![0x99, 0x03, 0xe8];
        wide.extend(std::iter::repeat_n(0x01u8, 1000));
        assert!(check(&wide).is_ok());
        // Sibling containers close before the next one opens.
        let mut siblings = vec![0x98, 0x64];
        for _ in 0..100 {
            siblings.extend_from_slice(&[0x81, 0x00]);
        }
        assert!(check(&siblings).is_ok());
    }

    #[test]
    fn cbor_hidden_in_a_byte_string_is_followed() {
        // h'<200 nested arrays>' — a decoder that parses the byte string's
        // contents recurses just as deeply as if they were inline.
        let inner = nested_arrays(200);
        let mut outer = vec![0x59];
        outer.extend_from_slice(&u16::try_from(inner.len()).unwrap().to_be_bytes());
        outer.extend_from_slice(&inner);
        assert!(check(&outer).is_err());
    }

    #[test]
    fn opaque_byte_strings_are_left_alone() {
        // A byte string that is not one complete CBOR item is never decoded,
        // so bytes that merely look like nesting must not refuse the token.
        let mut signature = vec![0x81; 200];
        // Trailing bytes leave the content incomplete as a CBOR item.
        signature.extend_from_slice(&[0x00, 0x00]);
        let mut outer = vec![0x59];
        outer.extend_from_slice(&u16::try_from(signature.len()).unwrap().to_be_bytes());
        outer.extend_from_slice(&signature);
        assert!(check(&outer).is_ok());
    }

    #[test]
    fn truncated_input_is_left_to_the_decoder() {
        // The scan reports what it saw rather than inventing an error: these
        // are refused later, by coset, with a parse error of its own.
        assert!(check(&[]).is_ok());
        assert!(check(&[0x9f]).is_ok());
        assert!(check(&[0x5a, 0xff]).is_ok());
        assert!(check(&[0x1c]).is_ok());
    }
}
