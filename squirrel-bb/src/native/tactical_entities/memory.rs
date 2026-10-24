//! What the battle's units hold (see [`crate::memory_report`]).

use crate::memory_report::{MemoryReport, ReportMemory, map_bytes, vec_bytes};

use super::TacticalEntities;

impl ReportMemory for TacticalEntities {
	fn report_memory(&self, report: &mut MemoryReport) {
		let m = self.machine.borrow();
		let dead = m.units.iter().filter(|u| u.dead).count();
		let paths: usize = m.units.iter().map(|u| u.path.capacity()).sum();
		report.add("battle units", "records", m.units.len(), vec_bytes(&m.units) + paths);
		report.add("battle units", "records of dead units", dead, 0);
		report.add("battle units", "identity index", m.by_identity.len(), map_bytes(&m.by_identity));
		let members: usize = m.rosters.values().map(Vec::len).sum();
		let roster_bytes: usize = map_bytes(&m.rosters) + m.rosters.values().map(vec_bytes).sum::<usize>();
		report.add("battle units", "roster members", members, roster_bytes);
		report.add("battle units", "shakes", m.shakes.len() + m.ended_shakes.len(), vec_bytes(&m.shakes) + vec_bytes(&m.ended_shakes));
	}
}
