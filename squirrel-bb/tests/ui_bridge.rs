//! The headless UI bridge: the SQ UI scripts run with no JS, SQ→JS calls are
//! recorded, and the harness simulates UI input by invoking the connected SQ
//! receiver directly (as the JS dispatcher would).

use squirrel::{SQVM, SqTable};
use squirrel_bb::{UiBridge, install_natives};

#[test]
fn records_sq_to_js_calls_and_simulates_input() {
	let mut vm = SQVM::new(1 << 16);
	install_natives(&mut vm);
	let bridge = UiBridge::new();
	bridge.install(&mut vm);

	// A representative screen + module, connected and exercised exactly as a real
	// UI script does (UI.connect, asyncCall/call, connectToModule).
	vm.run_script(
		r#"
		::okPressed <- false;
		::queriedArg <- null;

		local screen = {
			handle = null,
			moduleHandle = null,
			function onOkPressed() { ::okPressed = true; }
			function onQueryValue( _n ) { ::queriedArg = _n; return _n * 2; }
		};
		local mod = { function onModuleThing() {} };

		screen.handle = this.UI.connect("MyScreen", screen);
		screen.handle.asyncCall("show", { Title = "Hello" });   // async, screen-level
		screen.handle.call("setData", [1, 2, null, 4]);         // sync, screen-level
		screen.moduleHandle = screen.handle.connectToModule("MyModule", mod);
		screen.moduleHandle.asyncCall("update", 42);            // async, module-level

		::held <- screen;   // keep alive past this script
		"#,
		"ui_bridge_drive",
	)
	.expect("UI scripts run against the headless bridge");

	// --- OUTPUT: assert the recorded SQ→JS call log ---
	let calls = bridge.calls();
	assert_eq!(calls.len(), 3, "show + setData + update");

	assert_eq!(calls[0].method, "show");
	assert_eq!(calls[0].screen, "MyScreen");
	assert_eq!(calls[0].module, None);
	assert!(!calls[0].sync);

	assert_eq!(calls[1].method, "setData");
	assert!(calls[1].sync, "JSHandle.call is the synchronous channel");

	assert_eq!(calls[2].method, "update");
	assert_eq!(
		calls[2].module.as_deref(),
		Some("MyModule"),
		"module attribution"
	);
	assert!(!calls[2].sync);

	// ordering helper + data inspection of the captured payload
	assert_eq!(
		bridge.methods_for("MyScreen"),
		["show", "setData", "update"]
	);
	let show = bridge.calls_to("MyScreen", "show");
	assert_eq!(show.len(), 1);
	let title = show[0].as_table(&vm).and_then(|t| t.get::<String>("Title"));
	assert_eq!(
		title.as_deref(),
		Some("Hello"),
		"the asyncCall data table is captured"
	);

	// --- INPUT: simulate UI actions by firing the connected receiver directly ---
	let screen = bridge
		.receiver("MyScreen", None)
		.expect("screen is connected");
	assert!(
		bridge.receiver("MyScreen", Some("MyModule")).is_some(),
		"module is connected"
	);

	// "click OK" — fire-and-forget; assert the SQ side reacted.
	screen
		.call_method(&vm, "onOkPressed", ())
		.expect("fire onOkPressed");
	assert_eq!(SqTable::root(&vm).get::<bool>("okPressed"), Some(true));

	// the synchronous query path — assert the SQ method's return value.
	let doubled = screen
		.call_method_ret::<i64, _>(&vm, "onQueryValue", (21i64,))
		.expect("fire onQueryValue");
	assert_eq!(doubled, Some(42));
	assert_eq!(SqTable::root(&vm).get::<i64>("queriedArg"), Some(21));

	// Release retained refs before the VM closes (drop-before-close contract).
	drop(screen);
	bridge.clear();
}

#[test]
fn disconnect_forgets_the_connection_and_returns_null() {
	let mut vm = SQVM::new(1 << 16);
	install_natives(&mut vm);
	let bridge = UiBridge::new();
	bridge.install(&mut vm);

	vm.run_script(
		r#"
		local screen = { function onX() {} };
		local h = this.UI.connect("Gone", screen);
		::wasNull <- (this.UI.disconnect(h) == null);   // disconnect returns null
		"#,
		"ui_bridge_disconnect",
	)
	.expect("connect + disconnect run");

	assert_eq!(SqTable::root(&vm).get::<bool>("wasNull"), Some(true));
	assert!(
		bridge.receiver("Gone", None).is_none(),
		"disconnect forgot the connection"
	);

	bridge.clear();
}
