//! A bound on CBOR nesting, applied before any decoder sees the bytes.
//!
//! `coset` hands untrusted input to ciborium, which does impose a limit of
//! its own: `from_reader` decodes with `recurse: 256` and fails with
//! `Error::RecursionLimitExceeded` beyond it. This module tightens that to
//! [`MAX_NESTING_DEPTH`], which is 16, and does so for two reasons. A CWT is
//! a shallow structure — nothing the schema needs comes near 16 levels — so
//! 256 frames of somebody else's recursion is stack this crate has no use
//! for; and the refusal here costs one iterative pass over the bytes, with
//! no recursion and no allocation beyond a stack of container counters,
//! rather than the recursive descent it replaces. The size cap does not help
//! with either: one byte buys one level.
//!
//! COSE also carries CBOR *inside* byte strings, and each of those is its
//! own decode with a fresh recursion budget. Exactly two are decoded — the
//! COSE_Sign1 protected header (element 0 of the array) and the payload
//! (element 2) — so the walk follows those two and nothing else. A
//! signature, a `kid`, an EC coordinate is opaque to every decoder in this
//! crate, so 32 random bytes that happen to begin like nested arrays must
//! not refuse a valid token.
//!
//! Those two slots must also hold a *definite-length* byte string. An
//! indefinite-length one has no single span to walk — its content is spread
//! over chunks — while ciborium concatenates the chunks and hands the result
//! to coset with a recursion budget of its own, which for the protected
//! header happens before any signature is checked. RFC 8949 deterministic
//! encoding forbids indefinite lengths and nothing in this stack emits one,
//! so such a token is refused outright rather than walked.

use crate::CwtError;
use std::ops::Range;

/// The deepest CBOR nesting a token may carry. CWTs are shallow structures;
/// this is far above anything the schema needs and far below the recursion a
/// decoder can survive.
pub const MAX_NESTING_DEPTH: usize = 16;

/// Refuse `bytes` if its CBOR — or the CBOR in the two byte strings a
/// COSE_Sign1 decoder parses in their own right — nests deeper than
/// [`MAX_NESTING_DEPTH`].
pub fn check(bytes: &[u8]) -> Result<(), CwtError> {
    if walk(bytes, 0).depth > MAX_NESTING_DEPTH {
        return Err(too_deep());
    }
    for range in decoded_byte_strings(bytes)? {
        // Whatever prefix the walk covered is judged on its own. An
        // over-deep prefix is over-deep however the byte string ends, and
        // ciborium does not require its input to stop where the item does:
        // trailing bytes make the content no less recursive to decode.
        if walk(&bytes[range], 0).depth > MAX_NESTING_DEPTH {
            return Err(too_deep());
        }
    }
    Ok(())
}

fn too_deep() -> CwtError {
    CwtError::Cose(format!("nesting depth exceeds {MAX_NESTING_DEPTH}"))
}

/// The byte strings a COSE_Sign1 decoder parses as CBOR in their own right:
/// the protected header (element 0) and the payload (element 2).
///
/// Anything that is not shaped like a COSE_Sign1 — a key set, a bare byte
/// string — carries none, and neither do the other slots of one. Those
/// bytes reach no decoder, so their shape is not the token's to answer for.
///
/// An *indefinite-length* byte string in either decoded position is refused
/// rather than described. Its content is spread over chunks with a range of
/// its own each, so there is no single span to walk, while ciborium
/// concatenates them and hands coset the result to decode with a fresh
/// 256-level budget — for the protected header, before any signature is
/// checked. RFC 8949 deterministic encoding forbids indefinite lengths and
/// nothing in this stack emits one, so refusing costs no real token.
fn decoded_byte_strings(bytes: &[u8]) -> Result<Vec<Range<usize>>, CwtError> {
    let mut pos = 0usize;
    // Tag 61 (CWT) and tag 18 (COSE_Sign1) may each wrap the array, and
    // neither changes which of its elements are decoded.
    let mut head = read_head(bytes, pos);
    while let Some(Head {
        major: 6,
        argument: Some(_),
        content,
    }) = head
    {
        pos = content;
        head = read_head(bytes, pos);
    }

    let Some(head) = head.filter(|head| head.major == 4) else {
        return Ok(Vec::new());
    };
    // A COSE_Sign1 is four elements; anything shorter has no payload slot.
    if head.argument.is_some_and(|elements| elements < 3) {
        return Ok(Vec::new());
    }

    let mut ranges = Vec::new();
    pos = head.content;
    for index in 0..3usize {
        // The break byte ends an indefinite-length array before element 2.
        if bytes.get(pos) == Some(&0xff) {
            break;
        }
        let Some(item) = read_head(bytes, pos) else {
            break;
        };
        if matches!(index, 0 | 2) && item.major == 2 {
            if item.argument.is_none() {
                return Err(CwtError::Cose(
                    "indefinite-length byte string in a decoded position".into(),
                ));
            }
            if let Some(range) = byte_string_range(bytes, &item) {
                ranges.push(range);
            }
        }
        let Some(next) = skip_item(bytes, pos) else {
            break;
        };
        pos = next;
    }
    Ok(ranges)
}

