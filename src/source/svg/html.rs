//! An SVG's HTML labels, laid out and painted by Blitz (Stylo's styles, Taffy's layout, Parley's
//! text) through anyrender into vello_cpu, drawn over what the SVG backend draws: the one file
//! that names Blitz and what it paints with (ADR-0024). `svg.rs` finds the labels, places them
//! and writes them out; this turns them into pixels.
//!
//! Stands in for the painter until Spec-0004's third task: it paints none.

use super::Label;

/// Who lays the labels out, credited with the backend's own.
#[expect(dead_code, reason = "credited once the labels are painted, Spec-0004's third task")]
pub(super) const CREDIT: &str = "Blitz";

/// Whether the `labels`, under the drawing's stylesheets `css`, are ready to paint: none are yet.
pub(super) fn prepare(css: &str, labels: &[Label]) -> bool {
    let _ = (css, labels);
    false
}
