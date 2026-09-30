use crate::store::channels::mask_url;

/// The key that says two sightings are the same RSS item, unique per channel
/// (history keeps `(channel_id, identity_key)` unique).
///
/// The item's GUID is used when it has one (a blank GUID counts as none),
/// otherwise its link, and only when the feed gives neither, its title.
/// The source is part of the key (`guid:`, `link:`, `title:`), so a GUID that
/// happens to equal some other item's link does not merge the two.
///
/// Values are used verbatim, not normalized: a feed that changes an item's
/// GUID or link makes it a new item. Secret query values of the channel are
/// masked in the key, so the key can be stored and shown; two items that
/// differ only in a secret value share a key.
pub fn identity_key(
    guid: Option<&str>,
    link: Option<&str>,
    title: Option<&str>,
    secret_query: &[String],
) -> String {
    if let Some(guid) = guid.filter(|g| !g.trim().is_empty()) {
        format!("guid:{}", mask_url(guid, secret_query))
    } else if let Some(link) = link.filter(|l| !l.trim().is_empty()) {
        format!("link:{}", mask_url(link, secret_query))
    } else {
        format!("title:{}", title.unwrap_or_default())
    }
}

/// The item's link as it is stored: secret query values masked.
pub fn stored_link(link: Option<&str>, secret_query: &[String]) -> String {
    mask_url(link.unwrap_or_default(), secret_query)
}
