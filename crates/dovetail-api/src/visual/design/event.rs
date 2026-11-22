//! The `design` feature's events.

use crate::design::{Preview, VariantSet};
use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `design`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignEvent {
    /// A slice's preview changed state (ADR 0026).
    PreviewChanged(Preview),
    /// A slice declared its variants, or the user picked or kept an option (ADR 0026). It
    /// carries the whole set.
    VariantsUpserted(VariantSet),
}
