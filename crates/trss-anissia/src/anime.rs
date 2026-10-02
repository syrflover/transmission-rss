use trss_core::Millis;

/// The schedule group for anime that air on no weekday (`기타`).
pub const WEEK_OTHER: u8 = 7;
/// The schedule group for anime that have not started (`신작`).
pub const WEEK_UPCOMING: u8 = 8;

/// An anime as Anissia's schedule last listed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anime {
    /// Anissia's `animeNo`.
    pub anime_no: i64,
    /// The Korean title.
    pub subject: String,
    pub original_subject: Option<String>,
    /// 0 (Sunday) to 6 (Saturday), [`WEEK_OTHER`] or [`WEEK_UPCOMING`].
    pub week: u8,
    /// `HH:MM` in Asia/Seoul.
    pub air_time: Option<String>,
    /// `YYYY-MM-DD`, or `YYYY-MM` when only the month is known.
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    /// Anissia's `ON` or `OFF`.
    pub status: String,
    /// When the row was received.
    pub fetched_at: Millis,
}
