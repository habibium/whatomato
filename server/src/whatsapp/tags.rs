//! Tagging people in a group: `@number` in the text, and the same person in the message's mentions

use std::collections::HashMap;

use whatsapp_rust::{features::GroupParticipant, prelude::Jid};

/// Whether `body` tags `number` as `@number`, and not a longer number that starts with it
#[must_use]
pub fn written(body: &str, number: &str) -> bool {
    let tag = format!("@{number}");
    body.match_indices(&tag)
        .any(|(at, _)| !follows_digit(&body[at + tag.len()..]))
}

fn follows_digit(rest: &str) -> bool {
    rest.starts_with(|c: char| c.is_ascii_digit())
}

/// The members a group knows by LID, by their number
pub(super) fn lids_of(members: &[GroupParticipant]) -> HashMap<String, Jid> {
    members
        .iter()
        .filter_map(|member| {
            let lid = member
                .lid
                .as_ref()
                .or_else(|| member.jid.is_lid().then_some(&member.jid))?;
            let number = member
                .phone_number
                .as_ref()
                .or(Some(&member.jid))
                .filter(|jid| jid.is_pn())?;
            Some((number.user.to_string(), lid.to_non_ad()))
        })
        .collect()
}

/// The body and mentions as the group addresses the people tagged: by LID where it knows them that
/// way, since a phone shows a name only for a tag in the form its group uses. A number no longer
/// written in the body, as after the owner edited it on approval, isn't tagged.
pub(super) fn retag(
    mut body: String,
    numbers: Vec<String>,
    lids: &HashMap<String, Jid>,
) -> (String, Vec<String>) {
    let mut mentions = Vec::with_capacity(numbers.len());
    for number in numbers {
        if !written(&body, &number) {
            continue;
        }
        match lids.get(&number) {
            Some(lid) => {
                body = swap(&body, &number, &lid.user);
                mentions.push(lid.to_string());
            }
            None => mentions.push(Jid::pn(number).to_string()),
        }
    }
    (body, mentions)
}

fn swap(body: &str, from: &str, to: &str) -> String {
    let tag = format!("@{from}");
    let mut swapped = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = rest.find(&tag) {
        swapped.push_str(&rest[..at]);
        let after = &rest[at + tag.len()..];
        if follows_digit(after) {
            swapped.push_str(&tag);
        } else {
            swapped.push('@');
            swapped.push_str(to);
        }
        rest = after;
    }
    swapped.push_str(rest);
    swapped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(jid: &str, phone_number: Option<&str>, lid: Option<&str>) -> GroupParticipant {
        let mut member = GroupParticipant::new(jid.parse().unwrap());
        member.phone_number = phone_number.map(|jid| jid.parse().unwrap());
        member.lid = lid.map(|jid| jid.parse().unwrap());
        member
    }

    #[test]
    fn a_tag_is_the_whole_number() {
        assert!(written("Hi @8801711111111, see this", "8801711111111"));
        assert!(written("@8801711111111", "8801711111111"));
        assert!(!written("Hi @88017111111119", "8801711111111"));
        assert!(!written("Hi 8801711111111", "8801711111111"));
        assert!(written(
            "@88017111111119 and @8801711111111.",
            "8801711111111"
        ));
    }

    #[test]
    fn a_group_that_hides_numbers_is_tagged_by_lid() {
        let lids = lids_of(&[
            member("111@lid", Some("8801711111111@s.whatsapp.net"), None),
            member("8801722222222@s.whatsapp.net", None, Some("222@lid")),
            // No number to match a tag against
            member("333@lid", None, None),
        ]);
        assert_eq!(lids.len(), 2);

        let (body, mentions) = retag(
            "@8801711111111 and @8801722222222, and @88017111111119 isn't tagged. @8801733333333"
                .to_owned(),
            vec![
                "8801711111111".to_owned(),
                "8801722222222".to_owned(),
                "8801733333333".to_owned(),
                // Taken out of the text when the owner edited it
                "8801744444444".to_owned(),
            ],
            &lids,
        );
        assert_eq!(
            body,
            "@111 and @222, and @88017111111119 isn't tagged. @8801733333333"
        );
        assert_eq!(
            mentions,
            ["111@lid", "222@lid", "8801733333333@s.whatsapp.net"]
        );
    }
}
