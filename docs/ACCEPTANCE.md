# Acceptance boundaries

Passing hosted CI means only the checks actually recorded for the exact commit and platform passed. No test should be silently skipped or mocked while being described as real native verification.

## Automated evidence

- Native runner/compiler architecture; Rust fmt/clippy/locked tests; frontend type-check/build and UI logic tests.
- Generated codec corpus through the actual scoped pair, decoded output/header/sample checks, trim, preservation/collisions/cancellation, and staged-helper app integration.
- Native package construction, binary architecture, helper hash/config/import checks, available third-party notices/source, and package checksums.
- Test-only native UI smoke, when recorded, is separate from uninstrumented production packages. Browser mocks are not native IPC/audio evidence.

## Not established by compilation or package extraction

- Clean standard-user installation, uninstall residue, WebView2-absent setup and read-only paths.
- Normal double-click launch, console visibility, file picker/drag/drop, keyboard/high-DPI/accessibility and quit/cancel races.
- Physical sound output or real audio-device behavior.
- Signed Windows trust, macOS Developer ID/notarization, quarantine/Gatekeeper and offline launch.
- Live authorized provider media acquisition/progress/online confirmation, or authenticated independent component updates.
- Full-disk/removable-storage/power-loss durability, older OS compatibility and long-path edge cases.
- Playback of generated output on a physical Ditto+ pedal (not tested).

Unsigned CI packages are for controlled testing, not ordinary-user signed distribution. Future VM acceptance uses isolated guests and known baselines; this repository does not install/configure a host hypervisor or grant host administration.
