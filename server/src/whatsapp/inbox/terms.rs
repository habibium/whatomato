//! The terms a search matches in place of message text.
//!
//! Each is a keyed hash of three characters of the lowercased text, cut to 32 bits. A match is
//! confirmed by opening the message, so a collision costs a decryption, never a wrong result.

use std::collections::BTreeSet;

use uuid::Uuid;

use super::Body;
use crate::crypto::Cipher;

/// Characters in one term; a query shorter than this can't use them
pub const WIDTH: usize = 3;
/// Past this many distinct terms a long message's rest goes unindexed
const PER_MESSAGE_MAX: usize = 4096;

/// A kept message's terms, sorted and distinct
#[must_use]
pub fn of_body(cipher: &Cipher, account_id: Uuid, body: &Body) -> Vec<i32> {
    let mut terms = BTreeSet::new();
    for text in [&body.text, &body.file_name].into_iter().flatten() {
        for slice in slices(text) {
            if terms.len() == PER_MESSAGE_MAX {
                break;
            }
            terms.insert(term(cipher, account_id, &slice));
        }
    }
    terms.into_iter().collect()
}

/// The terms every message containing `query` has; `None` when it's too short to have any
#[must_use]
pub fn of_query(cipher: &Cipher, account_id: Uuid, query: &str) -> Option<Vec<i32>> {
    let terms: BTreeSet<i32> = slices(query)
        .map(|slice| term(cipher, account_id, &slice))
        .collect();
    (!terms.is_empty()).then(|| terms.into_iter().collect())
}

fn slices(text: &str) -> impl Iterator<Item = String> {
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    (0..chars.len().saturating_sub(WIDTH - 1))
        .map(move |start| chars[start..start + WIDTH].iter().collect())
}

fn term(cipher: &Cipher, account_id: Uuid, slice: &str) -> i32 {
    let context = [b"terms:".as_slice(), account_id.as_bytes()].concat();
    let tag = cipher.tag(&context, slice.as_bytes());
    i32::from_be_bytes([tag[0], tag[1], tag[2], tag[3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::whatsapp::inbox::Kind;

    fn body(text: &str) -> Body {
        Body {
            sender_jid: "8801700000000@s.whatsapp.net".to_owned(),
            sender_number: None,
            sender_name: None,
            kind: Kind::Text,
            text: Some(text.to_owned()),
            file_name: None,
            reply_to: None,
            mentions: Vec::new(),
            edited: false,
            media: None,
        }
    }

    fn found(cipher: &Cipher, account_id: Uuid, text: &str, query: &str) -> bool {
        let terms = of_body(cipher, account_id, &body(text));
        of_query(cipher, account_id, query)
            .unwrap()
            .iter()
            .all(|term| terms.binary_search(term).is_ok())
    }

    #[test]
    fn a_message_has_the_terms_of_any_part_of_it() {
        let cipher = Cipher::new(&[7; 32]);
        let account = Uuid::new_v4();
        let text = "Anthropic's IPO is one of the reasons for gold price crash";

        assert!(found(&cipher, account, text, "gold"));
        assert!(found(&cipher, account, text, "GOLD PRICE"));
        assert!(found(&cipher, account, text, "thropic"));
        assert!(!found(&cipher, account, text, "silver"));
    }

    #[test]
    fn counts_characters_not_bytes() {
        let cipher = Cipher::new(&[7; 32]);
        let account = Uuid::new_v4();

        assert!(found(&cipher, account, "আমি বাংলায় লিখি", "বাংলা"));
        assert_eq!(slices("ভাত").count(), 1);
    }

    #[test]
    fn a_query_shorter_than_a_term_has_none() {
        let cipher = Cipher::new(&[7; 32]);
        assert_eq!(of_query(&cipher, Uuid::new_v4(), "hi"), None);
    }

    #[test]
    fn another_account_gets_other_terms() {
        let cipher = Cipher::new(&[7; 32]);
        let text = body("same words");

        assert_ne!(
            of_body(&cipher, Uuid::new_v4(), &text),
            of_body(&cipher, Uuid::new_v4(), &text)
        );
    }

    #[test]
    fn terms_are_sorted_and_distinct() {
        let cipher = Cipher::new(&[7; 32]);
        let terms = of_body(&cipher, Uuid::new_v4(), &body("aaaa aaaa"));

        assert!(terms.windows(2).all(|pair| pair[0] < pair[1]));
        // "aaa", "aa ", "a a", " aa"
        assert_eq!(terms.len(), 4);
    }
}
