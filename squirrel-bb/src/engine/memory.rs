//! What the engine's services hold, together (see [`crate::memory_report`]).

use crate::memory_report::{MemoryReport, ReportMemory};
use crate::source::ScriptSource;

use super::Engine;

impl<S: ScriptSource + 'static> ReportMemory for Engine<S> {
	fn report_memory(&self, report: &mut MemoryReport) {
		self.modules.report_memory(report);
		self.sprites.report_memory(report);
		self.world_grid.report_memory(report);
		self.world_entities.report_memory(report);
		self.tactical_grid.report_memory(report);
		self.tactical_entities.report_memory(report);
		self.ui.report_memory(report);
		self.audio.report_memory(report);
		let (saves, bytes) = self.saves.memory_use();
		report.add("saves", "kept in memory", saves, bytes);
	}
}
