//! macOS: the cursor as an `NSCursor` made here, set on the window by us alone.
//!
//! The picture's pixels (enlarged by the backing factor first, see [`super`]) go into a
//! bitmap whose size in points is pixels / scale, so the display shows each pixel once
//! (and the hotspot is in points too). Its alpha is
//! straight, not premultiplied, as the pictures are.
//!
//! winit sets its own cursor from its view's cursor rectangle (`resetCursorRects`): its
//! arrow, or a transparent cursor while hidden. It takes no `NSCursor` from outside
//! (its own custom cursors are sized in points as pixels). A view of ours over it with
//! a rectangle of our cursor lost to winit's: AppKit set winit's arrow (28x40 points,
//! `NSCursor.arrowCursor` on macOS 26) on every mouse move, and ours showed only as it
//! was set (as `BB_INPUT_LOG` showed). So the window's cursor rectangles are off
//! (`disableCursorRects`), and [`Owner`] sets the cursor itself, again whenever
//! something else has replaced it ([`Owner::keep`], on the pointer's moves, its
//! entering and the window's focus).

use std::cell::RefCell;
use std::ptr::NonNull;

use bb_assets::CursorImage;
use objc2::encode::{Encode, Encoding};
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{AllocAnyThread, MainThreadMarker, msg_send};
use objc2_app_kit::{NSBitmapFormat, NSBitmapImageRep, NSCursor, NSDeviceRGBColorSpace, NSImage, NSView, NSWindow};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// `CGPoint`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
	x: f64,
	y: f64,
}

// SAFETY: `repr(C)` with CGPoint's fields and encoding.
unsafe impl Encode for Point {
	const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}

/// `CGSize`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
	width: f64,
	height: f64,
}

// SAFETY: `repr(C)` with CGSize's fields and encoding.
unsafe impl Encode for Size {
	const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}

pub(super) type Cursor = Retained<NSCursor>;

/// `image` as a cursor for a display of `scale`.
pub(super) fn cursor(image: &CursorImage, scale: f64) -> Result<Cursor, String> {
	let (width, height) = (image.width as usize, image.height as usize);
	if width == 0 || height == 0 || image.rgba.len() != width * height * 4 || !(scale > 0.0) {
		return Err(format!("a {width}x{height} cursor picture of {} bytes at scale {scale}", image.rgba.len()));
	}
	// SAFETY: null planes (the rep allocates its own), and sizes that agree with each other.
	let rep = unsafe {
		NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
			NSBitmapImageRep::alloc(),
			std::ptr::null_mut(),
			width as isize,
			height as isize,
			8,
			4,
			true,
			false,
			NSDeviceRGBColorSpace,
			NSBitmapFormat::AlphaNonpremultiplied,
			(width * 4) as isize,
			32,
		)
	}
	.ok_or("AppKit made no bitmap")?;
	// SAFETY: the rep's own buffer of `bytesPerRow * pixelsHigh` bytes, as asked above.
	unsafe { std::ptr::copy_nonoverlapping(image.rgba.as_ptr(), rep.bitmapData(), image.rgba.len()) };
	let points = Size {
		width: width as f64 / scale,
		height: height as f64 / scale,
	};
	// SAFETY: `setSize:` and `initWithSize:` take a CGSize, `addRepresentation:` an image
	// rep, `initWithImage:hotSpot:` an image and a CGPoint.
	unsafe {
		let _: () = msg_send![&rep, setSize: points];
		let picture: Retained<NSImage> = msg_send![NSImage::alloc(), initWithSize: points];
		picture.addRepresentation(&rep);
		let hotspot = Point {
			x: f64::from(image.hotspot.0) / scale,
			y: f64::from(image.hotspot.1) / scale,
		};
		let cursor: Option<Retained<NSCursor>> = msg_send![NSCursor::alloc(), initWithImage: &*picture, hotSpot: hotspot];
		cursor.ok_or_else(|| "AppKit made no cursor".to_string())
	}
}

/// The window's cursor, ours alone.
pub(super) struct Owner {
	window: Retained<NSWindow>,
	/// A transparent cursor, for none.
	hidden: Cursor,
	/// The cursor shown now.
	shown: RefCell<Cursor>,
}

