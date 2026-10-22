# Acceptance boundaries

Passing hosted CI means only the checks actually recorded for the exact commit and platform passed. No test should be silently skipped or mocked while being described as real native verification.

## Automated evidence

## Retained checkpoint evidence — October 21, 2026 EDT

The combined private Apple Silicon production app/DMG build executed 86 Rust, 70 Node and 9 UI tests without failures/skips, plus 5 reconstruction tests and 14 isolated native smoke checks. The native notice collector recorded 273 inputs and zero failures; all five required checksum-pinned MPL sources were paired and actual packaged notices checked. The reviewed CoreFoundation variant compiled; production excluded smoke instrumentation. These results apply to the retained source export, not automatically to later commits or workflow/documentation edits.

Final ordinary consumer follow-up exercised the production picker, held-helper conversion/cancellation and one-second trim/retry, with independent PCM validation and source preservation. The final consumer sample is silent PCM; this is format/trim evidence, not audible fidelity. Local ad-hoc bundle verification is not trusted Developer ID signing, notarization or downloaded/quarantined launch acceptance.

Historical failure evidence remains unchanged: the paired private deliverables retain an initial failed independent-decode diagnostic; the separately retained final smoke report records all 14 checks and successful decode. Checksum integrity alone does not mean every included diagnostic is the final result. Earlier Windows consumer installation/layout/uninstall checks apply to `5014bd5`, not the combined remediation. **Fresh combined-source Windows regression is still required**, then owner UX/UI testing and fixes. No release or Mac binary upload is authorized by this source checkpoint.

## Categories exercised by automated checks

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
