//! Snapshot trees and their comparison.

use std::fmt;

use crate::json::Json;

/// One visible element of a snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
	pub tag: String,
	pub cls: String,
	pub text: Option<String>,
	/// `[x, y, width, height]` in CSS pixels.
	pub rect: [i64; 4],
	/// The recorded computed styles, in recording order.
	pub style: Vec<(String, String)>,
	/// An image the engine draws (`coui://procedural/…`: portraits).
	pub procedural: bool,
	pub children: Vec<Node>,
}

/// A whole snapshot: the viewport and the element tree under `body`.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
	pub viewport: (i64, i64),
	pub root: Node,
}

impl Snapshot {
	/// Read `BBSnapshot.take()`'s JSON.
	pub fn parse(json: &Json) -> Result<Snapshot, String> {
		let viewport = json
			.get("viewport")
			.and_then(Json::as_array)
			.and_then(|v| Some((v.first()?.as_i64()?, v.get(1)?.as_i64()?)))
			.ok_or("snapshot: no viewport")?;
		let root = node(json.get("root").ok_or("snapshot: no root")?)?;
		Ok(Snapshot { viewport, root })
	}

	/// The snapshot in `BBSnapshot.take()`'s format.
	pub fn to_json(&self) -> Json {
		Json::Object(vec![
			(
				"viewport".into(),
				Json::Array(vec![Json::Int(self.viewport.0), Json::Int(self.viewport.1)]),
			),
			("root".into(), node_json(&self.root)),
		])
	}
}

fn node_json(node: &Node) -> Json {
	let mut entries = vec![
		("tag".to_string(), Json::Str(node.tag.clone())),
		("cls".to_string(), Json::Str(node.cls.clone())),
	];
	if node.procedural {
		entries.push(("procedural".into(), Json::Bool(true)));
	}
	if let Some(text) = &node.text {
		entries.push(("text".into(), Json::Str(text.clone())));
	}
	entries.push((
		"rect".into(),
		Json::Array(node.rect.iter().map(|&v| Json::Int(v)).collect()),
	));
	entries.push((
		"style".into(),
		Json::Object(
			node.style
				.iter()
				.map(|(k, v)| (k.clone(), Json::Str(v.clone())))
				.collect(),
		),
	));
	entries.push((
		"children".into(),
		Json::Array(node.children.iter().map(node_json).collect()),
	));
	Json::Object(entries)
}

fn node(json: &Json) -> Result<Node, String> {
	let text = |key: &str| json.get(key).and_then(Json::as_str).map(str::to_string);
	let rect = json
		.get("rect")
		.and_then(Json::as_array)
		.map(|r| r.iter().filter_map(Json::as_i64).collect::<Vec<_>>())
		.and_then(|r| <[i64; 4]>::try_from(r).ok())
		.ok_or("snapshot: a node without a rect")?;
	let style = match json.get("style") {
		Some(Json::Object(entries)) => entries
			.iter()
			.filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
			.collect(),
		_ => Vec::new(),
	};
	let children = json
		.get("children")
		.and_then(Json::as_array)
		.unwrap_or_default()
		.iter()
		.map(node)
		.collect::<Result<_, _>>()?;
	Ok(Node {
		tag: text("tag").ok_or("snapshot: a node without a tag")?,
		cls: text("cls").unwrap_or_default(),
		text: text("text"),
		rect,
		style,
		procedural: json.get("procedural") == Some(&Json::Bool(true)),
		children,
	})
}

/// What a comparison lets pass.
#[derive(Clone, Debug)]
pub struct Tolerance {
	/// How far a box's edges may be off (CSS pixels).
	pub pixels: i64,
	/// Style properties not compared.
	pub ignore_styles: Vec<String>,
	/// Elements not compared, with their subtrees: those whose path contains one of
	/// these, e.g. a list whose content depends on the machine (display modes).
	pub ignore_paths: Vec<String>,
	/// Elements whose boxes aren't compared, nor their subtrees' (their text and styles
	/// still are): those whose path contains one of these, e.g. a list of text lines
	/// that differ in height by a known amount.
	pub ignore_boxes: Vec<String>,
	/// Whether to compare the boxes and styles of the images the engine draws
	/// ([`Node::procedural`]: portraits), whose sizes follow what they show. Off by
	/// default: we draw placeholders until the renderer draws them.
	pub procedural_images: bool,
	/// The page zoom the golden was taken at: the original's UI scale (`UIScale` 1.5
	/// is a page zoom of 1.5 there). Old WebKit reports `text-shadow` lengths divided by
	/// it.
	pub page_zoom: f64,
}

