use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

/// A point in time in Unix milliseconds.
pub type Millis = i64;

/// Source of the current time in Unix milliseconds.
pub type Clock = Arc<dyn Fn() -> Millis + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as Millis)
    })
}
