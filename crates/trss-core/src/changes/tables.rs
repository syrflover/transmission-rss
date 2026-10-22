//! Which tables announce their changes, as what, and how a connection notes
//! them.
//!
//! [`TABLES`] gives every table of the migrated schema either the kind its
//! changes are announced as and the column that names the changed row's target,
//! or the reason it is not announced. A test fails for a table that is missing
//! from it, a table in it that the schema lacks, and a target column the table
//! does not have, so a migration that adds a table cannot leave the screens
//! deaf to it.
//!
//! A connection that announces ([`crate::db::Db::open_announcing`]) installs
//! [`install`]'s TEMP triggers, which belong to that connection only. After an
//! INSERT, UPDATE or DELETE of a row of an announced table, its trigger
//! writes the kind and the target ID of the row into the table `changes.noted`
//! of an in-memory database attached to the connection, and [`take`] reads and
//! empties that table once the connection is back in autocommit mode, which is
//! after the writes were committed. The attached database is part of the
//! transaction: the rows of a rolled back transaction or savepoint are gone
//! with it, and a statement that matched no row noted nothing.
//!
//! The table is not a TEMP table because the TEMP database follows the
//! connection's `temp_store`: kept in memory only with `temp_store = MEMORY`,
//! which would also keep every sort and temporary b-tree of every query in
//! memory, where the default spills them to a file. An attached `:memory:`
//! database keeps the table and its journal in memory and leaves `temp_store`
//! as it was. A TEMP trigger may write into any attached database.
//!
//! Triggers are what makes this independent of the code that writes. SQLite's
//! update hook is not told of changes to `WITHOUT ROWID` tables (most of the
//! schema) or of a `DELETE` without `WHERE`, and the preupdate hook is not
//! built with the bundled SQLite. A trigger fires for rows that a schema
//! trigger inserted and rows a foreign key action deleted as well. `INSERT OR
//! REPLACE` fires the trigger of the inserted row only (the replaced row's
//! delete trigger fires only with `recursive_triggers`, which the schema does
//! not use), so a replaced row's old target is not noted; no code of the app
//! replaces a row whose target differs from the new one's.

use rusqlite::Connection;

use super::ChangeKind;

