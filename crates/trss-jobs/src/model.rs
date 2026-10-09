//! The words the job records use (see `migrations/jobs/schema.sql` in
//! `trss-core`), with their codes in the database.

macro_rules! codes {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $code:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            pub fn code(self) -> &'static str {
                match self {
                    $($name::$variant => $code),+
                }
            }

            pub fn parse(code: &str) -> Option<$name> {
                match code {
                    $($code => Some($name::$variant),)+
                    _ => None,
                }
            }
        }

        impl rusqlite::types::FromSql for $name {
            fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
                let text = value.as_str()?;
                $name::parse(text).ok_or_else(|| {
                    rusqlite::types::FromSqlError::Other(
                        format!("unknown {} {text:?}", stringify!($name)).into(),
                    )
                })
            }
        }

        impl rusqlite::types::ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                Ok(self.code().into())
            }
        }
    };
}

codes! {
    /// Where a job is (see the schema's comment).
    JobState {
        Pending = "pending",
        Running = "running",
        Waiting = "waiting",
        Held = "held",
        Failed = "failed",
        Partial = "partial",
        Done = "done",
    }
}

codes! {
    /// What a waiting job or item waits for.
    Wait {
        /// A person has to pass the site's check (`인증 필요`).
        Auth = "auth",
        /// No source this build knows reads the post, or this build cannot
        /// yet analyse what it received (`자막 대기`).
        Subtitle = "subtitle",
        /// A person has to say which episode a file is (`회차 확인 필요`,
        /// the job's 배치 확인).
        Placement = "placement",
        /// A person has to approve replacing a subtitle (`교체 승인`).
        Approval = "approval",
        /// The episode's video is not there yet (`영상 대기`).
        Video = "video",
    }
}

codes! {
    /// The steps of a job, in order.
    StepKind {
        Found = "found",
        Open = "open",
        Auth = "auth",
        Receive = "receive",
        /// A person confirms where the files go (`배치 확인`).
        Placement = "placement",
        /// The files are kept in the work folder's `.trss/` (`보관`).
        Store = "store",
        /// A stored subtitle is copied beside its video (`적용`).
        Apply = "apply",
        /// A replacement waits for a person's approval (`교체 승인`).
        Approval = "approval",
    }
}

codes! {
    StepState {
        Current = "current",
        Waiting = "waiting",
        Done = "done",
        Failed = "failed",
        Partial = "partial",
    }
}

codes! {
    /// Where one item of a job is: the job's words, without `partial`.
    ItemState {
        Pending = "pending",
        Running = "running",
        Waiting = "waiting",
        Held = "held",
        Failed = "failed",
        Done = "done",
    }
}

codes! {
    /// Where one receipt of a file is (see the schema's comment).
    FileState {
        Intended = "intended",
        Fetched = "fetched",
        Done = "done",
        Held = "held",
        Failed = "failed",
        Abandoned = "abandoned",
    }
}

codes! {
    /// What a kept file is (`docs/specs/settings.md`, 보관 관계 필드).
    AssetKind {
        Subtitle = "subtitle",
        Font = "font",
        /// A package's description, licence and the like.
        Attachment = "attachment",
        /// A file a subtitle needs beside it (the SUB of an IDX).
        Companion = "companion",
        Other = "other",
    }
}

codes! {
    /// A subtitle's format as the analysis found it; `other` is a subtitle the
    /// app keeps but does not apply by itself (SSA, WebVTT, images).
    SubtitleFormat {
        Ass = "ass",
        Srt = "srt",
        Smi = "smi",
        Other = "other",
    }
}

codes! {
    /// What a row of a job's placement plan is to come to.
    PlanAction {
        /// Stored, then put beside its video.
        Apply = "apply",
        /// Stored only.
        Store = "store",
        /// Not kept.
        Drop = "drop",
    }
}

codes! {
    /// What a person chose to apply from a stored subtitle (`chosen` of a
    /// plan row; see `migrations/jobs/chosen_rows.sql` in `trss-core`).
    Chosen {
        /// As the episode's subtitle: a replacement when it has one.
        Apply = "apply",
        /// Beside the applied copies of the same creator's other format; none
        /// is taken off.
        Add = "add",
    }
}

codes! {
    /// What came of a row of a job's placement plan (see the schema's
    /// comment).
    Outcome {
        Applied = "applied",
        Stored = "stored",
        Existing = "existing",
        NoVideo = "no_video",
        Held = "held",
        Failed = "failed",
        Dropped = "dropped",
    }
}

