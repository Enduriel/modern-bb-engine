//! Comparing two campaign saves of the original's layout the way the original's own
//! load and save leave one (probed): equal but for the header's times and the order
//! of the scene's quadtree layers, which follows the order entities were added in (the
//! original re-adds them on load, and some land elsewhere).

use super::super::container;
use super::scene::{Layer, LayerBody, Scene};
use super::super::stream::SaveReader;

/// A save's parts: its metadata, its virtual time, its scene with each quadtree layer by
/// id, and the bytes after the scene.
struct Parts {
	meta: super::super::meta::Meta,
	vtime: f64,
	scene: Scene,
	rest: Vec<u8>,
}

fn parts(bytes: &[u8], what: &str) -> Result<Parts, String> {
	let (meta, mut r): (_, SaveReader) = container::decode(bytes.to_vec(), what)?;
	let vtime = r.f64()?;
	let mut scene = Scene::read(&mut r)?;
	for layer in &mut scene.layers {
		if let Some(LayerBody::QuadTree(q)) = &mut layer.body {
			q.entities.sort_by_key(|e| e.head.id);
		}
	}
	let rest = bytes[r.pos()..].to_vec();
	Ok(Parts { meta, vtime, scene, rest })
}

fn layer_differences(a: &Layer, b: &Layer, out: &mut Vec<String>) {
	let name = String::from_utf8_lossy(&a.name).into_owned();
	if (a.u2c, &a.name, a.type_, a.flags) != (b.u2c, &b.name, b.type_, b.flags) {
		out.push(format!("layer {name}: its header differs"));
		return;
	}
	match (&a.body, &b.body) {
		(Some(LayerBody::QuadTree(x)), Some(LayerBody::QuadTree(y))) => {
			if (x.depth, x.bounds) != (y.depth, y.bounds) {
				out.push(format!("layer {name}: depth/bounds {:?} vs {:?}", (x.depth, x.bounds), (y.depth, y.bounds)));
			}
			if x.entities.len() != y.entities.len() {
				out.push(format!("layer {name}: {} vs {} entities", x.entities.len(), y.entities.len()));
			}
			let differing: Vec<_> = x.entities.iter().zip(&y.entities).filter(|(e, f)| e != f).collect();
			if !differing.is_empty() {
				out.push(format!("layer {name}: {} entities differ", differing.len()));
				for (e, f) in differing.into_iter().take(3) {
					out.push(format!("  {e:?}\n  vs {f:?}"));
				}
			}
		}
		(Some(LayerBody::Tiles(x)), Some(LayerBody::Tiles(y))) => {
			if (x.metrics, x.i4c, x.u5c, &x.mask) != (y.metrics, y.i4c, y.u5c, &y.mask) {
				out.push(format!("layer {name}: its properties differ"));
			}
			match (&x.tiles, &y.tiles) {
				(Some((dx, nx)), Some((dy, ny))) => {
					let differing = dx.iter().zip(dy).filter(|(p, q)| p != q).count();
					if differing > 0 || dx.len() != dy.len() {
						let first = dx.iter().zip(dy).position(|(p, q)| p != q);
						out.push(format!(
							"layer {name}: {differing} terrain tiles differ (first: {:?})",
							first.map(|i| (i, dx[i], dy[i]))
						));
					}
					if nx != ny {
						out.push(format!("layer {name}: its nested layers differ"));
					}
				}
				(t, u) if t != u => out.push(format!("layer {name}: one has terrain tiles, one doesn't")),
				_ => {}
			}
		}
		(p, q) if p != q => out.push(format!("layer {name}: its contents differ")),
		_ => {}
	}
}

/// How two campaign saves of the original's layout differ, as above. Empty: they're the
/// same save.
/// The name a campaign save of the original's layout was saved under (its metadata's).
pub fn original_save_name(bytes: &[u8]) -> Result<String, String> {
	let (meta, _): (super::super::meta::Meta, SaveReader) = container::decode(bytes.to_vec(), "save")?;
	Ok(meta.name)
}

pub fn compare_original_saves(a: &[u8], b: &[u8]) -> Result<Vec<String>, String> {
	let (a, b) = (parts(a, "first save")?, parts(b, "second save")?);
	let mut out = Vec::new();
	let label = |m: &super::super::meta::Meta| (m.version, m.name.clone(), m.data.clone());
	if label(&a.meta) != label(&b.meta) {
		out.push(format!("metadata: {:?} vs {:?}", label(&a.meta), label(&b.meta)));
	}
	if a.vtime.to_bits() != b.vtime.to_bits() {
		out.push(format!("virtual time {} vs {}", a.vtime, b.vtime));
	}
	let (s, t) = (&a.scene, &b.scene);
	let header = |s: &Scene| (s.d4, s.name.clone(), s.ambient_color, s.ambient_saturation.to_bits(), s.b8, s.zeros, s.r28, s.e0);
	if header(s) != header(t) {
		out.push(format!("scene header: {:?} vs {:?}", header(s), header(t)));
	}
	match (&s.grid, &t.grid) {
		(Some(g), Some(h)) => {
			if (g.origin, g.width, g.height) != (h.origin, h.width, h.height) {
				out.push(format!("grid {:?} vs {:?}", (g.origin, g.width, g.height), (h.origin, h.width, h.height)));
			} else {
				let differing: Vec<usize> = (0..g.tiles.len()).filter(|&i| g.tiles[i] != h.tiles[i]).collect();
				if let Some(&i) = differing.first() {
					out.push(format!("{} tiles differ (first, #{i}: {:?} vs {:?})", differing.len(), g.tiles[i], h.tiles[i]));
				}
			}
		}
		(g, h) if g.is_some() != h.is_some() => out.push("one has a grid, one doesn't".to_string()),
		_ => {}
	}
	if s.layers.len() != t.layers.len() {
		out.push(format!("{} vs {} layers", s.layers.len(), t.layers.len()));
	}
	for (x, y) in s.layers.iter().zip(&t.layers) {
		layer_differences(x, y, &mut out);
	}
	if a.rest != b.rest {
		let first = a.rest.iter().zip(&b.rest).position(|(x, y)| x != y).unwrap_or(a.rest.len().min(b.rest.len()));
		let differing = a.rest.iter().zip(&b.rest).filter(|(x, y)| x != y).count();
		out.push(format!(
			"after the scene: {} vs {} bytes, {differing} differ, the first at +{first} ({:02x?} vs {:02x?})",
			a.rest.len(),
			b.rest.len(),
			&a.rest[first..(first + 16).min(a.rest.len())],
			&b.rest[first..(first + 16).min(b.rest.len())],
		));
	}
	Ok(out)
}
