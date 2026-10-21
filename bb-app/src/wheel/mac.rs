//! macOS: whether the wheel event being handled is a stepped wheel's, handed on as
//! pixels.
//!
//! macOS gives pixels ("precise" deltas) for a touchpad, a Magic Mouse and anything
//! posted in pixel units, and lines for a wheel that clicks. A touch surface's scroll
//! comes in phases (began, changed, ended, then momentum). A scroll in pixels without
//! any phase is a wheel's step posted in pixels: Parsec's host posts its client's wheel
//! so (`CGEventCreateScrollWheelEvent` in `kCGScrollEventUnitPixel` with the message's
//! delta unscaled, disassembled from its `parsecd` library, 2026-10-21).
//!
//! winit hands the event's delta on from inside AppKit's dispatch of it, so the event
//! is `NSApp.currentEvent` then.

use objc2::{MainThreadMarker, msg_send};
use objc2_app_kit::{NSApplication, NSEventPhase, NSEventType};

/// Whether the event being handled is a scroll in pixels without a phase.
pub(super) fn stepped() -> bool {
	let Some(mtm) = MainThreadMarker::new() else { return false };
	let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else { return false };
	event.r#type() == NSEventType::ScrollWheel
		&& event.hasPreciseScrollingDeltas()
		&& event.phase() == NSEventPhase::None
		&& event.momentumPhase() == NSEventPhase::None
}

/// The event being handled, for `BB_INPUT_LOG`: its type, whether precise, its phases
/// and deltas.
pub(super) fn describe() -> String {
	let Some(mtm) = MainThreadMarker::new() else { return "not on the main thread".into() };
	let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else { return "no current event".into() };
	if event.r#type() != NSEventType::ScrollWheel {
		return format!("current event of type {:?}", event.r#type());
	}
	// SAFETY: both return a CGFloat (f64 on 64-bit).
	let (scrolling, delta): (f64, f64) = unsafe { (msg_send![&event, scrollingDeltaY], msg_send![&event, deltaY]) };
	format!(
		"precise {} phase {:?} momentum {:?} scrollingDeltaY {scrolling} deltaY {delta}",
		event.hasPreciseScrollingDeltas(),
		event.phase(),
		event.momentumPhase(),
	)
}