impl Default for Tolerance {
	fn default() -> Self {
		Tolerance {
			pixels: 2,
			ignore_styles: Vec::new(),
			ignore_paths: Vec::new(),
			ignore_boxes: Vec::new(),
			procedural_images: false,
			page_zoom: 1.0,
		}
	}
}

/// One way our page differs from the golden.
#[derive(Clone, Debug, PartialEq)]
pub enum Difference {
	/// In the golden, missing from our page.
	Missing { path: String },
	/// On our page, not in the golden.
	Extra { path: String },
	Text {
		path: String,
		golden: Option<String>,
		ours: Option<String>,
	},
	Rect {
		path: String,
		golden: [i64; 4],
		ours: [i64; 4],
	},
	Style {
		path: String,
		property: String,
		golden: String,
		ours: String,
	},
}

impl Difference {
	/// The path of the element that differs.
	pub fn path(&self) -> &str {
		match self {
			Difference::Missing { path }
			| Difference::Extra { path }
			| Difference::Text { path, .. }
			| Difference::Rect { path, .. }
			| Difference::Style { path, .. } => path,
		}
	}
}

impl fmt::Display for Difference {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Difference::Missing { path } => write!(f, "missing: {path}"),
			Difference::Extra { path } => write!(f, "extra: {path}"),
			Difference::Text { path, golden, ours } => {
				write!(f, "text: {path}: {golden:?} -> {ours:?}")
			}
			Difference::Rect { path, golden, ours } => {
				write!(f, "box: {path}: {golden:?} -> {ours:?}")
			}
			Difference::Style {
				path,
				property,
				golden,
				ours,
			} => write!(f, "style: {path}: {property}: {golden:?} -> {ours:?}"),
		}
	}
}

/// A style value in a form both engines agree on: quotes dropped, whitespace
/// collapsed, `rgb(r, g, b)` written as `rgba(r, g, b, 1)`.
pub fn normalize_style(value: &str) -> String {
	let unquoted: String = value.chars().filter(|c| *c != '"' && *c != '\'').collect();
	let collapsed = unquoted.split_whitespace().collect::<Vec<_>>().join(" ");
	let mut out = String::new();
	let mut rest = collapsed.as_str();
	while let Some(start) = rest.find("rgb(") {
		out.push_str(&rest[..start]);
		let after = &rest[start + 4..];
		match after.find(')') {
			Some(end) => {
				out.push_str("rgba(");
				out.push_str(&after[..end]);
				out.push_str(", 1)");
				rest = &after[end + 1..];
			}
			None => {
				out.push_str(&rest[start..]);
				rest = "";
			}
		}
	}
	out.push_str(rest);
	out
}

/// `property: value` of a `tag` element in the form both engines agree on:
/// [`normalize_style`], plus the values the engines report differently for the same
/// rendering: font weights by name or number, `display` values old WebKit reports with
/// the `-webkit-` prefix the CSS used (`-webkit-flex`), and the `overflow: clip`
/// Chromium's built-in style gives images (replaced elements don't overflow either way).
fn normalized(tag: &str, property: &str, value: &str) -> String {
	let value = normalize_style(value);
	match (property, value.as_str()) {
		("font-weight", "normal") => "400".into(),
		("font-weight", "bold") => "700".into(),
		("display", prefixed) if prefixed.starts_with("-webkit-") => {
			prefixed["-webkit-".len()..].into()
		}
		("overflow-x" | "overflow-y", "clip") if matches!(tag, "img" | "canvas" | "video") => {
			"visible".into()
		}
		_ => value,
	}
}