/// Where the content of a definite-length byte string lies, if the encoding
/// actually holds as many bytes as it claims.
fn byte_string_range(bytes: &[u8], head: &Head) -> Option<Range<usize>> {
    let length = usize::try_from(head.argument?).ok()?;
    let end = head
        .content
        .checked_add(length)
        .filter(|end| *end <= bytes.len())?;
    Some(head.content..end)
}

/// One CBOR head: its major type, its argument (`None` for an
/// indefinite-length item) and where the item's content begins.
#[derive(Clone, Copy)]
struct Head {
    major: u8,
    argument: Option<u64>,
    content: usize,
}

/// Read the head of the item at `pos`, or `None` if the encoding is
/// truncated or malformed. This is a scan, not a decoder: it stops rather
/// than diagnosing, and leaves the real error to the parser that follows.
fn read_head(bytes: &[u8], pos: usize) -> Option<Head> {
    let initial = *bytes.get(pos)?;
    let major = initial >> 5;
    let additional = initial & 0x1f;
    let mut content = pos + 1;
    let argument = match additional {
        0..=23 => Some(u64::from(additional)),
        24..=27 => {
            let width = 1usize << (additional - 24);
            let slice = bytes.get(content..content.checked_add(width)?)?;
            content += width;
            Some(
                slice
                    .iter()
                    .fold(0u64, |value, byte| (value << 8) | u64::from(*byte)),
            )
        }
        31 => None,
        // 28..=30 are reserved: the encoding is malformed.
        _ => return None,
    };
    Some(Head {
        major,
        argument,
        content,
    })
}

/// What one pass over the CBOR item at `start` found.
struct Walk {
    /// The deepest container nesting seen, whether or not the item ended.
    depth: usize,
    /// Where the item ended; meaningful only when `complete`.
    end: usize,
    /// The walk covered one whole, well-formed item.
    complete: bool,
}

/// Where the item at `start` ends, or `None` if it is not well-formed.
fn skip_item(bytes: &[u8], start: usize) -> Option<usize> {
    let walk = walk(bytes, start);
    walk.complete.then_some(walk.end)
}

