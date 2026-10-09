//! What the open plans of a `교체 승인` to-do change, summed, and when their
//! subtitles were received (`docs/specs/jobs.md`, 할 일).

use serde::Serialize;

use crate::place::replace::records::{Compared, Comparison, Format, Item, PlanView};

/// What the open plans of a `교체 승인` to-do change, summed: the card's
/// reason line.
#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Changes {
    /// Dialogue lines.
    pub added: u64,
    pub changed: u64,
    pub removed: u64,
    /// Lines whose timing moved.
    pub timing: u64,
    /// Styles added, removed and changed.
    pub styles: u64,
    /// Fonts added and removed.
    pub fonts: u64,
    /// Plans whose contents were not compared (made before the app compared
    /// them, or not readable): what they change is not in the numbers above.
    pub uncompared: u64,
    /// Compared plans that left a part out: a language of the dialogue with
    /// no counterpart, or the styles and fonts of an ASS set against another
    /// format. Two files with no styles at all leave nothing out.
    pub partial: u64,
    /// The open plans summed, `uncompared` and `partial` of them among them.
    pub plans: u64,
}

impl Changes {
    /// What one plan changes: the total of that plan alone, as a row of the
    /// plans' list says it.
    pub fn of(comparison: Option<&Comparison>) -> Self {
        let mut changes = Changes::default();
        changes.add(comparison);
        changes
    }

    /// Adds one open plan, with the comparison of its contents (`None` for a
    /// plan made before the app compared them).
    pub fn add(&mut self, comparison: Option<&Comparison>) {
        self.plans += 1;
        let Some(Comparison {
            result: Compared::Diff(diff),
            ..
        }) = comparison
        else {
            self.uncompared += 1;
            return;
        };
        self.added += diff.dialogue.added;
        self.changed += diff.dialogue.changed;
        self.removed += diff.dialogue.removed;
        self.timing += diff.timing.count;
        if let Some(styles) = &diff.styles {
            self.styles +=
                (styles.added.len() + styles.removed.len() + styles.changed.len()) as u64;
        }
        if let Some(fonts) = &diff.fonts {
            self.fonts += (fonts.added.len() + fonts.removed.len()) as u64;
        }
        let ass = diff.old.format == Format::Ass || diff.new.format == Format::Ass;
        let left_out = diff.not_compared.iter().any(|n| n.item == Item::Dialogue)
            || (ass && (diff.styles.is_none() || diff.fonts.is_none()));
        if left_out {
            self.partial += 1;
        }
    }
}

/// When the subtitles of a `교체 승인` to-do's open plans were received: the
/// `현재`·`새 자막` line of its card in the work detail. Of several plans, the
/// newest of each.
#[derive(Debug, Default)]
pub struct Received {
    /// The current subtitle the app manages: when it was received.
    pub current: Option<i64>,
    /// A current file the app did not manage: its change time.
    pub current_changed: Option<i64>,
    pub new: Option<i64>,
}

impl Received {
    pub fn add(&mut self, view: &PlanView) {
        if let Some((path, file)) = view.plan.current() {
            match view.applied.iter().find(|(p, _)| *p == path.path) {
                Some((_, facts)) => self.current = self.current.max(Some(facts.received_at)),
                None => self.current_changed = self.current_changed.max(Some(file.mtime_ms())),
            }
        }
        if let Some(new) = &view.new {
            self.new = self.new.max(Some(new.received_at));
        }
    }
}