/// A length in pixels, if `value` is one (`"422.5px"`).
fn pixels(value: &str) -> Option<f64> {
	value.strip_suffix("px")?.parse().ok()
}

/// `value` split into its numbers and the text around them: `"rgba(0, 0, 0, 0.3)"` is
/// `"rgba(#, #, #, #)"` with 0, 0, 0 and 0.3.
fn numbers(value: &str) -> (String, Vec<f64>) {
	let bytes = value.as_bytes();
	let mut skeleton = String::new();
	let mut found = Vec::new();
	let mut i = 0;
	while i < bytes.len() {
		let digit_at = |j: usize| bytes.get(j).is_some_and(u8::is_ascii_digit);
		let starts = digit_at(i)
			|| (bytes[i] == b'.' && digit_at(i + 1))
			|| (bytes[i] == b'-'
				&& (digit_at(i + 1) || (bytes.get(i + 1) == Some(&b'.') && digit_at(i + 2))));
		if !starts {
			skeleton.push(bytes[i] as char);
			i += 1;
			continue;
		}
		let start = i;
		i += 1;
		while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
			i += 1;
		}
		match value[start..i].parse() {
			Ok(number) => {
				skeleton.push('#');
				found.push(number);
			}
			Err(_) => skeleton.push_str(&value[start..i]),
		}
	}
	(skeleton, found)
}

/// `value` with each pixel length multiplied by `zoom`.
fn zoomed_pixels(value: &str, zoom: f64) -> String {
	if zoom == 1.0 {
		return value.to_string();
	}
	let (skeleton, found) = numbers(value);
	let mut out = String::new();
	let mut found = found.into_iter();
	let mut rest = skeleton.as_str();
	while let Some(at) = rest.find('#') {
		out.push_str(&rest[..at]);
		rest = &rest[at + 1..];
		let number = found.next().unwrap_or(0.0);
		let number = if rest.starts_with("px") {
			number * zoom
		} else {
			number
		};
		out.push_str(&format!("{number}"));
	}
	out.push_str(rest);
	out
}

/// Whether `a` and `b` are the same apart from numbers within one 8-bit step (1/255)
/// of each other: old WebKit keeps colour alpha in 8 bits and opacity as a 32-bit
/// float, so it reports 0.3 as 0.298039 and 0.4 as 0.4000000059604645.
fn close_numbers(a: &str, b: &str) -> bool {
	let ((a_text, a_numbers), (b_text, b_numbers)) = (numbers(a), numbers(b));
	a_text == b_text
		&& a_numbers.len() == b_numbers.len()
		&& a_numbers
			.iter()
			.zip(&b_numbers)
			.all(|(x, y)| (x - y).abs() <= 1.0 / 255.0 + 1e-9)
}