/// Walk the CBOR item at `start` without decoding any of it.
///
/// Malformed input is not an error here: the walk stops and reports what it
/// saw, leaving the real diagnosis to the decoder that follows. Byte strings
/// are skipped whole — whether their content is itself CBOR is a question
/// only the position of the string can answer, and [`decoded_byte_strings`]
/// is what answers it.
fn walk(bytes: &[u8], start: usize) -> Walk {
    // One entry per open container: `Some(n)` counts the items still owed by
    // a definite-length container, `None` marks an indefinite-length one that
    // ends at a break byte.
    let mut stack: Vec<Option<u64>> = Vec::new();
    let mut depth = 0usize;
    let mut pos = start;
    let mut started = false;

    loop {
        while matches!(stack.last(), Some(Some(0))) {
            stack.pop();
        }
        if started && stack.is_empty() {
            return Walk {
                depth,
                end: pos,
                complete: true,
            };
        }
        let incomplete = Walk {
            depth,
            end: pos,
            complete: false,
        };
        let Some(&initial) = bytes.get(pos) else {
            return incomplete;
        };

        // A break byte closes the innermost indefinite-length container and
        // is not itself an item of that container.
        if initial == 0xff {
            if matches!(stack.last(), Some(None)) {
                pos += 1;
                started = true;
                stack.pop();
                continue;
            }
            return incomplete;
        }

        let Some(head) = read_head(bytes, pos) else {
            return incomplete;
        };
        pos = head.content;
        started = true;

        // Every item fills one slot of the container holding it.
        if let Some(Some(remaining)) = stack.last_mut() {
            *remaining = remaining.saturating_sub(1);
        }

        let opened = match head.major {
            // Byte and text strings: definite ones are skipped whole,
            // indefinite ones are containers of chunks.
            2 | 3 => match head.argument {
                Some(length) => {
                    let Some(end) = usize::try_from(length)
                        .ok()
                        .and_then(|length| pos.checked_add(length))
                        .filter(|end| *end <= bytes.len())
                    else {
                        return Walk {
                            depth,
                            end: pos,
                            complete: false,
                        };
                    };
                    pos = end;
                    None
                }
                None => Some(None),
            },
            4 => Some(head.argument),
            // A map owes two items — a key and a value — per pair.
            5 => Some(head.argument.map(|pairs| pairs.saturating_mul(2))),
            // A tag wraps exactly one item; `31` is not a legal tag argument.
            6 => {
                if head.argument.is_none() {
                    return Walk {
                        depth,
                        end: pos,
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

    /// A definite-length byte string holding `content`.
    fn byte_string(content: &[u8]) -> Vec<u8> {
        let length = u16::try_from(content.len()).expect("test byte strings stay under 64 KiB");
        let mut bytes = Vec::new();
        if length < 24 {
            bytes.push(0x40 | u8::try_from(length).unwrap());
        } else if length < 256 {
            bytes.extend([0x58, u8::try_from(length).unwrap()]);
        } else {
            bytes.push(0x59);
            bytes.extend(length.to_be_bytes());
        }
        bytes.extend_from_slice(content);
        bytes
    }

    /// The COSE_Sign1 array — `[protected, {}, payload, signature]` — built
    /// by hand, so a test can put arbitrary bytes in any one slot.
    fn sign1(protected: &[u8], payload: &[u8], signature: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x84];
        bytes.extend(byte_string(protected));
        bytes.push(0xa0);
        bytes.extend(byte_string(payload));
        bytes.extend(byte_string(signature));
        bytes
    }

    /// An indefinite-length byte string carrying `content` as one chunk:
    /// `5F <definite chunk> FF`.
    fn indefinite_byte_string(content: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x5f];
        bytes.extend(byte_string(content));
        bytes.push(0xff);
        bytes
    }

    /// The COSE_Sign1 array with `slot` — 0 (protected header) or 2
    /// (payload) — holding an indefinite-length byte string over `content`,
    /// and every other slot ordinary.
    fn sign1_with_indefinite(slot: usize, content: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x84];
        if slot == 0 {
            bytes.extend(indefinite_byte_string(content));
        } else {
            bytes.extend(byte_string(&[0xa0]));
        }
        bytes.push(0xa0);
        if slot == 2 {
            bytes.extend(indefinite_byte_string(content));
        } else {
            bytes.extend(byte_string(&[0xa0]));
        }
        bytes.extend(byte_string(&[]));
        bytes
    }

    #[test]
    fn shallow_cbor_passes() {
        // {1: -8, 4: h'6b31'} — the shape of a COSE protected header.
        let header = [0xa2, 0x01, 0x27, 0x04, 0x42, 0x6b, 0x31];
        assert!(check(&header).is_ok());
        assert!(check(&nested_arrays(MAX_NESTING_DEPTH)).is_ok());
        assert!(check(&sign1(&header, &[0xa0], &[0u8; 64])).is_ok());
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

    /// The two byte strings a COSE_Sign1 decoder parses in their own right
    /// are walked: a decoder recurses through their content exactly as if it
    /// were inline, so the bound has to hold there too.
    #[test]
    fn cbor_in_the_header_and_the_payload_is_followed() {
        let deep = nested_arrays(200);
        assert!(check(&sign1(&[0xa0], &deep, &[0u8; 64])).is_err());
        assert!(check(&sign1(&deep, &[0xa0], &[0u8; 64])).is_err());

        // Tagged the way authnz-rs and `arkavo-permit` emit them: tag 18
        // around the array, optionally under the CWT tag. Neither changes
        // which elements a decoder parses.
        let mut tagged = vec![0xd2];
        tagged.extend(sign1(&[0xa0], &deep, &[0u8; 64]));
        assert!(check(&tagged).is_err());
        let mut cwt = vec![0xd8, 0x3d];
        cwt.extend(tagged);
        assert!(check(&cwt).is_err());
    }

    /// The bound holds on whatever prefix the walk covered. A byte string
    /// that is 200 arrays deep and then two bytes more is not one clean CBOR
    /// item, but `ciborium::from_reader` does not require the input to end
    /// where the item does: it recurses through the nesting all the same.
    #[test]
    fn an_over_deep_prefix_is_over_deep_however_it_ends() {
        let mut trailing = nested_arrays(200);
        trailing.extend_from_slice(&[0x00, 0x00]);
        assert!(matches!(
            check(&sign1(&[0xa0], &trailing, &[0u8; 64])),
            Err(CwtError::Cose(message)) if message.contains("nesting depth")
        ));

        // Truncated the other way — the nesting never closes — is refused on
        // the same grounds rather than passed to the decoder.
        assert!(check(&sign1(&[0xa0], &[0x81; 200], &[0u8; 64])).is_err());
    }

    /// An indefinite-length byte string in a slot a decoder parses is the one
    /// way past the bound: no single range covers its content, so nothing was
    /// walked, while ciborium concatenates the chunks and hands coset the
    /// result with a fresh 256-level budget — for the protected header, before
    /// any signature is checked. It is refused on its shape instead.
    #[test]
    fn an_indefinite_length_byte_string_in_a_decoded_slot_is_refused() {
        let deep = nested_arrays(200);

        // `84 5F 58C9 <200x 81, 00> FF A0 41A0 40`: the deep nesting hides in
        // an indefinite-length protected header.
        let header = sign1_with_indefinite(0, &deep);
        assert_eq!(
            &header[..4],
            &[0x84, 0x5f, 0x58, 0xc9],
            "the vector under test: {header:02x?}"
        );
        for token in [header, sign1_with_indefinite(2, &deep)] {
            assert!(
                matches!(
                    check(&token),
                    Err(CwtError::Cose(ref message)) if message.contains("indefinite-length byte string")
                ),
                "not refused: {:02x?}",
                &token[..8.min(token.len())]
            );
        }

        // The refusal is about the encoding, not about what it holds: an
        // indefinite-length slot carrying nothing deep at all is refused too,
        // because there is no span for the bound to hold on.
        assert!(check(&sign1_with_indefinite(0, &[0xa0])).is_err());
        assert!(check(&sign1_with_indefinite(2, &[0xa0])).is_err());

        // And an ordinary token, whose slots are definite-length, still parses.
        assert!(check(&sign1(&[0xa0], &[0xa0], &[0u8; 64])).is_ok());
    }

    /// Byte strings nothing decodes are left alone whatever they contain.
    /// The signature and the `kid` are the two a valid token could plausibly
    /// carry deep-looking bytes in: 64 or 32 bytes of a hash or a signature
    /// begin however they begin, and refusing the token for it would be
    /// refusing it for the shape of its own randomness.
    #[test]
    fn opaque_byte_strings_are_left_alone() {
        let deep = nested_arrays(200);
        assert!(check(&sign1(&[0xa0], &[0xa0], &deep)).is_ok());

        // A `kid` inside the protected header — which *is* walked — holding
        // bytes that look like 32 levels of nested array.
        let mut protected = vec![0xa2, 0x01, 0x27, 0x04];
        protected.extend(byte_string(&[0x81u8; 32]));
        assert!(check(&sign1(&protected, &[0xa0], &[0u8; 64])).is_ok());

        // And a byte string that is not part of a COSE_Sign1 at all.
        assert!(check(&byte_string(&deep)).is_ok());
    }

    #[test]
    fn truncated_input_is_left_to_the_decoder() {
        // The scan reports what it saw rather than inventing an error: these
        // are refused later, by coset, with a parse error of its own.
        assert!(check(&[]).is_ok());
        assert!(check(&[0x9f]).is_ok());
        assert!(check(&[0x5a, 0xff]).is_ok());
        assert!(check(&[0x1c]).is_ok());
        // A truncated COSE_Sign1 array: the elements that are there are
        // still walked, and the ones that are not simply end the scan.
        assert!(check(&[0x84, 0x41, 0xa0]).is_ok());
    }
}
