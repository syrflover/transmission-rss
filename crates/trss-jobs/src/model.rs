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
        /// No source this build knows reads the post (`자막 대기`).
        Subtitle = "subtitle",
    }
}

codes! {
    /// The steps of a job, in order.
    StepKind {
        Found = "found",
        Open = "open",
        Auth = "auth",
        Receive = "receive",
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

impl JobState {
    /// A state the job leaves only by a person's action or a later ticket's
    /// work, not by the runner.
    pub fn is_finished(self) -> bool {
        matches!(self, JobState::Failed | JobState::Partial | JobState::Done)
    }
}
