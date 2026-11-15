//! Recipes and Set plans (wbx-39): the Recipe format and its checks
//! ([`recipe`]), and the deterministic sequencer that builds a Set plan
//! from a Recipe and a pool of tracks ([`sequence`]). Pure: no Library, no
//! Sidecar, no clock, no randomness. The service gathers each track's
//! tempo, key, Intensity, Situations, genre and date added, and keeps the
//! plan in the Sidecar.
mod camelot;
pub mod guide;
pub mod recipe;
pub mod sequence;

pub use camelot::Camelot;
pub use recipe::{Problem, Recipe};
pub use sequence::{plan, Plan, Unsatisfiable};
