//! RSS feature module.
//!
//! [`ChannelEvaluator`] decides, for an item title, whether a channel's excludes and ordered
//! rules select it and where it would be saved. It has no Transmission, network or database
//! dependency, so the worker and the web preview can call the same judgment.
//! [`legacy`] adapts the YAML channel configuration to it for the `transmission-rss` binary.

mod evaluate;
pub mod legacy;

pub use evaluate::{
    ChannelEvaluator, ChannelSpec, Evaluation, Outcome, RuleError, RuleSpec, SkipReason,
};
