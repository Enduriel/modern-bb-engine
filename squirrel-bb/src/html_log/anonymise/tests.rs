use super::*;

fn windows(home: &str) -> Anonymiser {
	let mut a = Anonymiser::new(Style::Windows);
	assert!(a.hide_bytes(home.as_bytes(), "%USERPROFILE%"));
	a
}

fn unix(home: &str) -> Anonymiser {
	let mut a = Anonymiser::new(Style::Unix);
	assert!(a.hide_bytes(home.as_bytes(), "~"));
	a
}

fn apply(a: &Anonymiser, text: &str) -> String {
	String::from_utf8(a.apply(text.as_bytes()).into_owned()).unwrap()
}

#[test]
fn a_windows_home_with_either_separator() {
	let a = windows(r"C:\Users\Kacper Nowak");
	assert_eq!(
		apply(&a, r"saves: C:\Users\Kacper Nowak\Documents\Battle Brothers\savegames"),
		r"saves: %USERPROFILE%\Documents\Battle Brothers\savegames"
	);
	assert_eq!(apply(&a, "C:/Users/Kacper Nowak/AppData/Roaming/x"), "%USERPROFILE%/AppData/Roaming/x");
	assert_eq!(apply(&a, r"C:\Users/Kacper Nowak\x"), r"%USERPROFILE%\x");
	// Repeated separators, as an escaped string shows them.
	assert_eq!(apply(&a, r"C:\\Users\\Kacper Nowak\\x"), r"%USERPROFILE%\\x");
	// A verbatim path (as `canonicalize` gives on Windows).
	assert_eq!(apply(&a, r"\\?\C:\Users\Kacper Nowak\x"), r"\\?\%USERPROFILE%\x");
}

#[test]
fn case_is_ignored_on_windows_only() {
	let a = windows(r"C:\Users\Kacper");
	assert_eq!(apply(&a, r"c:\users\KACPER\Documents"), r"%USERPROFILE%\Documents");
	let u = unix("/home/kacper");
	assert_eq!(apply(&u, "/home/Kacper/x"), "/home/Kacper/x");
	assert_eq!(apply(&u, "/home/kacper/x"), "~/x");
}

#[test]
fn a_unix_home() {
	let a = unix("/home/kacper/");
	assert_eq!(
		apply(&a, "the game's data: /home/kacper/.local/share/Steam/steamapps/common/Battle Brothers/data (from Steam)"),
		"the game's data: ~/.local/share/Steam/steamapps/common/Battle Brothers/data (from Steam)"
	);
	let mac = unix("/Users/kacper");
	assert_eq!(apply(&mac, "/Users/kacper/Library/Application Support/battle-brothers-modern"), "~/Library/Application Support/battle-brothers-modern");
	// Backslashes don't separate on Unix.
	assert_eq!(apply(&a, r"/home\kacper/x"), r"/home\kacper/x");
}

