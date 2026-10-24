//! A memory-infra trace read (see [`super::memory_dump`]): the events as a Chrome JSON
//! trace, and each process's figures from its memory dump, as `parity/perf/cdp_memory.py`
//! reads them.

use crate::json::Json;

/// One process's memory as Chromium counts it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChromiumProcess {
	pub pid: i64,
	/// Its name in the trace (`Renderer`, `GPU Process`, `Browser`, …).
	pub name: String,
	/// The process totals (`resident_set_bytes`, `private_footprint_bytes`, …).
	pub totals: Vec<(String, u64)>,
	/// Every allocator dump's size, by its path (`malloc`, `v8/main/heap`, …), largest
	/// first; the top-level ones are those without a `/`.
	pub allocators: Vec<(String, u64)>,
}

/// The trace text, the processes, and what couldn't be read, from the
/// `Tracing.dataCollected` params `collected`.
pub(super) fn read(collected: &[Vec<u8>]) -> (String, Vec<ChromiumProcess>, Vec<String>) {
	let mut errors = Vec::new();
	let mut events = Vec::new();
	for params in collected {
		match Json::parse(&String::from_utf8_lossy(params)) {
			Ok(Json::Object(mut fields)) => match fields.iter().position(|(k, _)| k == "value") {
				Some(i) => match fields.swap_remove(i).1 {
					Json::Array(value) => events.extend(value),
					_ => errors.push("a trace chunk without events".into()),
				},
				None => errors.push("a trace chunk without events".into()),
			},
			Ok(_) => errors.push("a trace chunk that isn't an object".into()),
			Err(e) => errors.push(format!("a trace chunk: {e}")),
		}
	}
	let mut trace = String::from("{\"traceEvents\":[");
	for (i, event) in events.iter().enumerate() {
		if i > 0 {
			trace.push(',');
		}
		event.write(&mut trace);
	}
	trace.push_str("]}\n");
	(trace, processes(&events), errors)
}

/// Each process with a memory dump in `events`, by pid.
fn processes(events: &[Json]) -> Vec<ChromiumProcess> {
	let pid = |e: &Json| e.get("pid").and_then(Json::as_i64);
	let mut out: Vec<ChromiumProcess> = Vec::new();
	for e in events {
		let (Some(pid), Some(dumps)) = (pid(e), e.get("args").and_then(|a| a.get("dumps"))) else { continue };
		if e.get("ph").and_then(Json::as_str) != Some("v") {
			continue;
		}
		let mut process = ChromiumProcess { pid, ..Default::default() };
		if let Some(Json::Object(totals)) = dumps.get("process_totals") {
			process.totals = totals.iter().filter_map(|(k, v)| Some((k.clone(), number(v)?))).collect();
		}
		if let Some(Json::Object(allocators)) = dumps.get("allocators") {
			process.allocators = allocators
				.iter()
				.filter_map(|(name, d)| Some((name.clone(), number(d.get("attrs")?.get("size")?.get("value")?)?)))
				.collect();
			process.allocators.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
		}
		match out.iter_mut().find(|p| p.pid == pid) {
			Some(p) => *p = process,
			None => out.push(process),
		}
	}
	for e in events {
		let is_name = e.get("ph").and_then(Json::as_str) == Some("M") && e.get("name").and_then(Json::as_str) == Some("process_name");
		let name = e.get("args").and_then(|a| a.get("name")).and_then(Json::as_str);
		if let (true, Some(pid), Some(name)) = (is_name, pid(e), name)
			&& let Some(p) = out.iter_mut().find(|p| p.pid == pid)
		{
			p.name = name.to_string();
		}
	}
	out
}

/// A size as the trace writes it: a hex string (`"1a2b"`) or a number.
fn number(v: &Json) -> Option<u64> {
	match v {
		Json::Str(hex) => u64::from_str_radix(hex.trim_start_matches("0x"), 16).ok(),
		Json::Int(n) => u64::try_from(*n).ok(),
		Json::Float(f) if *f >= 0.0 => Some(*f as u64),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_a_dump() {
		let chunk = br#"{"value":[
			{"ph":"M","pid":7,"name":"process_name","args":{"name":"Renderer"}},
			{"ph":"v","pid":7,"name":"periodic_interval","args":{"dumps":{
				"process_totals":{"private_footprint_bytes":"100000"},
				"allocators":{
					"malloc":{"attrs":{"size":{"type":"scalar","units":"bytes","value":"200000"}}},
					"v8/main":{"attrs":{"size":{"type":"scalar","units":"bytes","value":"10"}}},
					"cc":{"attrs":{}}
				}}}}
		]}"#;
		let (trace, processes, errors) = read(&[chunk.to_vec(), b"oops".to_vec()]);
		assert_eq!(errors.len(), 1, "{errors:?}");
		assert!(trace.starts_with("{\"traceEvents\":[{"), "{trace}");
		assert_eq!(processes.len(), 1);
		let p = &processes[0];
		assert_eq!((p.pid, p.name.as_str()), (7, "Renderer"));
		assert_eq!(p.totals, [("private_footprint_bytes".to_string(), 0x100000)]);
		assert_eq!(p.allocators, [("malloc".to_string(), 0x200000), ("v8/main".to_string(), 0x10)]);
	}
}
