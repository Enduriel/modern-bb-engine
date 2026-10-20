//! The grid as the world entities' natives read it: straight from the store, as the
//! original's natives read their C++ map, not through the script-visible `World`
//! functions (`World.getTile`, `worldToTile`, a tile's `Type`), which a mod may replace
//! and which cost a script-level call each.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef};

use super::{GridState, WorldGrid, tile_at};
use crate::native::world_layout;

/// A shared handle on the world grid for other natives (see the module docs).
#[derive(Clone)]
pub(crate) struct GridAccess(Rc<RefCell<GridState>>);

impl GridAccess {
	/// The `WorldTile` at axial `(q, r)`, as `World.getTile(q, r)` gives it, or `None`
	/// off the map.
	pub(crate) fn tile(&self, vm: &SQVM, q: i64, r: i64) -> Option<SqStrongRef> {
		tile_at(&self.0, vm, q, r + q / 2)
	}

	/// The `Type` of the tile at axial `(q, r)`, or `None` off the map.
	pub(crate) fn terrain(&self, q: i64, r: i64) -> Option<i64> {
		let g = self.0.borrow();
		g.idx(q, r + q / 2).map(|id| g.tiles[id].type_)
	}

	/// The axial tile containing world pixel position `(x, y)`, as `World.worldToTile`.
	pub(crate) fn world_to_tile(&self, x: f64, y: f64) -> (i64, i64) {
		let (w, h) = {
			let g = self.0.borrow();
			(g.width, g.height)
		};
		world_layout::world_to_tile(x, y, w, h)
	}
}

impl WorldGrid {
	/// The grid for other natives to read directly (see [`GridAccess`]).
	pub(crate) fn access(&self) -> GridAccess {
		GridAccess(self.grid.clone())
	}
}