impl Owner {
	pub(super) fn new(window: &Window) -> Result<Owner, String> {
		MainThreadMarker::new().ok_or("not on the main thread")?;
		let handle = window.window_handle().map_err(|e| e.to_string())?;
		let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
			return Err("not an AppKit window".into());
		};
		let view: NonNull<NSView> = handle.ns_view.cast();
		// SAFETY: winit's view, alive as long as the window (which outlives this).
		let view = unsafe { view.as_ref() };
		let window = view.window().ok_or("winit's view is in no window")?;
		let clear = CursorImage {
			width: 1,
			height: 1,
			hotspot: (0, 0),
			rgba: vec![0; 4],
		};
		let hidden = cursor(&clear, 1.0)?;
		window.disableCursorRects();
		Ok(Owner {
			window,
			shown: RefCell::new(hidden.clone()),
			hidden,
		})
	}

	/// Show `cursor` over the window, or none (a transparent one).
	pub(super) fn show(&self, cursor: Option<&Cursor>) {
		let cursor = cursor.unwrap_or(&self.hidden);
		*self.shown.borrow_mut() = cursor.clone();
		self.keep();
	}

	/// Set the cursor shown again if something else replaced it (another app's while
	/// the pointer was away, or AppKit's cursor rectangles if they came back on).
	/// Whether it had to.
	pub(super) fn keep(&self) -> bool {
		if self.window.areCursorRectsEnabled() {
			self.window.disableCursorRects();
		}
		let shown = self.shown.borrow();
		if std::ptr::eq(&**shown, &*NSCursor::currentCursor()) {
			return false;
		}
		shown.set();
		true
	}
}

/// The cursors now, for `BB_INPUT_LOG`: whether the app's current cursor is `shown`, and
/// the system's (what shows on the screen, from any app) size in points.
pub(super) fn describe(shown: Option<&Cursor>) -> String {
	let current = NSCursor::currentCursor();
	let ours = shown.is_some_and(|c| std::ptr::eq(&**c, &*current));
	#[allow(deprecated)]
	let system = NSCursor::currentSystemCursor().map(|c| {
		// SAFETY: `size` returns a CGSize.
		let size: Size = unsafe { msg_send![&c.image(), size] };
		format!("{}x{} points", size.width, size.height)
	});
	format!("the app's cursor is {}ours; the system's {}", if ours { "" } else { "not " }, system.unwrap_or_else(|| "none".into()))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A 32x32 picture on a 2x display is 16x16 points of 32x32 pixels, its hotspot in
	/// points too, and its colours as given (straight alpha).
	#[test]
	fn a_picture_is_its_pixels_at_any_scale() {
		let mut rgba = vec![0u8; 32 * 32 * 4];
		rgba[..4].copy_from_slice(&[200, 100, 50, 128]);
		let image = CursorImage {
			width: 32,
			height: 32,
			hotspot: (6, 10),
			rgba,
		};
		for scale in [1.0, 2.0] {
			let cursor = cursor(&image, scale).expect("a cursor");
			// SAFETY: `image` returns the cursor's NSImage, `size` a CGSize, `hotSpot` a
			// CGPoint, `representations` an array of image reps; `pixelsWide`/`pixelsHigh`
			// NSIntegers; `bitmapData` the rep's bytes.
			unsafe {
				let picture: Retained<NSImage> = msg_send![&cursor, image];
				let size: Size = msg_send![&picture, size];
				assert_eq!((size.width, size.height), (32.0 / scale, 32.0 / scale));
				let hotspot: Point = msg_send![&cursor, hotSpot];
				assert_eq!((hotspot.x, hotspot.y), (6.0 / scale, 10.0 / scale));
				let reps: Retained<NSObject> = msg_send![&picture, representations];
				let rep: Retained<NSBitmapImageRep> = msg_send![&reps, firstObject];
				let (wide, high): (isize, isize) = (msg_send![&rep, pixelsWide], msg_send![&rep, pixelsHigh]);
				assert_eq!((wide, high), (32, 32));
				assert_eq!(std::slice::from_raw_parts(rep.bitmapData(), 4), [200, 100, 50, 128]);
			}
		}
	}

	/// The game's screen is in points on a Retina display: the picture enlarged by the
	/// backing factor (each pixel copied, `super::enlarged`) shows at its own size in
	/// points, every display pixel its own: 32 points of 64x64 pixels at 2x.
	#[test]
	fn an_enlarged_picture_is_its_size_in_points() {
		let image = CursorImage {
			width: 32,
			height: 32,
			hotspot: (6, 10),
			rgba: vec![255; 32 * 32 * 4],
		};
		let cursor = cursor(&super::super::enlarged(&image, 2), 2.0).expect("a cursor");
		// SAFETY: as in the test above.
		unsafe {
			let picture: Retained<NSImage> = msg_send![&cursor, image];
			let size: Size = msg_send![&picture, size];
			assert_eq!((size.width, size.height), (32.0, 32.0));
			let hotspot: Point = msg_send![&cursor, hotSpot];
			assert_eq!((hotspot.x, hotspot.y), (6.0, 10.0));
			let reps: Retained<NSObject> = msg_send![&picture, representations];
			let rep: Retained<NSBitmapImageRep> = msg_send![&reps, firstObject];
			let (wide, high): (isize, isize) = (msg_send![&rep, pixelsWide], msg_send![&rep, pixelsHigh]);
			assert_eq!((wide, high), (64, 64));
		}
	}
}
