//! What the game needs of the machine, checked at start, and what the player is told
//! when something is missing: the GPU, its graphics API and driver, Chromium's GPU paint.
//!
//! The UI page is a shared GPU texture with no software path, so a
//! machine without a usable GPU can't show the game. Such a start ends at once with a
//! message naming what is missing and pointing at the minimum requirements
//! (`docs/requirements.md`): on stderr, in `log.html`, and in a message box ([`dialog`])
//! instead of a window that flashes or a crash seconds later.
//!
//! The failures are errors like any other on their way out of the game's start, marked
//! with [`MARK`] ([`unsupported`]) so that `main` knows to explain them ([`explain`]).

pub mod dialog;

/// What starts the error of a start that failed on the hardware ([`unsupported`]).
pub const MARK: &str = "this computer can't run the game: ";

/// An error for a start that failed because the machine lacks `what`.
pub fn unsupported(what: impl std::fmt::Display) -> String {
	format!("{MARK}{what}")
}

/// What the machine lacked, if `error` is a hardware failure ([`unsupported`]).
pub fn missing(error: &str) -> Option<&str> {
	error.find(MARK).map(|at| &error[at + MARK.len()..])
}

/// The minimum requirements for this operating system, in a sentence or two.
pub fn minimum() -> &'static str {
	if cfg!(target_os = "linux") {
		"On Linux the game needs a GPU with a Vulkan driver that can import DMA-BUF buffers \
		 (VK_EXT_external_memory_dma_buf): AMD GCN 1.0 (Radeon HD 7000, 2012) or newer on \
		 Mesa's RADV, Intel Broadwell (5th generation Core, 2014) or newer on Mesa's ANV, or \
		 NVIDIA Maxwell (GeForce GTX 750/900, 2014) or newer on NVIDIA's driver 515 or later. \
		 OpenGL alone, software renderers (lavapipe, llvmpipe) and virtual machines without \
		 a GPU passed through aren't enough."
	} else if cfg!(target_os = "macos") {
		"On macOS the game needs a Mac with Metal, on a macOS version Chromium supports."
	} else if cfg!(windows) {
		"Windows isn't supported yet: for now the game runs on Linux and macOS."
	} else {
		"This operating system isn't supported yet: for now the game runs on Linux and macOS."
	}
}

/// The player's message for a start that failed on the hardware: what was missing, the
/// minimum requirements, and where the details are. `None` for any other error.
pub fn explain(error: &str, logs: Option<&std::path::Path>) -> Option<String> {
	let missing = missing(error)?;
	let mut text = format!("Battle Brothers can't start on this computer: {missing}.\n\n{}\n\n", minimum());
	text.push_str("The minimum requirements are in requirements.md (docs/ in the source).");
	if let Some(logs) = logs {
		text.push_str(&format!("\nDetails: log.html and cef.log in {}", logs.display()));
	}
	Some(text)
}

/// Whether `gpu` can run the game with its UI page: a hardware GPU on this system's API
/// for shared textures, with the device features the page needs, and (Linux) a render
/// node for Chromium's GPU buffers. The first thing missing, as an [`unsupported`] error.
#[cfg(feature = "ui")]
pub fn check_gpu(gpu: &bb_render::Gpu) -> Result<(), String> {
	if gpu.device_type == wgpu::DeviceType::Cpu {
		return Err(unsupported(format!(
			"no hardware GPU was found, only the software renderer {}",
			gpu.adapter
		)));
	}
	#[cfg(target_os = "linux")]
	{
		if gpu.backend != wgpu::Backend::Vulkan {
			return Err(unsupported(format!(
				"the GPU has no usable Vulkan driver ({} runs through {:?} only)",
				gpu.adapter, gpu.backend
			)));
		}
		let missing = bb_render::UiPage::FEATURES - gpu.device.features();
		if !missing.is_empty() {
			return Err(unsupported(format!(
				"the GPU's Vulkan driver ({}) can't import DMA-BUF buffers (VK_EXT_external_memory_dma_buf), \
				 which the UI needs",
				gpu.adapter
			)));
		}
		if !render_node() {
			return Err(unsupported(
				"no GPU render node (/dev/dri/renderD*) is visible to the game, which Chromium needs to draw the UI",
			));
		}
	}
	#[cfg(target_os = "macos")]
	if gpu.backend != wgpu::Backend::Metal {
		return Err(unsupported(format!("the GPU has no Metal device ({} runs through {:?})", gpu.adapter, gpu.backend)));
	}
	#[cfg(not(any(target_os = "linux", target_os = "macos")))]
	return Err(unsupported("the game's UI can't be drawn on the GPU on this operating system yet"));
	#[cfg(any(target_os = "linux", target_os = "macos"))]
	Ok(())
}

/// Whether a DRM render node is visible (where Chromium allocates its GPU buffers, GBM).
#[cfg(all(feature = "ui", target_os = "linux"))]
fn render_node() -> bool {
	std::fs::read_dir("/dev/dri")
		.into_iter()
		.flatten()
		.flatten()
		.any(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
}

/// An error from opening the UI page, as the player's failure where it is one: the
/// browser engine didn't start, or its GPU never painted the page.
#[cfg(feature = "ui")]
pub fn page_error(error: String) -> String {
	if let Some(at) = error.find(bb_ui::host::NO_PAINTS) {
		let why = error[at + bb_ui::host::NO_PAINTS.len()..].trim_start_matches(": ");
		unsupported(format!("Chromium couldn't draw the UI on the GPU ({why})"))
	} else if error.contains(bb_ui::host::CEF_FAILED) {
		unsupported(format!("the browser engine for the UI couldn't start ({error})"))
	} else {
		error
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_hardware_failure_is_explained_with_the_requirements_and_the_logs() {
		let error = unsupported("no hardware GPU was found, only the software renderer llvmpipe (Vulkan)");
		assert_eq!(missing(&error), Some("no hardware GPU was found, only the software renderer llvmpipe (Vulkan)"));
		let text = explain(&error, Some(std::path::Path::new("/logs"))).expect("explained");
		assert!(text.starts_with("Battle Brothers can't start on this computer: no hardware GPU was found"), "{text}");
		assert!(text.contains(minimum()), "{text}");
		assert!(text.contains("requirements.md"), "{text}");
		assert!(text.ends_with("in /logs"), "{text}");
	}

	#[test]
	fn other_errors_are_left_alone() {
		assert_eq!(missing("unknown argument --foo"), None);
		assert_eq!(explain("unknown argument --foo", None), None);
	}

	#[test]
	fn a_wrapped_hardware_failure_is_still_one() {
		let error = format!("boot: {}", unsupported("no Vulkan"));
		assert_eq!(missing(&error), Some("no Vulkan"));
	}
}