/// Whether the golden's and our (normalized) values of `property` show the same
/// thing. Beyond equal text, this allows what old WebKit reports differently for the
/// same rendering:
/// - lengths it snapped to whole pixels (e.g. `auto` margins: 422px + 423px where
///   Chromium computes 422.5px twice);
/// - numbers it stored with less precision ([`close_numbers`]);
/// - `z-index: 0` for elements that form a stacking context without one (`auto`), and
///   `auto` for static elements, where z-index does nothing (Chromium reports the
///   declared value);
/// - the declared border width on a side with no border style (the spec computes 0),
///   where our box has no border image.
///
/// It also allows what our compatibility layer changes without changing the box:
/// `top`, `bottom` and the vertical margins of absolutely positioned boxes centred
/// between their insets by `auto` margins (`host/compat.js` rewrites those insets).
/// The golden can't tell those apart from other absolutely positioned boxes with equal
/// vertical margins, so those properties are left out for all of them; their boxes are
/// still compared, so a box out of place still shows. And where our box has a border
/// image, its border style is `solid` for the golden's `none`, and its colour
/// transparent: old WebKit keeps the border widths under a border image (the widths
/// must match there), and `host/compat.js` gives such rules a style to keep them,
/// drawn transparent where the image doesn't show (old WebKit draws nothing there).
fn same_style(
	golden: &Node,
	ours: &Node,
	property: &str,
	theirs: &str,
	mine: &str,
	page_zoom: f64,
) -> bool {
	let golden_style = |key: &str| {
		golden
			.style
			.iter()
			.find(|(k, _)| k == key)
			.map(|(_, v)| v.as_str())
	};
	let border_image = ours
		.style
		.iter()
		.any(|(k, v)| k == "border-image-source" && v != "none");
	if theirs == mine {
		return true;
	}
	if let (Some(a), Some(b)) = (pixels(theirs), pixels(mine))
		&& (a - b).abs() <= 1.0
	{
		return true;
	}
	if close_numbers(theirs, mine) {
		return true;
	}
	match property {
		"z-index" => {
			matches!((theirs, mine), ("0", "auto") | ("auto", "0"))
				|| (theirs == "auto" && golden_style("position") == Some("static"))
		}
		p if p.starts_with("border-") && p.ends_with("-width") => {
			!border_image && matches!(golden_style("border-top-style"), Some("none" | "hidden"))
		}
		"border-top-style" => border_image && theirs == "none" && mine == "solid",
		"border-top-color" => border_image && mine == "rgba(0, 0, 0, 0)",
		"top" | "bottom" | "margin-top" | "margin-bottom" => {
			// Equal margins, or a half pixel apart before old WebKit snapped them (-17.5px
			// twice reported as -17px and -18px).
			let margin = |key: &str| golden_style(key).and_then(pixels);
			golden_style("position") == Some("absolute")
				&& matches!((margin("margin-top"), margin("margin-bottom")), (Some(t), Some(b)) if (t - b).abs() <= 1.0)
		}
		// Seen on the battle screen: for a box centred by `margin: auto` (the turn bar's
		// 100rem-wide middle, `position: relative`), old WebKit reports the used margins
		// (780px either side) and Chromium 0px, with the box in the same place.
		"margin-left" | "margin-right" => {
			let margin = |key: &str| golden_style(key).and_then(pixels);
			mine == "0px"
				&& matches!((margin("margin-left"), margin("margin-right")), (Some(l), Some(r)) if (l - r).abs() <= 1.0)
		}
		// Under a page zoom, old WebKit reports a shadow's lengths divided by it (a 1px
		// shadow at UI scale 1.5 as 0.667px).
		"text-shadow" => close_numbers(&zoomed_pixels(theirs, page_zoom), mine),
		// A transform's translation is in pixels: it follows the box's size, which may be
		// a pixel off (`translateY(-50%)` of an 18px or a 19px label).
		"transform" => {
			let ((theirs_text, theirs_numbers), (mine_text, mine_numbers)) =
				(numbers(theirs), numbers(mine));
			theirs_text == mine_text
				&& theirs_text.starts_with("matrix(")
				&& theirs_numbers.len() == 6
				&& mine_numbers.len() == 6
				&& theirs_numbers
					.iter()
					.zip(&mine_numbers)
					.enumerate()
					.all(|(i, (a, b))| {
						(a - b).abs() <= if i < 4 { 1.0 / 255.0 + 1e-9 } else { 1.0 }
					})
		}
		_ => false,
	}
}

/// How an element is named in a path: its tag and classes.
fn label(node: &Node) -> String {
	let classes: Vec<&str> = node.cls.split_whitespace().collect();
	if classes.is_empty() {
		node.tag.clone()
	} else {
		format!("{}.{}", node.tag, classes.join("."))
	}
}

/// Whether two elements are the same element of the page (for alignment).
fn same_kind(a: &Node, b: &Node) -> bool {
	a.tag == b.tag && a.cls.split_whitespace().eq(b.cls.split_whitespace())
}

/// List how `ours` differs from `golden`, within `tolerance`.
pub fn compare(golden: &Snapshot, ours: &Snapshot, tolerance: &Tolerance) -> Vec<Difference> {
	let mut differences = Vec::new();
	compare_nodes(
		&golden.root,
		&ours.root,
		&label(&golden.root),
		tolerance,
		&mut differences,
	);
	differences
}

