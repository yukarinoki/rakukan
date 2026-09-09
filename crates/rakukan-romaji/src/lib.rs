mod converter;
mod rules;
mod trie;

pub use converter::{BackspaceResult, ConversionEvent, RomajiConverter};
pub use trie::SearchResult;

fn hiragana_to_katakana(text: &str) -> String {
    text.chars()
        .map(|c| {
            if ('\u{3041}'..='\u{3096}').contains(&c) {
                char::from_u32(c as u32 + 0x60).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}