/// What the tables of the schema do for the screens.
pub(crate) enum Announced {
    /// Changes are announced as `kind`. `target` is the column whose value is
    /// the ID of the row's target in the kind's ID space (`None`: the row
    /// names no such target, and the change is announced without an ID).
    As {
        kind: ChangeKind,
        target: Option<&'static str>,
    },
    /// No screen shows the table, or another path announces what it holds.
    /// The reason is for the reader of this list.
    Not(#[allow(dead_code)] &'static str),
}

use Announced::{As, Not};
use ChangeKind::*;

const fn row(kind: ChangeKind, target: &'static str) -> Announced {
    As {
        kind,
        target: Some(target),
    }
}

const fn any(kind: ChangeKind) -> Announced {
    As { kind, target: None }
}

/// Every table of the migrated schema, sorted by name.
pub(crate) const TABLES: &[(&str, Announced)] = &[
    ("anilist_entries", any(SeasonInfo)),
    (
        "anilist_pace",
        Not("when AniList may be asked next; no screen shows it"),
    ),
    ("anissia_anime", row(Anissia, "anime_no")),
    // The work page's candidates say when Anissia was last read
    // (`last_read_at`), which a reading `trss` makes on its own moves even
    // when it finds no new line.
    ("anissia_caption_poll", any(Anissia)),
    (
        "anissia_pace",
        Not("when Anissia may be asked next; no screen shows it"),
    ),
    ("archive_suggestion_kept", row(Rules, "rule_id")),
    (
        "artwork_files",
        Not("the image files the app made, for the sweep of unused ones; the covers screens read are work_artwork and subscription_artwork"),
    ),
    ("caption_observations", any(Anissia)),
    ("channel_read_days", any(CollectStatus)),
    ("channel_read_status", any(CollectStatus)),
    ("channels", any(Rules)),
    ("collection_cycle", any(CollectStatus)),
    ("collection_settings", any(Settings)),
    (
        "commands",
        Not("CommandStore announces each command with its ID"),
    ),
    ("episode_undo_files", any(Rules)),
    ("episode_undos", row(Rules, "rule_id")),
    ("episodes", row(Library, "work_id")),
    ("first_run", any(Settings)),
    ("history_changes", row(History, "item_id")),
    ("history_first_reads", any(History)),
    ("history_items", row(History, "id")),
    (
        "library_generation",
        Not("a counter that follows the library tables, which announce what it follows"),
    ),
    ("media_files", row(Library, "work_id")),
    ("movie_anissia", row(SeasonInfo, "work_id")),
    ("policy_settings", any(Settings)),
    ("rejected_titles", any(Rules)),
    ("rule_started", row(Rules, "rule_id")),
    ("rule_subscriptions", row(Rules, "rule_id")),
    ("rules", row(Rules, "id")),
    (
        "search_pace",
        Not("when a feed host may be searched next; no screen shows it"),
    ),
    ("season_anissia", row(SeasonInfo, "work_id")),
    ("season_entries", row(SeasonInfo, "work_id")),
    ("season_info", row(SeasonInfo, "work_id")),
    ("seasons", row(Library, "work_id")),
    ("subscription_artwork", row(Rules, "rule_id")),
    ("subtitle_applied", row(SubtitleStore, "work_id")),
    ("subtitle_asset_removals", any(SubtitleStore)),
    ("subtitle_assets", row(SubtitleStore, "work_id")),
    ("subtitle_cleanups", row(SubtitleStore, "work_id")),
    ("subtitle_episode_exceptions", row(SubtitleMapping, "work_id")),
    ("subtitle_episode_mappings", row(SubtitleMapping, "work_id")),
    ("subtitle_item_rechecks", row(SubtitleJob, "job_id")),
    ("subtitle_job_dropped", row(SubtitleJob, "job_id")),
    ("subtitle_job_events", row(SubtitleJob, "job_id")),
    ("subtitle_job_files", row(SubtitleJob, "job_id")),
    ("subtitle_job_items", row(SubtitleJob, "job_id")),
    ("subtitle_job_members", any(SubtitleJob)),
    ("subtitle_job_plan", row(SubtitleJob, "job_id")),
    ("subtitle_job_screens", row(SubtitleScreen, "job_id")),
    ("subtitle_job_steps", row(SubtitleJob, "job_id")),
    ("subtitle_jobs", row(SubtitleJob, "id")),
    (
        "subtitle_mapping_clock",
        Not("a version counter that follows the mapping tables, which announce what it follows"),
    ),
    ("subtitle_mapping_conflicts", row(SubtitleMapping, "work_id")),
    ("subtitle_package_entries", any(SubtitleStore)),
    ("subtitle_packages", row(SubtitleStore, "work_id")),
    ("subtitle_relocations", row(SubtitleJob, "job_id")),
    ("subtitle_replacement_diffs", any(SubtitleJob)),
    ("subtitle_replacement_paths", any(SubtitleJob)),
    ("subtitle_replacements", row(SubtitleJob, "job_id")),
    ("subtitle_sources", row(Anissia, "anime_no")),
    ("subtitle_stored", row(SubtitleStore, "work_id")),
    ("subtitle_stored_assets", any(SubtitleStore)),
    ("transmission_downloading", any(CollectStatus)),
    (
        "transmission_listing",
        Not("when Transmission's torrents were listed, which the past search, the `다시 받기` offers and the deadlines read; the cycle writes it before its end, which is announced"),
    ),
    ("transmission_snapshot", any(CollectStatus)),
    (
        "transmission_torrents",
        Not("every torrent Transmission holds, which the past search and the `다시 받기` offers read; written with `transmission_listing`"),
    ),
    ("unread_works", any(Library)),
    ("unrecognized_checks", row(Library, "work_id")),
    ("unrecognized_files", row(Library, "work_id")),
    ("video_revisions", row(History, "item_id")),
    ("watch_folders", any(Library)),
    ("work_artwork", row(Artwork, "work_id")),
    ("work_subtitle_policy", row(Library, "work_id")),
    ("worker_info", any(CollectStatus)),
    ("works", row(Library, "id")),
];

/// The kind named `name` among the kinds of [`TABLES`], which are the kinds a
/// trigger notes.
fn kind_named(name: &str) -> Option<ChangeKind> {
    TABLES.iter().find_map(|(_, how)| match how {
        As { kind, .. } if kind.name() == name => Some(*kind),
        _ => None,
    })
}

/// The statements that make the table the triggers write into, in the attached
/// database `changes`, and the triggers, for the connection to run once.
///
/// A trigger's statement takes the conflict algorithm of the statement that
/// fires it when that one names its own (an upsert's `DO UPDATE`, `OR FAIL`,
/// `OR ROLLBACK`), which turns an `INSERT OR IGNORE` into a failure on a
/// repeated target. `ON CONFLICT DO NOTHING` is part of the statement itself
/// and stays.
///
/// A target is noted as `(kind, absent, id)`: `absent` is 1 and `id` empty for
/// a row whose target column is NULL or that has none, so that the primary key
/// holds one row per kind and ID, and one for "no ID", in a transaction.
fn setup_sql() -> String {
    let mut sql = String::from(
        "CREATE TABLE changes.noted (
             kind   TEXT    NOT NULL,
             absent INTEGER NOT NULL,
             id     TEXT    NOT NULL,
             PRIMARY KEY (kind, absent, id)
         ) WITHOUT ROWID;\n",
    );
    for (table, how) in TABLES {
        let As { kind, target } = how else { continue };
        let note = |row: &str| {
            match target {
            Some(column) => format!(
                "INSERT INTO changes.noted
                 VALUES ('{kind}', {row}.\"{column}\" IS NULL, ifnull(CAST({row}.\"{column}\" AS TEXT), ''))
                 ON CONFLICT DO NOTHING;",
                kind = kind.name(),
            ),
            None => format!(
                "INSERT INTO changes.noted VALUES ('{kind}', 1, '') ON CONFLICT DO NOTHING;",
                kind = kind.name(),
            ),
        }
        };
        for (event, body) in [
            ("insert", note("NEW")),
            ("update", format!("{} {}", note("OLD"), note("NEW"))),
            ("delete", note("OLD")),
        ] {
            sql.push_str(&format!(
                "CREATE TEMP TRIGGER \"changes_{table}_{event}\"
                 AFTER {when} ON main.\"{table}\" BEGIN {body} END;\n",
                when = event.to_uppercase(),
            ));
        }
    }
    sql
}

/// Makes this connection note the changes of announced tables. Run once after
/// the schema is migrated; what is noted stays in memory, so the noting never
/// touches a file.
pub(crate) fn install(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("ATTACH DATABASE ':memory:' AS changes", [])?;
    conn.execute_batch(&setup_sql())
}

/// Reads and empties what the connection has noted: each kind and target ID
/// once. Call it when the connection is not inside a transaction, so that what
/// it returns was committed.
pub(crate) fn take(conn: &Connection) -> rusqlite::Result<Vec<(ChangeKind, Option<String>)>> {
    let mut stmt = conn.prepare_cached("DELETE FROM changes.noted RETURNING kind, absent, id")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, bool>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut noted = Vec::new();
    for row in rows {
        let (kind, absent, id) = row?;
        // Every kind a trigger notes comes from the list.
        if let Some(kind) = kind_named(&kind) {
            noted.push((kind, (!absent).then_some(id)));
        }
    }
    Ok(noted)
}
