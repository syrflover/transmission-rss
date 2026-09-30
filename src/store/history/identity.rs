use sha2::{Digest, Sha256};

use crate::store::channels::mask_url;

/// The key that says two sightings are the same RSS item, unique per channel
/// (history keeps `(channel_id, identity_key)` unique).
///
/// The item's GUID is used when it has one (a blank GUID counts as none),
/// otherwise its link, and only when the feed gives neither, its title.
/// The source is part of the key (`guid:`, `link:`, `title:`), so a GUID that
/// happens to equal some other item's link does not merge the two.
///
/// The key is `<source>:<SHA-256 of the value, in lowercase hex>`. The value is
/// hashed exactly as the feed gives it (not normalized, and not masked): two
/// items are the same item exactly when their values are byte-for-byte equal,
/// and a feed that changes an item's GUID or link makes it a new item. Masking
/// the value first would merge items that differ only in a secret-named query
/// value (`details.php?id=101` and `?id=102`), and would change every key when
/// a query name is switched between secret and public. The hash is one-way, so
/// the key holds nothing of the value (a GUID or link may carry a token) and can
/// be stored and shown.
pub fn identity_key(guid: Option<&str>, link: Option<&str>, title: Option<&str>) -> String {
    let (source, value) = if let Some(guid) = guid.filter(|g| !g.trim().is_empty()) {
        ("guid", guid)
    } else if let Some(link) = link.filter(|l| !l.trim().is_empty()) {
        ("link", link)
    } else {
        ("title", title.unwrap_or_default())
    };
    format!("{source}:{:x}", Sha256::digest(value.as_bytes()))
}

/// The item's link as it is stored: secret query values masked.
pub fn stored_link(link: Option<&str>, secret_query: &[String]) -> String {
    mask_url(link.unwrap_or_default(), secret_query)
}
