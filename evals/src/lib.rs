//! Evaluation harness for `cort`, as a sibling crate to the product crate.
//!
//! Dev-only. `install.sh` never builds or ships anything in here, and the product crate must not
//! depend on it. It exists to answer one question with numbers that cannot lie: does an agent
//! holding `cort` reach the labelled answer for less than an agent holding a shell?

pub mod adopt;
pub mod arms;
pub mod demand;
pub mod grade;
pub mod hook;
pub mod recall;
pub mod stream;
pub mod summary;
pub mod verify;

pub const ESTIMATOR: &str = "ascii/4 + non-ascii*1 (v1)";