fn compare_nodes(
	golden: &Node,
	ours: &Node,
	path: &str,
	tolerance: &Tolerance,
	out: &mut Vec<Difference>,
) {
	if tolerance
		.ignore_paths
		.iter()
		.any(|p| path.contains(p.as_str()))
	{
		return;
	}
	if golden.text != ours.text {
		out.push(Difference::Text {
			path: path.to_string(),
			golden: golden.text.clone(),
			ours: ours.text.clone(),
		});
	}
	// Either side's mark counts: goldens taken before snapshots marked these lack it.
	if (golden.procedural || ours.procedural) && !tolerance.procedural_images {
		return;
	}
	// Old WebKit reports an empty box for `<br>`; Chromium, its place in the line.
	if golden.tag != "br"
		&& !tolerance
			.ignore_boxes
			.iter()
			.any(|p| path.contains(p.as_str()))
		&& golden
			.rect
			.iter()
			.zip(ours.rect.iter())
			.any(|(g, o)| (g - o).abs() > tolerance.pixels)
	{
		out.push(Difference::Rect {
			path: path.to_string(),
			golden: golden.rect,
			ours: ours.rect,
		});
	}
	for (property, value) in &golden.style {
		if tolerance.ignore_styles.contains(property) {
			continue;
		}
		let theirs = normalized(&golden.tag, property, value);
		let mine = ours
			.style
			.iter()
			.find(|(p, _)| p == property)
			.map(|(_, v)| normalized(&ours.tag, property, v))
			.unwrap_or_default();
		if !same_style(golden, ours, property, &theirs, &mine, tolerance.page_zoom) {
			out.push(Difference::Style {
				path: path.to_string(),
				property: property.clone(),
				golden: theirs,
				ours: mine,
			});
		}
	}
	let (g, o) = (&golden.children, &ours.children);
	let ignored = |path: &str| tolerance.ignore_paths.iter().any(|p| path.contains(p.as_str()));
	for step in align(g, o) {
		match step {
			Step::Both(i, j) => {
				let child = format!("{path} > {}[{i}]", label(&g[i]));
				compare_nodes(&g[i], &o[j], &child, tolerance, out);
			}
			Step::Golden(i) => {
				let path = format!("{path} > {}[{i}]", label(&g[i]));
				if !ignored(&path) {
					out.push(Difference::Missing { path });
				}
			}
			Step::Ours(j) => {
				let path = format!("{path} > {}", label(&o[j]));
				if !ignored(&path) {
					out.push(Difference::Extra { path });
				}
			}
		}
	}
}

/// One step of an alignment of two child lists.
enum Step {
	Both(usize, usize),
	Golden(usize),
	Ours(usize),
}

/// A longest-common-subsequence alignment of `g` and `o` by [`same_kind`].
fn align(g: &[Node], o: &[Node]) -> Vec<Step> {
	let (n, m) = (g.len(), o.len());
	// lcs[i][j]: the LCS length of g[i..] and o[j..].
	let mut lcs = vec![vec![0usize; m + 1]; n + 1];
	for i in (0..n).rev() {
		for j in (0..m).rev() {
			lcs[i][j] = if same_kind(&g[i], &o[j]) {
				lcs[i + 1][j + 1] + 1
			} else {
				lcs[i + 1][j].max(lcs[i][j + 1])
			};
		}
	}
	let (mut i, mut j, mut steps) = (0, 0, Vec::new());
	while i < n && j < m {
		if same_kind(&g[i], &o[j]) {
			steps.push(Step::Both(i, j));
			i += 1;
			j += 1;
		} else if lcs[i + 1][j] >= lcs[i][j + 1] {
			steps.push(Step::Golden(i));
			i += 1;
		} else {
			steps.push(Step::Ours(j));
			j += 1;
		}
	}
	steps.extend((i..n).map(Step::Golden));
	steps.extend((j..m).map(Step::Ours));
	steps
}

#[cfg(test)]
mod tests {
	use super::{
		Difference, Snapshot, Tolerance, close_numbers, compare, normalize_style, zoomed_pixels,
	};
	use crate::json::Json;

	fn snapshot(text: &str) -> Snapshot {
		Snapshot::parse(&Json::parse(text).unwrap()).unwrap()
	}

