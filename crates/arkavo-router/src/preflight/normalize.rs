//! Unicode homoglyph normalization for preflight moderation
//!
//! Normalizes visually similar Unicode characters to their ASCII equivalents,
//! preventing bypass of keyword-based detection via Cyrillic, Greek, fullwidth,
//! and mathematical alphanumeric substitutions.

/// Normalize Unicode homoglyphs to ASCII equivalents in a single pass.
///
/// Covers: fullwidth ASCII (U+FF01-FF5E), Cyrillic lookalikes, Greek lookalikes,
/// mathematical alphanumeric symbols, CJK brackets, and other confusables.
pub fn normalize_homoglyphs(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        output.push(map_homoglyph(ch));
    }
    output
}

/// Map a single character to its ASCII equivalent if it's a known homoglyph.
fn map_homoglyph(ch: char) -> char {
    // Fullwidth ASCII: U+FF01 ('!') through U+FF5E ('~')
    let code = ch as u32;
    if (0xFF01..=0xFF5E).contains(&code) {
        return char::from_u32(code - 0xFEE0).unwrap_or(ch);
    }

    match ch {
        // Cyrillic uppercase → Latin uppercase
        '\u{0410}' => 'A', // А
        '\u{0412}' => 'B', // В
        '\u{0421}' => 'C', // С
        '\u{0415}' => 'E', // Е
        '\u{041D}' => 'H', // Н
        '\u{0406}' => 'I', // І
        '\u{0408}' => 'J', // Ј
        '\u{041A}' => 'K', // К
        '\u{041C}' => 'M', // М
        '\u{041E}' => 'O', // О
        '\u{0420}' => 'P', // Р
        '\u{0405}' => 'S', // Ѕ
        '\u{0422}' => 'T', // Т
        '\u{0425}' => 'X', // Х
        '\u{040E}' => 'Y', // Ў (close enough for bypass detection)
        '\u{0417}' => 'Z', // З (visually similar in some fonts)

        // Cyrillic lowercase → Latin lowercase
        '\u{0430}' => 'a', // а
        '\u{0435}' => 'e', // е
        '\u{043E}' => 'o', // о
        '\u{0440}' => 'p', // р
        '\u{0441}' => 'c', // с
        '\u{0443}' => 'y', // у
        '\u{0445}' => 'x', // х
        '\u{0455}' => 's', // ѕ
        '\u{0456}' => 'i', // і
        '\u{0458}' => 'j', // ј

        // Greek uppercase → Latin uppercase
        '\u{0391}' => 'A', // Α
        '\u{0392}' => 'B', // Β
        '\u{0395}' => 'E', // Ε
        '\u{0397}' => 'H', // Η
        '\u{0399}' => 'I', // Ι
        '\u{039A}' => 'K', // Κ
        '\u{039C}' => 'M', // Μ
        '\u{039D}' => 'N', // Ν
        '\u{039F}' => 'O', // Ο
        '\u{03A1}' => 'P', // Ρ
        '\u{03A4}' => 'T', // Τ
        '\u{03A7}' => 'X', // Χ
        '\u{0396}' => 'Z', // Ζ

        // Greek lowercase → Latin lowercase
        '\u{03BF}' => 'o', // ο
        '\u{03B1}' => 'a', // α (close enough in many fonts)

        // Mathematical alphanumeric symbols (U+1D400-1D7FF)
        // Bold uppercase A-Z: U+1D400-1D419
        '\u{1D400}'..='\u{1D419}' => char::from_u32('A' as u32 + (code - 0x1D400)).unwrap_or(ch),
        // Bold lowercase a-z: U+1D41A-1D433
        '\u{1D41A}'..='\u{1D433}' => char::from_u32('a' as u32 + (code - 0x1D41A)).unwrap_or(ch),
        // Italic uppercase A-Z: U+1D434-1D44D
        '\u{1D434}'..='\u{1D44D}' => char::from_u32('A' as u32 + (code - 0x1D434)).unwrap_or(ch),
        // Italic lowercase a-z: U+1D44E-1D467
        '\u{1D44E}'..='\u{1D467}' => char::from_u32('a' as u32 + (code - 0x1D44E)).unwrap_or(ch),
        // Bold italic uppercase A-Z: U+1D468-1D481
        '\u{1D468}'..='\u{1D481}' => char::from_u32('A' as u32 + (code - 0x1D468)).unwrap_or(ch),
        // Bold italic lowercase a-z: U+1D482-1D49B
        '\u{1D482}'..='\u{1D49B}' => char::from_u32('a' as u32 + (code - 0x1D482)).unwrap_or(ch),
        // Sans-serif uppercase A-Z: U+1D5A0-1D5B9
        '\u{1D5A0}'..='\u{1D5B9}' => char::from_u32('A' as u32 + (code - 0x1D5A0)).unwrap_or(ch),
        // Sans-serif lowercase a-z: U+1D5BA-1D5D3
        '\u{1D5BA}'..='\u{1D5D3}' => char::from_u32('a' as u32 + (code - 0x1D5BA)).unwrap_or(ch),
        // Monospace uppercase A-Z: U+1D670-1D689
        '\u{1D670}'..='\u{1D689}' => char::from_u32('A' as u32 + (code - 0x1D670)).unwrap_or(ch),
        // Monospace lowercase a-z: U+1D68A-1D6A3
        '\u{1D68A}'..='\u{1D6A3}' => char::from_u32('a' as u32 + (code - 0x1D68A)).unwrap_or(ch),

        // CJK brackets → ASCII brackets
        '\u{FF08}' => '(', // fullwidth (
        '\u{FF09}' => ')', // fullwidth )
        '\u{FF3B}' => '[', // fullwidth [
        '\u{FF3D}' => ']', // fullwidth ]
        '\u{FF5B}' => '{', // fullwidth {
        '\u{FF5D}' => '}', // fullwidth }
        '\u{3008}' => '<', // CJK angle bracket
        '\u{3009}' => '>', // CJK angle bracket
        '\u{300A}' => '<', // CJK double angle bracket
        '\u{300B}' => '>', // CJK double angle bracket

        // Other confusables
        '\u{00A0}' => ' ',              // non-breaking space
        '\u{2000}'..='\u{200A}' => ' ', // various Unicode spaces
        '\u{200B}' => '\0',             // zero-width space → null (stripped by caller if needed)
        '\u{200C}' => '\0',             // zero-width non-joiner
        '\u{200D}' => '\0',             // zero-width joiner
        '\u{FEFF}' => '\0',             // BOM / zero-width no-break space
        '\u{2013}' => '-',              // en dash
        '\u{2014}' => '-',              // em dash
        '\u{2018}' => '\'',             // left single quote
        '\u{2019}' => '\'',             // right single quote
        '\u{201C}' => '"',              // left double quote
        '\u{201D}' => '"',              // right double quote

        _ => ch,
    }
}