#[test]
fn only_the_whole_folder() {
	let a = windows(r"C:\Users\Al");
	assert_eq!(apply(&a, r"C:\Users\Alice\Documents"), r"C:\Users\Alice\Documents");
	assert_eq!(apply(&a, r"C:\Users\Al.old\x"), r"C:\Users\Al.old\x");
	assert_eq!(apply(&a, r"D:\Users\Al\x"), r"D:\Users\Al\x");
	// The folder itself, wherever its name ends.
	assert_eq!(apply(&a, r"home: C:\Users\Al"), "home: %USERPROFILE%");
	assert_eq!(apply(&a, r"in C:\Users\Al. Next"), "in %USERPROFILE%. Next");
	assert_eq!(apply(&a, r#"path "C:\Users\Al", (C:\Users\Al)"#), r#"path "%USERPROFILE%", (%USERPROFILE%)"#);
	assert_eq!(apply(&a, r#"<div class="text">C:\Users\Al</div>"#), r#"<div class="text">%USERPROFILE%</div>"#);
	let u = unix("/home/al");
	assert_eq!(apply(&u, "/home/alice/x /home/al /home/al/x"), "/home/alice/x ~ ~/x");
}

#[test]
fn documents_outside_the_home_folder() {
	let mut a = windows(r"C:\Users\Kacper");
	assert!(a.hide_bytes(br"D:\Kacper\Documents", "%DOCUMENTS%"));
	assert_eq!(
		apply(&a, r"saves: D:\Kacper\Documents\Battle Brothers\savegames; settings: C:\Users\Kacper\AppData\Roaming"),
		r"saves: %DOCUMENTS%\Battle Brothers\savegames; settings: %USERPROFILE%\AppData\Roaming"
	);
	// Documents in the home folder (OneDrive's backup included) is the home's.
	assert!(!a.hide_bytes(br"C:\Users\Kacper\OneDrive\Documents", "%DOCUMENTS%"));
	assert_eq!(apply(&a, r"C:\Users\Kacper\OneDrive\Documents\x"), r"%USERPROFILE%\OneDrive\Documents\x");
}

#[test]
fn a_folder_inside_another_is_matched_first() {
	// Hidden in either order, the longer one is tried first.
	let mut a = Anonymiser::new(Style::Windows);
	assert!(a.hide_bytes(br"E:\Users\Al\Docs", "%DOCUMENTS%"));
	assert!(a.hide_bytes(br"E:\Users\Al", "%USERPROFILE%"));
	assert_eq!(apply(&a, r"E:\Users\Al\Docs\x E:\Users\Al\y"), r"%DOCUMENTS%\x %USERPROFILE%\y");
}

#[test]
fn percent_encoded_urls() {
	let a = windows(r"C:\Users\Jan Kowalski");
	assert_eq!(apply(&a, "file:///C:/Users/Jan%20Kowalski/Documents/x.html"), "file:///%USERPROFILE%/Documents/x.html");
	assert_eq!(apply(&a, "file:///C%3A%2FUsers%5Cjan%20kowalski%2Fx"), "file:///%USERPROFILE%%2Fx");
	let u = unix("/home/jan kowalski");
	assert_eq!(apply(&u, "file:///home/jan%20kowalski/x"), "file://~/x");
	// An encoded name that goes on is another folder.
	assert_eq!(apply(&u, "file:///home/jan%20kowalski2/x"), "file:///home/jan%20kowalski2/x");
}

#[test]
fn applying_twice_changes_nothing_more() {
	let mut a = windows(r"C:\Users\Kacper");
	a.hide_bytes(br"D:\Kacper\Documents", "%DOCUMENTS%");
	let once = apply(&a, r"C:\Users\Kacper\x D:\Kacper\Documents\y");
	assert_eq!(apply(&a, &once), once);
	let u = unix("/home/k");
	let once = apply(&u, "/home/k/x ~/y");
	assert_eq!(once, "~/x ~/y");
	assert_eq!(apply(&u, &once), once);
	// Text without a hidden folder is not copied.
	assert!(matches!(a.apply(b"nothing here"), Cow::Borrowed(_)));
}

#[test]
fn no_folder_of_its_own_is_never_hidden() {
	for home in ["", "/", "//", "relative/home", "~"] {
		let mut a = Anonymiser::new(Style::Unix);
		assert!(!a.hide_bytes(home.as_bytes(), "~"), "{home:?}");
		assert_eq!(apply(&a, "/usr/lib /home/x"), "/usr/lib /home/x");
	}
	for home in ["", r"C:", r"C:\", "C:/", r"\", r"\\", r"\\server", r"\\server\", r"\\?\C:\", r"\\.\pipe\x", r"Users\x", "%USERPROFILE%"] {
		let mut a = Anonymiser::new(Style::Windows);
		assert!(!a.hide_bytes(home.as_bytes(), "%USERPROFILE%"), "{home:?}");
		assert_eq!(apply(&a, r"C:\Windows \\server\share\x"), r"C:\Windows \\server\share\x");
	}
	// A home on a share is a folder of its own.
	let a = windows(r"\\server\homes\al");
	assert_eq!(apply(&a, r"\\server\homes\al\Documents"), r"%USERPROFILE%\Documents");
	assert_eq!(apply(&a, r"\\server\homes\alice"), r"\\server\homes\alice");
}

#[test]
fn this_users_home_is_hidden() {
	let home = std::env::home_dir().expect("a home folder");
	let a = Anonymiser::for_this_user();
	let text = format!("{}", home.join("x").display());
	let placeholder = Style::HOST.home_placeholder();
	assert_eq!(String::from_utf8(a.apply(text.as_bytes()).into_owned()).unwrap(), format!("{placeholder}{}x", std::path::MAIN_SEPARATOR));
}
