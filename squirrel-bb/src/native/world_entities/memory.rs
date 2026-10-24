//! What the world's entities hold (see [`crate::memory_report`]).

use crate::memory_report::{MemoryReport, ReportMemory, map_bytes, vec_bytes};

use super::WorldEntities;

impl ReportMemory for WorldEntities {
	fn report_memory(&self, report: &mut MemoryReport) {
		let m = self.machine.borrow();
		let alive = m.entities.iter().filter(|e| e.alive).count();
		let owned: usize = m
			.entities
			.iter()
			.map(|e| e.path.capacity() + vec_bytes(&e.identities) + map_bytes(&e.labels))
			.sum();
		report.add("world entities", "records", m.entities.len(), vec_bytes(&m.entities) + owned);
		report.add("world entities", "records alive", alive, 0);
		report.add("world entities", "records dead, slot not free", m.dead.len(), vec_bytes(&m.dead));
		report.add("world entities", "identity index", m.by_identity.len(), map_bytes(&m.by_identity));
		report.add("world entities", "id index", m.by_id.len(), map_bytes(&m.by_id));
	}
}
