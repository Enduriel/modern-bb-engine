//! What the world map holds (see [`crate::memory_report`]).

use crate::memory_report::{MemoryReport, ReportMemory, vec_bytes};

use super::WorldGrid;

impl ReportMemory for WorldGrid {
	fn report_memory(&self, report: &mut MemoryReport) {
		let g = self.grid.borrow();
		report.add("world map", "tiles", g.tiles.len(), vec_bytes(&g.tiles));
		let details: usize = g.tiles.iter().map(|t| t.details.len()).sum();
		let detail_bytes: usize = g.tiles.iter().map(|t| vec_bytes(&t.details)).sum();
		report.add("world map", "details", details, detail_bytes);
		report.add("world map", "fog of war mask", 1, g.fog.alpha().len());
		report.add("world map", "region names", g.region_texts.len(), vec_bytes(&g.region_texts));
		report.add("world map", "edits not taken yet", g.edits.len(), vec_bytes(&g.edits));
	}
}
