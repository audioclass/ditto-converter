# Ditto Converter

Read README.md and docs/BUILD.md before changing build contracts.

- Dedicated native Tauri desktop utility; no user-run server or browser UI.
- Preserve original media; validate output bytes before safe non-clobbering export.
- Local conversion must not depend on optional online components or network availability.
- Playlist metadata enters an ordered removable review queue before acquisition.
- Never use shell execution for filenames, URLs or metadata; no automatic browser cookies.
- Original code is MIT; retain independently licensed component notices and source rights.
- Build native per-platform helpers; never stage Linux binaries on Windows/macOS.
- Keep smoke/test-control instrumentation out of production packages.
- Compilation, unit tests, UI smoke, clean installation, audible playback and pedal hardware acceptance are separate categories. Do not imply a native or hardware pass from cross-checks.
- Keep generated state within the checkout; do not add secrets, private logs or local machine details.
- No production-service, host-virtualization or remote-desktop changes are part of development.
