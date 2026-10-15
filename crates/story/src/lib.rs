//! Moteur d'histoires Telmi, sans dépendance matérielle.
//!
//! Lit le format produit par Telmi Sync (`nodes.json`) en mémoire bornée et
//! fournit la navigation et les sauvegardes compatibles avec Telmi OS.

pub mod engine;
pub mod model;
pub mod parse;
pub mod save;

pub use engine::{Player, Resume, Step};
pub use model::{Stage, StageId, Story};
pub use parse::{load_story, parse_story, ParseError};