codes! {
    /// An effect on a work folder's file.
    EffectKind {
        Store = "store",
        Apply = "apply",
        /// A replacement takes an existing file off its path.
        Remove = "remove",
        /// A replacement keeps a subtitle the app did not manage as a stored
        /// one before it changes it.
        Import = "import",
    }
}

codes! {
    /// Where an effect on a file is (see the schema's comment).
    EffectState {
        Intended = "intended",
        Prepared = "prepared",
        /// A removal's file is renamed aside, into the work folder's
        /// `.trss/tmp/`.
        SetAside = "set_aside",
        Done = "done",
        Held = "held",
        Failed = "failed",
        Abandoned = "abandoned",
    }
}

codes! {
    /// Where a replacement plan is (see the schema's comment).
    PlanState {
        Open = "open",
        Kept = "kept",
        Approved = "approved",
        Done = "done",
        Stale = "stale",
        Held = "held",
        Failed = "failed",
    }
}

codes! {
    /// Where a relocation's removal of an applied copy is (see
    /// `migrations/jobs/relocation.sql` in `trss-core`).
    RemovalState {
        /// Waiting for the person's confirmation of the relocation.
        Planned = "planned",
        Intended = "intended",
        /// The copy is renamed aside, into the work folder's `.trss/tmp/`.
        SetAside = "set_aside",
        Done = "done",
        /// The copy stays where it is (`reason` says why).
        Kept = "kept",
        Held = "held",
    }
}

codes! {
    /// What a replacement plan does to a path beside the video.
    PathAction {
        Add = "add",
        Replace = "replace",
        Remove = "remove",
        Keep = "keep",
    }
}

impl SubtitleFormat {
    /// The extension a copy beside a video gets.
    pub fn extension(self) -> Option<&'static str> {
        match self {
            SubtitleFormat::Ass => Some("ass"),
            SubtitleFormat::Srt => Some("srt"),
            SubtitleFormat::Smi => Some("smi"),
            SubtitleFormat::Other => None,
        }
    }

    /// The format a file's name says it is, for showing a subtitle the app holds
    /// no facts about: `None` when the name has no extension, otherwise by the
    /// extension, case aside (`ssa` is shown as `ass` and `sami` as `smi`; `zip`
    /// and any other extension are `other`).
    ///
    /// This is the shown format. What the app stores for a subtitle it applied
    /// is decided when it analyses the file (`place`), and differs for `ssa`
    /// and `sami`, which are stored as `other`.
    pub fn shown_for_name(name: &str) -> Option<SubtitleFormat> {
        name.rsplit_once('.')?;
        Some(match trss_subtitles::verify::Format::promised_by(name) {
            Some(trss_subtitles::verify::Format::Ass) => SubtitleFormat::Ass,
            Some(trss_subtitles::verify::Format::Srt) => SubtitleFormat::Srt,
            Some(trss_subtitles::verify::Format::Smi) => SubtitleFormat::Smi,
            Some(_) | None => SubtitleFormat::Other,
        })
    }
}

impl JobState {
    /// A state the job leaves only by a person's action or a later ticket's
    /// work, not by the runner.
    pub fn is_finished(self) -> bool {
        matches!(self, JobState::Failed | JobState::Partial | JobState::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::SubtitleFormat::{self, *};

    #[test]
    fn the_shown_format_of_a_name_is_its_extensions() {
        let shown = |name| SubtitleFormat::shown_for_name(name);
        for (name, expected) in [
            ("Show - 01.ass", Some(Ass)),
            ("Show - 01.ASS", Some(Ass)),
            ("Show - 01.ssa", Some(Ass)),
            ("Show - 01.srt", Some(Srt)),
            ("Show - 01.Srt", Some(Srt)),
            ("Show - 01.smi", Some(Smi)),
            ("Show - 01.sami", Some(Smi)),
            ("Show - 01.zip", Some(Other)),
            ("Show - 01.vtt", Some(Other)),
            ("Show - 01.", Some(Other)),
            ("Show.01.mkv", Some(Other)),
            ("dir.d/Show - 01.srt", Some(Srt)),
            ("dir.d/Show - 01", Some(Other)),
            (".ass", Some(Ass)),
            ("Show - 01", None),
            ("", None),
        ] {
            assert_eq!(shown(name), expected, "{name:?}");
        }
    }
}
