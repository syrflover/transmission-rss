//! Subscription suggestions of an import: what the comment above each rule
//! offers, and which offers could not become a subscription anyway.
//!
//! A suggestion is only a suggestion: the review shows it, the user checks the
//! ones to keep, and only checked ones become subscriptions
//! (`docs/specs/settings.md`, 기존 YAML). The comment is read by
//! [`super::comments`]; this module adds what the rules around it decide.

use std::{collections::HashMap, path::Path};

use super::comments::Reading;
use crate::{folders::is_collect_folder_itself, store::channels::RuleInput};

/// The four cases of the spec's suggestion table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An Anissia address and a creator: checked at first.
    WithCreator,
    /// An Anissia address only (`제작자 미정`): unchecked at first.
    AddressOnly,
    /// A comment that could not be read (`주석을 읽을 수 없음`).
    Unreadable,
    /// No comment (`주석 없음`).
    None,
}

impl Kind {
    /// The stable code the web sends.
    pub fn code(self) -> &'static str {
        match self {
            Kind::WithCreator => "with_creator",
            Kind::AddressOnly => "address_only",
            Kind::Unreadable => "unreadable",
            Kind::None => "none",
        }
    }
}

/// What one rule's comment offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub reading: Reading,
    /// Why a suggestion that was read cannot become a subscription, as a
    /// sentence for the review; `None` when it can (or there is none).
    pub blocked: Option<String>,
}

impl Suggestion {
    pub fn kind(&self) -> Kind {
        match &self.reading {
            Reading::None => Kind::None,
            Reading::Unreadable { .. } => Kind::Unreadable,
            Reading::Address {
                creator: Some(_), ..
            } => Kind::WithCreator,
            Reading::Address { creator: None, .. } => Kind::AddressOnly,
        }
    }

    /// Whether the review starts with the suggestion checked.
    pub fn checked_at_first(&self) -> bool {
        self.kind() == Kind::WithCreator && self.blocked.is_none()
    }

    /// The anime and creator of a suggestion that can become a subscription.
    pub fn offer(&self) -> Option<(i64, Option<&str>)> {
        match (&self.reading, &self.blocked) {
            (
                Reading::Address {
                    anime_no, creator, ..
                },
                None,
            ) => Some((*anime_no, creator.as_deref())),
            _ => None,
        }
    }
}

/// The suggestions for the rules of one channel, in order. `readings` and
/// `rules` run side by side; `rules` carry the directories as they will be
/// stored (below the collect folder).
///
/// A rule saving into the collect folder itself cannot be a subscription, and a
/// channel follows one anime with one rule: the first rule that offers an anime
/// keeps it, a later rule that offers the same anime is blocked.
pub fn suggest(readings: &[Reading], rules: &[RuleInput]) -> Vec<Suggestion> {
    let mut first_of: HashMap<i64, usize> = HashMap::new();
    readings
        .iter()
        .zip(rules)
        .enumerate()
        .map(|(index, (reading, rule))| {
            let mut blocked = None;
            if let Reading::Address { anime_no, .. } = reading {
                if is_collect_folder_itself(Path::new(&rule.directory)) {
                    blocked = Some(
                        "이 규칙은 수집 폴더 자체에 받아서 구독으로 만들 수 없어요. 저장 폴더를 작품 폴더로 고친 뒤 구독해 주세요."
                            .to_owned(),
                    );
                } else if let Some(first) = first_of.get(anime_no) {
                    blocked = Some(format!(
                        "한 채널은 같은 작품을 한 규칙으로만 구독해요. {}번째 규칙의 제안이 같은 작품이에요.",
                        first + 1
                    ));
                } else {
                    first_of.insert(*anime_no, index);
                }
            }
            Suggestion {
                reading: reading.clone(),
                blocked,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