/// Strip zero-width and invisible characters from input.
/// Called after homoglyph mapping to remove null bytes introduced by ZWS/ZWNJ/ZWJ/BOM.
pub fn strip_invisible(input: &str) -> String {
    input.chars().filter(|&c| c != '\0').collect()
}

/// Full normalization pipeline: map homoglyphs then strip invisible characters.
pub fn normalize(input: &str) -> String {
    strip_invisible(&normalize_homoglyphs(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cyrillic_uppercase_mapping() {
        // Cyrillic О (U+041E) → Latin O
        assert_eq!(normalize_homoglyphs("DR\u{041E}P"), "DROP");
        // Cyrillic А (U+0410) → Latin A
        assert_eq!(normalize_homoglyphs("\u{0410}LTER"), "ALTER");
        // Cyrillic С (U+0421) → Latin C
        assert_eq!(normalize_homoglyphs("SELE\u{0421}T"), "SELECT");
    }

    #[test]
    fn test_cyrillic_lowercase_mapping() {
        assert_eq!(normalize_homoglyphs("s\u{0435}lect"), "select");
        assert_eq!(normalize_homoglyphs("dr\u{043E}p"), "drop");
    }

    #[test]
    fn test_greek_mapping() {
        // Greek Ο (U+039F) → Latin O
        assert_eq!(normalize_homoglyphs("DR\u{039F}P"), "DROP");
        // Greek Α (U+0391) → Latin A
        assert_eq!(normalize_homoglyphs("\u{0391}LTER"), "ALTER");
    }

    #[test]
    fn test_fullwidth_ascii() {
        // Fullwidth D=U+FF24, R=U+FF32, O=U+FF2F, P=U+FF30
        assert_eq!(
            normalize_homoglyphs("\u{FF24}\u{FF32}\u{FF2F}\u{FF30}"),
            "DROP"
        );
        // Fullwidth digits
        assert_eq!(normalize_homoglyphs("\u{FF11}\u{FF12}\u{FF13}"), "123");
    }

    #[test]
    fn test_mathematical_bold() {
        // Bold 'D' = U+1D403, 'R' = U+1D411, 'O' = U+1D40E, 'P' = U+1D40F
        assert_eq!(
            normalize_homoglyphs("\u{1D403}\u{1D411}\u{1D40E}\u{1D40F}"),
            "DROP"
        );
    }

    #[test]
    fn test_mixed_homoglyphs() {
        // Mix of Cyrillic and Latin
        assert_eq!(normalize("DR\u{041E}P T\u{0410}BLE"), "DROP TABLE");
    }

    #[test]
    fn test_zero_width_stripping() {
        // Zero-width space between DROP and TABLE
        assert_eq!(normalize("DROP\u{200B}TABLE"), "DROPTABLE");
        // BOM removal
        assert_eq!(normalize("\u{FEFF}DROP"), "DROP");
    }

    #[test]
    fn test_unicode_spaces() {
        // EN space (U+2002) → regular space
        assert_eq!(normalize_homoglyphs("DROP\u{2002}TABLE"), "DROP TABLE");
        // Non-breaking space → regular space
        assert_eq!(normalize_homoglyphs("DROP\u{00A0}TABLE"), "DROP TABLE");
    }

    #[test]
    fn test_cjk_brackets() {
        assert_eq!(normalize_homoglyphs("\u{FF08}test\u{FF09}"), "(test)");
    }

    #[test]
    fn test_ascii_passthrough() {
        let ascii = "DROP TABLE users; SELECT * FROM data";
        assert_eq!(normalize_homoglyphs(ascii), ascii);
    }

    #[test]
    fn test_empty_input() {
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn test_smart_quotes() {
        assert_eq!(normalize_homoglyphs("\u{201C}hello\u{201D}"), "\"hello\"");
    }
}
