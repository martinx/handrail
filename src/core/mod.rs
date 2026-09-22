//! Everything that does not depend on a particular agent.
//!
//! - [`catalog`]: packs and profiles, loaded from any [`catalog::Source`] and validated.
//! - [`plan`]: the exact set of file changes an operation will make, with preconditions.
//! - [`apply`]: carries a plan out transactionally, so an interrupted run never leaves a
//!   mix of old and new files.
//! - [`version`]: dotted version comparison for `min_version` gating.

pub mod apply;
pub mod catalog;
pub mod plan;
pub mod version;
