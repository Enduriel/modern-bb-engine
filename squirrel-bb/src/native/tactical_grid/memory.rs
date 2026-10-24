//! What the battle's map holds (see [`crate::memory_report`]).

use crate::memory_report::{MemoryReport, ReportMemory, vec_bytes};

use super::TacticalGrid;

impl ReportMemory for TacticalGrid {
	fn report_memory(&self, report: &mut MemoryReport) {
		let g = self.grid.borrow();
		report.add("battle map", "tiles", g.tiles.len(), vec_bytes(&g.tiles));
	}
}
