# download-cef

Utility functions to download and extract prebuilt [Chromium Embedded Framework](https://github.com/chromiumembedded/cef)
archives on any supported platform.

## This copy (Battle Brothers modern engine)

A patched copy of `download-cef` **3.0.0** from crates.io (upstream
<https://github.com/tauri-apps/cef-rs>, commit `207bdd2917c038283635022ffcf7767496bcc7e6`,
path `download-cef`), licensed Apache-2.0 OR MIT (`LICENSE-APACHE`, `LICENSE-MIT`, taken
from that commit). The workspace uses it through `[patch.crates-io]` in its `Cargo.toml`;
cef-dll-sys's build script fetches CEF with it.

Why: upstream's `ureq` is built with its default `rustls` feature, which trusts a bundled
copy of Mozilla's CA list (`webpki-roots`). This copy trusts the system's own roots
instead, and has no bundled roots to fall back to.

What changed, and nothing else:

- `Cargo.toml` (cargo's normalized manifest from the published crate; `Cargo.toml.orig`
  and the published `Cargo.lock` dropped): `ureq` with `default-features = false` and
  `gzip`, `json`, `socks-proxy`, `rustls-no-provider`; `rustls` on `ring` (`ring`, `std`,
  `tls12`); `rustls-native-certs` 0.8.5.
- `src/lib.rs`: one `ureq::Agent` (`agent()`, built once) for both requests (the index
  and the archive) instead of `ureq::get`. Its TLS is rustls on `ring`, its roots
  (`RootCerts::Specific`) the system's, loaded by `rustls-native-certs`: Windows'
  certificate store, macOS' keychain, the Linux distribution's bundle, or
  `SSL_CERT_FILE`/`SSL_CERT_DIR` when set. None loaded fails with the new
  `Error::NoSystemRoots`, naming why. The agent is otherwise upstream's default.

The archive is still checked against the SHA-1 in CEF's index, as upstream does.

Note: cargo sets `SSL_CERT_FILE`/`SSL_CERT_DIR` for the build scripts it runs (on Linux,
to the system's bundle it found), so under cargo those name the roots used.
