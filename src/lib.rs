//! frametime: trace performance problems in frame-time captures from any game.
//!
//! Read-only analysis. Findings state their evidence and a confidence and never
//! claim a cause as fact.

pub mod bound;
pub mod compare;
pub mod findings;
pub mod parse;
pub mod periodic;
pub mod present;
pub mod render;
pub mod report;
pub mod sensors;
pub mod series;
pub mod startup;
pub mod stats;
pub mod stutter;
pub mod synth;
pub mod util;
