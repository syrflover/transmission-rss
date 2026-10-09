//! RSS feature module.
//!
//! [`ChannelEvaluator`] decides, for an item title, whether a channel's excludes and ordered
//! rules select it and where it would be saved. It has no Transmission, network or database
//! dependency, so the worker and the web preview can call the same judgment.

mod evaluate;

pub use evaluate::{
    compile_regex, regex_error, save_path, ChannelEvaluator, ChannelSpec, Evaluation, Outcome,
    RuleError, RuleSpec, SkipReason,
};