	const GOLDEN: &str = r#"{"viewport":[100,100],"root":{"tag":"body","cls":"","rect":[0,0,100,100],
		"style":{"color":"rgb(1, 2, 3)","font-family":"'Times New Roman'"},
		"children":[
			{"tag":"div","cls":"a","rect":[0,0,10,10],"style":{},"children":[]},
			{"tag":"div","cls":"b","text":"Hi","rect":[0,10,10,10],"style":{},"children":[]},
			{"tag":"div","cls":"c","rect":[0,20,10,10],"style":{},"children":[]}
		]}}"#;

	#[test]
	fn identical_snapshots_have_no_differences() {
		let g = snapshot(GOLDEN);
		assert!(compare(&g, &g, &Tolerance::default()).is_empty());
	}

	#[test]
	fn engine_spellings_of_the_same_value_match() {
		assert_eq!(normalize_style("rgb(1, 2, 3)"), "rgba(1, 2, 3, 1)");
		assert_eq!(
			normalize_style("\"Times New Roman\""),
			normalize_style("'Times New Roman'")
		);
		assert_eq!(
			normalize_style("rgb(0, 0, 0) 1px 1px 2px"),
			"rgba(0, 0, 0, 1) 1px 1px 2px"
		);
	}

	#[test]
	fn shadows_scale_with_the_page_zoom() {
		let golden =
			"rgba(0, 0, 0, 1) 0.6666666865348816px 0.6666666865348816px 0.6666666865348816px";
		assert!(close_numbers(
			&zoomed_pixels(golden, 1.5),
			"rgba(0, 0, 0, 1) 1px 1px 1px"
		));
		assert!(!close_numbers(
			&zoomed_pixels(golden, 1.0),
			"rgba(0, 0, 0, 1) 1px 1px 1px"
		));
	}

	#[test]
	fn numbers_within_an_8_bit_step_match() {
		assert!(close_numbers(
			"rgba(0, 0, 0, 0.298039)",
			"rgba(0, 0, 0, 0.3)"
		));
		assert!(close_numbers("0.4000000059604645", "0.4"));
		assert!(close_numbers("-0.5px 1px", "-0.5px 1px"));
		assert!(!close_numbers("rgba(0, 0, 0, 0.29)", "rgba(0, 0, 0, 0.3)"));
		assert!(!close_numbers("rgba(1, 0, 0, 0.3)", "rgba(0, 0, 0, 0.3)"));
		assert!(!close_numbers("0.4 1", "0.4"));
		assert!(!close_numbers("solid", "dashed"));
	}

	#[test]
	fn a_missing_node_does_not_misalign_its_siblings() {
		let g = snapshot(GOLDEN);
		let ours = snapshot(&GOLDEN.replace(
			r#"{"tag":"div","cls":"a","rect":[0,0,10,10],"style":{},"children":[]},"#,
			"",
		));
		let differences = compare(&g, &ours, &Tolerance::default());
		assert_eq!(
			differences,
			[Difference::Missing {
				path: "body > div.a[0]".into()
			}]
		);
	}

	#[test]
	fn reports_text_boxes_and_styles_beyond_the_tolerance() {
		let g = snapshot(GOLDEN);
		let ours = snapshot(
			&GOLDEN
				.replace(r#""text":"Hi""#, r#""text":"Ho""#)
				.replace("[0,20,10,10]", "[0,25,10,10]")
				.replace("[0,0,10,10]", "[1,0,10,10]")
				.replace("rgb(1, 2, 3)", "rgba(1, 2, 4, 1)"),
		);
		let shown: Vec<String> = compare(&g, &ours, &Tolerance::default())
			.iter()
			.map(|d| d.to_string())
			.collect();
		assert_eq!(
			shown,
			[
				r#"style: body: color: "rgba(1, 2, 3, 1)" -> "rgba(1, 2, 4, 1)""#,
				r#"text: body > div.b[1]: Some("Hi") -> Some("Ho")"#,
				r#"box: body > div.c[2]: [0, 20, 10, 10] -> [0, 25, 10, 10]"#,
			]
		);
	}
}
