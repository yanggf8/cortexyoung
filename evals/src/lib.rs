//! Evaluation harness for `cort`, as a sibling crate to the product crate.
//!
//! Dev-only. `install.sh` never builds or ships anything in here, and the product crate must not
//! depend on it. It exists to answer one question with numbers that cannot lie: does an agent
//! holding `cort` reach the labelled answer for less than an agent holding a shell?

pub mod arms;
pub mod capture_gap;
pub mod cost_audit;
pub mod cost_trial;
pub mod demand;
pub mod gate_audit;
pub mod grade;
pub mod recall;
pub mod stream;
pub mod summary;
pub mod time;
pub mod uptake;
pub mod uptake_judge;
pub mod verify;

pub const ESTIMATOR: &str = "ascii/4 + non-ascii*1 (v1)";
