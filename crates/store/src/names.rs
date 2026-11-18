//! Name normalisation for searching and duplicate detection.
//!
//! `search_key` turns a name into a compact key so that people can be found
//! however they type it: case, accents and Arabic/Persian letter variants do not
//! matter, and neither do spaces, hyphens, ZWNJ (half-space) or diacritics.
//! Examples: `"Ali  Reza"` = `"alireza"`, `"علي"` = `"علی"`, `"می‌خواهم"` = `"میخواهم"`.
//! The key is stored next to each name (`person_name.search_key`) and indexed.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

fn map_letter(c: char) -> Option<char> {
    Some(match c {
        // Arabic letter variants -> the Persian letter people actually mean
        '\u{064A}' | '\u{0649}' | '\u{06CC}' | '\u{06D0}' => '\u{06CC}', // yeh forms -> ی
        '\u{0643}' | '\u{06A9}' => '\u{06A9}',                           // kaf -> ک
        '\u{0629}' | '\u{06C0}' | '\u{06D5}' => '\u{0647}',              // teh marbuta / heh forms -> ه
        '\u{0623}' | '\u{0625}' | '\u{0671}' => '\u{0627}',              // hamza alef forms -> ا
        '\u{0624}' => '\u{0648}',                                        // ؤ -> و
        // digits: Arabic-Indic and Persian -> ASCII
        '\u{0660}'..='\u{0669}' => char::from_u32('0' as u32 + (c as u32 - 0x0660))?,
        '\u{06F0}'..='\u{06F9}' => char::from_u32('0' as u32 + (c as u32 - 0x06F0))?,
        // separators and invisible joiners: dropped
        '\u{200C}' | '\u{200D}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' | '\u{0640}' => return None,
        c if c.is_whitespace() || c == '-' || c == '\u{2010}' || c == '\u{2011}' || c == '\u{2012}' || c == '\u{2013}' || c == '\'' || c == '\u{2019}' || c == '.' => return None,
        c => c,
    })
}

/// Compact lower-case key (see module docs). Empty input gives an empty key.
pub fn search_key(name: &str) -> String {
    name.nfkd().filter(|c| !is_combining_mark(*c)).flat_map(|c| c.to_lowercase()).filter_map(map_letter).collect()
}

/// Initials for privacy mode, e.g. `"Sara Adams"` -> `"S. A."`.
pub fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).map(|c| format!("{}.", c.to_uppercase())).collect::<Vec<_>>().join(" ")
}

/// Full name from parts, in "given family" order.
pub fn compose_full_name(given: Option<&str>, family: Option<&str>) -> String {
    [given, family].iter().flatten().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_case_accents_and_spacing() {
        assert_eq!(search_key("Ali  Reza"), "alireza");
        assert_eq!(search_key("ALIREZA"), "alireza");
        assert_eq!(search_key("José Núñez"), "josenunez");
        assert_eq!(search_key("O'Brien-Smith"), "obriensmith");
    }

    #[test]
    fn persian_and_arabic_variants_match() {
        assert_eq!(search_key("علي"), search_key("علی"));
        assert_eq!(search_key("كريم"), search_key("کریم"));
        assert_eq!(search_key("می‌خواهم"), search_key("می خواهم"));
        assert_eq!(search_key("می‌خواهم"), search_key("میخواهم"));
        assert_eq!(search_key("فاطمة"), search_key("فاطمه"));
        assert_eq!(search_key("مُحَمَّد"), search_key("محمد"));
        assert_eq!(search_key("احمد"), search_key("أحمد"));
        assert_eq!(search_key("حـسـن"), search_key("حسن"));
    }

    #[test]
    fn digits_are_unified() {
        assert_eq!(search_key("۱۲۳"), "123");
        assert_eq!(search_key("١٢٣"), "123");
    }

    #[test]
    fn empty_and_junk() {
        assert_eq!(search_key(""), "");
        assert_eq!(search_key("  - . "), "");
    }

    #[test]
    fn helpers() {
        assert_eq!(initials("Sara Adams"), "S. A.");
        assert_eq!(initials("سارا آدمز"), "س. آ.");
        assert_eq!(compose_full_name(Some("Sara"), Some("Adams")), "Sara Adams");
        assert_eq!(compose_full_name(None, Some("Adams")), "Adams");
        assert_eq!(compose_full_name(Some(" "), None), "");
    }
}
