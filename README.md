# Ditto Converter

A focused Rust/Tauri desktop utility for preparing local audio and authorized online tracks for a Ditto+ looper workflow.

## Development status

This is experimental software, not a signed consumer release. Native Windows x64 and Apple Silicon macOS builds are being validated by the repository's **Native Windows and Apple Silicon** GitHub Actions workflow. Check the exact commit's run rather than assuming a passing badge or historical report establishes support.

**Source checkpoint — October 21, 2026 EDT:** the combined private Apple Silicon production build, reviewed dependency/notice remediation, source pairing, automated tests, 14 native smoke checks and final consumer cancellation/trim/retry follow-up passed for a retained source export. Later documentation/workflow-label edits are not that exact tested export. Fresh Windows regression of the combined source is next, followed by owner hands-on UX/UI testing and fixes. This checkpoint is not a release and does not enable Mac binary uploads. See [acceptance boundaries](docs/ACCEPTANCE.md).

The application implements a shared review queue, local conversion, nondestructive trim and preview, collision-safe exports, and optional app-managed online tools. Output is classic mono 44.1 kHz 24-bit PCM WAV. **No generated output has been tested on a physical Ditto+ pedal.** Native compilation, automated tests, installation, audible playback and hardware compatibility are distinct acceptance categories.

## Build and test

See [build instructions](docs/BUILD.md), [third-party software](docs/THIRD_PARTY.md), [acceptance boundaries](docs/ACCEPTANCE.md), and [source-publication scope](docs/SANITIZATION.md).

- `src-tauri/`: native shell, settings, queue and IPC.
- `crates/ditto-core/`: audio validation, safe export, process ownership and recovery.
- `crates/ditto-online/`: optional component verification, metadata and acquisition.
- `ui/`: bundled TypeScript frontend; no external fonts, artwork or cloud backend.
- `scripts/ci/`: native helper construction and package/evidence checks.

Packages do not require users to install Python, run a server or edit PATH. Development commands are for contributors, not an end-user installation procedure.

## License and limitations

Original project code is [MIT licensed](LICENSE). Dependencies and independent FFmpeg/ffprobe/online helper programs retain their own licenses and source/notice obligations; see [third-party details](docs/THIRD_PARTY.md).

CI packages are **unsigned test builds**. They are not notarized macOS releases or trusted Windows signed installers. Do not disable OS security checks as an installation workaround. Online provider availability and permission are separate concerns: use only content and access methods you are authorized to use. Cookie/browser-session import is not implemented.

Not affiliated with or endorsed by TC Electronic.
