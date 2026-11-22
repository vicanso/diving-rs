//! Motion tokens and reduced-motion helpers matching `DESIGN.md` Section 7.

use nsh_core::animation::{Keyframe, KeyframeTiming, MotionChannel, MotionSpec};
use nsh_core::element::StyleRefinement;
use nsh_core::prelude::*;

/// Instant hover wash / subtle border highlight (`120 ms`).
pub const INSTANT: f32 = 120.0;
/// Fast press feedback / switch thumb / popup exit (`160 ms`).
pub const FAST: f32 = 160.0;
/// Base selection slide / glyph swap / inspector disclosure (`220 ms`).
pub const BASE: f32 = 220.0;
/// Slow dialog / menu reveal (`320 ms`).
pub const SLOW: f32 = 320.0;
/// Slower staggered list wave (`450 ms`).
pub const SLOWER: f32 = 450.0;

/// Button press scale (`0.97`). List rows never scale; they deepen background.
pub const PRESS_BTN_SCALE: f32 = 0.97;
/// Square icon button press scale (`0.95`).
pub const PRESS_ICON_SCALE: f32 = 0.95;

/// AffectUI soft-settle cubic bezier (`0.22, 1.0, 0.36, 1.0`).
pub const SOFT_SETTLE: (f32, f32, f32, f32) = (0.22, 1.0, 0.36, 1.0);

/// Critically damped spring for side sheets, panels, and dialog entrances (`NSH 0.6.0`).
pub fn panel_spring() -> Spring {
    Spring::from_stiffness(280.0).damping_ratio(1.0)
}

/// Spring-driven entrance specification for centered dialogs (`12pt` rise, `0.97` scale, fade).
pub fn dialog_enter_spec() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::default())
        .track(
            MotionChannel::Opacity,
            [Keyframe::at(0.0, 0.0), Keyframe::at(1.0, 1.0)],
        )
        .track(
            MotionChannel::TranslateY,
            [Keyframe::at(0.0, 12.0), Keyframe::at(1.0, 0.0)],
        )
        .track(
            MotionChannel::Scale,
            [Keyframe::at(0.0, 0.97), Keyframe::at(1.0, 1.0)],
        )
        .spring(panel_spring())
}

/// Returns `0.0` when system reduced motion is enabled so transitions snap immediately.
pub fn dur(ms: f32) -> f32 {
    if reduce_motion() { 0.0 } else { ms }
}

/// Applies the standard button press scale unless reduced motion is active.
pub fn press_scale(style: StyleRefinement, scale: f32) -> StyleRefinement {
    style.scale(move || if reduce_motion() { 1.0 } else { scale })
}
