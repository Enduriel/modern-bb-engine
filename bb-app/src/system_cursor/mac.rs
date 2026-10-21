//! macOS: the cursor as an `NSCursor` made here, shown over winit's view by a view of
//! our own.
//!
//! The picture's pixels (enlarged by the backing factor first, see [`super`]) go into a
//! bitmap whose size in points is pixels / scale, so the display shows each pixel once
//! (and the hotspot is in points too). Its alpha is
//! straight, not premultiplied, as the pictures are.
//!
//! winit sets its own cursor from its view's cursor rectangle (`resetCursorRects`), and
//! takes no `NSCursor` from outside. So a transparent view of ours lies over it, which
//! lets every mouse event through (`hitTest:` finds nothing) and puts a cursor rectangle
//! over the whole window with the cursor shown, or a transparent one for none (see
//! [`Overlay::show`]).

use std::cell::RefCell;
use std::ptr::NonNull;

use bb_assets::CursorImage;
use objc2::encode::{Encode, Encoding};
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSBitmapFormat, NSBitmapImageRep, NSCursor, NSDeviceRGBColorSpace, NSImage, NSResponder, NSView};
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

/// `CGRect`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
	origin: Point,
	size: Size,
}

// SAFETY: `repr(C)` with CGRect's fields and encoding.
unsafe impl Encode for Rect {
	const ENCODING: Encoding = Encoding::Struct("CGRect", &[Point::ENCODING, Size::ENCODING]);
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

define_class!(
	// SAFETY: NSView may be subclassed; this one has no `Drop`.
	#[unsafe(super(NSView, NSResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	#[name = "BbCursorView"]
	#[ivars = RefCell<Option<Cursor>>]
	struct CursorView;

	impl CursorView {
		/// No mouse event is ours: they go to the view below (winit's).
		#[unsafe(method(hitTest:))]
		fn hit_test(&self, _point: Point) -> *mut NSView {
			std::ptr::null_mut()
		}

		#[unsafe(method(resetCursorRects))]
		fn reset_cursor_rects(&self) {
			if let Some(cursor) = &*self.ivars().borrow() {
				// SAFETY: `bounds` returns a CGRect; `addCursorRect:cursor:` takes one and a
				// cursor, and may be called from `resetCursorRects`.
				unsafe {
					let bounds: Rect = msg_send![self, bounds];
					let _: () = msg_send![self, addCursorRect: bounds, cursor: &**cursor];
				}
			}
		}
	}
);

/// Our view over winit's.
pub(super) struct Overlay {
	view: Retained<CursorView>,
	/// A transparent cursor, for none.
	hidden: Cursor,
}

impl Overlay {
	pub(super) fn new(window: &Window) -> Result<Overlay, String> {
		let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;
		let handle = window.window_handle().map_err(|e| e.to_string())?;
		let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
			return Err("not an AppKit window".into());
		};
		let parent: NonNull<NSView> = handle.ns_view.cast();
		// SAFETY: winit's view, alive as long as the window (which outlives this).
		let parent = unsafe { parent.as_ref() };
		let this = CursorView::alloc(mtm).set_ivars(RefCell::new(None));
		// SAFETY: `bounds` returns a CGRect, NSView's `initWithFrame:` takes one.
		let view: Retained<CursorView> = unsafe {
			let bounds: Rect = msg_send![parent, bounds];
			msg_send![super(this), initWithFrame: bounds]
		};
		view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
		parent.addSubview(&view);
		let clear = CursorImage {
			width: 1,
			height: 1,
			hotspot: (0, 0),
			rgba: vec![0; 4],
		};
		Ok(Overlay {
			view,
			hidden: cursor(&clear, 1.0)?,
		})
	}

	/// Show `cursor` over the whole window, or none (a transparent one).
	///
	/// The overlay's cursor rectangle is always there, so winit's never decides: winit
	/// re-adds its own whenever its cursor is shown or hidden, and AppKit doesn't say
	/// which of two overlapping rectangles wins. winit's stays as it started (its
	/// arrow, never hidden).
	pub(super) fn show(&self, cursor: Option<&Cursor>) {
		let cursor = cursor.unwrap_or(&self.hidden);
		*self.view.ivars().borrow_mut() = Some(cursor.clone());
		if let Some(window) = self.view.window() {
			window.invalidateCursorRectsForView(&self.view);
		}
		// At once, not at the next mouse move.
		cursor.set();
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

impl Drop for Overlay {
	fn drop(&mut self) {
		self.view.removeFromSuperview();
	}
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
