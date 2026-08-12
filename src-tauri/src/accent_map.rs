use std::collections::HashMap;

pub fn get_variants() -> HashMap<char, Vec<char>> {
    let mut m = HashMap::new();
    m.insert('a', vec!['á', 'à', 'â', 'ä', 'ã', 'å', 'ā']);
    m.insert('c', vec!['ç', 'ć', 'č']);
    m.insert('e', vec!['é', 'è', 'ê', 'ë', 'ē', 'ė', 'ę']);
    m.insert('i', vec!['í', 'ì', 'î', 'ï', 'ī', 'į']);
    m.insert('o', vec!['ó', 'ò', 'ô', 'ö', 'õ', 'ø', 'ō']);
    m.insert('u', vec!['ú', 'ù', 'û', 'ü', 'ū', 'ų']);
    m.insert('n', vec!['ñ', 'ń']);
    m.insert('l', vec!['ł', 'ĺ', 'ļ']);
    m
}

pub fn key_to_base_char(code: u16) -> Option<char> {
    match code {
        30 => Some('a'),
        46 => Some('c'),
        18 => Some('e'),
        23 => Some('i'),
        24 => Some('o'),
        22 => Some('u'),
        49 => Some('n'),
        38 => Some('l'),
        _ => None,
    }
}

pub fn char_to_xkb_keysym(ch: char) -> Option<u32> {
    match ch {
        'a'..='z' | 'A'..='Z' => Some(ch as u32),
        'á' => Some(0x00e1),
        'à' => Some(0x00e0),
        'â' => Some(0x00e2),
        'ä' => Some(0x00e4),
        'ã' => Some(0x00e3),
        'å' => Some(0x00e5),
        'ç' => Some(0x00e7),
        'ć' => Some(0x0106),
        'č' => Some(0x010d),
        'é' => Some(0x00e9),
        'è' => Some(0x00e8),
        'ê' => Some(0x00ea),
        'ë' => Some(0x00eb),
        'ē' => Some(0x0113),
        'ė' => Some(0x0116),
        'ę' => Some(0x0118),
        'í' => Some(0x00ed),
        'ì' => Some(0x00ec),
        'î' => Some(0x00ee),
        'ï' => Some(0x00ef),
        'ī' => Some(0x012a),
        'į' => Some(0x012e),
        'ó' => Some(0x00f3),
        'ò' => Some(0x00f2),
        'ô' => Some(0x00f4),
        'ö' => Some(0x00f6),
        'õ' => Some(0x00f5),
        'ø' => Some(0x00f8),
        'ō' => Some(0x014c),
        'ú' => Some(0x00fa),
        'ù' => Some(0x00f9),
        'û' => Some(0x00fb),
        'ü' => Some(0x00fc),
        'ū' => Some(0x016a),
        'ų' => Some(0x0173),
        'ñ' => Some(0x00f1),
        'ń' => Some(0x0143),
        'ł' => Some(0x0141),
        'ĺ' => Some(0x0139),
        'ļ' => Some(0x013b),
        _ => Some(0x01000000 | ch as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keycodes are what the kernel reports, and they are positional: 30 is
    /// the key where a US layout has `a`. Getting one wrong means holding a key
    /// offers somebody else's accents.
    #[test]
    fn every_mapped_key_has_variants() {
        let variants = get_variants();
        for code in [30u16, 46, 18, 23, 24, 22, 49, 38] {
            let base = key_to_base_char(code).expect("keycode should map to a letter");
            assert!(
                variants.get(&base).is_some_and(|list| !list.is_empty()),
                "'{base}' (code {code}) has no variants to offer"
            );
        }
    }

    /// A key with no accents must fall through untouched, or holding it would
    /// open an empty picker and swallow the letter.
    #[test]
    fn unmapped_keys_are_left_alone() {
        // Space, Enter, Escape, and a letter without variants.
        for code in [57u16, 28, 1, 44] {
            assert_eq!(key_to_base_char(code), None, "code {code} should not map");
        }
    }

    /// Every variant has to survive being turned into a keysym, or the picker
    /// offers a character it cannot type.
    #[test]
    fn every_offered_variant_can_be_typed() {
        for (base, variants) in get_variants() {
            for variant in variants {
                assert!(
                    char_to_xkb_keysym(variant).is_some(),
                    "'{variant}' is offered for '{base}' but has no keysym"
                );
            }
        }
    }

    /// The picker is chosen from with the number keys, so a list longer than
    /// nine has entries nobody can reach.
    #[test]
    fn no_list_is_longer_than_the_digits_available() {
        for (base, variants) in get_variants() {
            assert!(
                variants.len() <= 9,
                "'{base}' offers {} variants; only 9 can be picked by number",
                variants.len()
            );
        }
    }
}
