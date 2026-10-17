# Native build and verification

The canonical reproducible procedure is `.github/workflows/native.yml`. It uses explicit standard GitHub-hosted `windows-2025` (x64) and `macos-15` (Apple Silicon/arm64) labels and asserts the actual runner and Rust host architectures. These are OS-family labels, not frozen VM images; exact image/compiler versions appear in artifacts.

## Prerequisites

Contributors need Node 24.21.0/npm, the Rust version in `rust-toolchain.toml` (with fmt/clippy), and Tauri's platform build prerequisites. Windows requires MSVC plus MSYS2 UCRT64 for the independently compiled helper pair. macOS requires Xcode/command-line tools and native Clang/make. The workflow provisions a full-feature FFmpeg through MSYS2/Homebrew **only for test-fixture generation**. Native toolchain installation on a personal machine is not automated by this repository.

## Workflow sequence

1. `npm ci --no-audit --no-fund` (lockfile-respecting).
2. Compile the scoped offline pair: `bash scripts/ci/build-ffmpeg.sh`. This verifies official FFmpeg 8.1.3 source SHA-256 and builds for the current native target, with no network/GPL/nonfree/version3/external-codec features.
3. The CI driver stages only exact native helpers and notices; it audits architecture, configuration, imports, runtime execution and hashes. It requires GitHub's environment-file interface for contained environment propagation; follow the workflow for complete invocation order.
4. Generate platform icons from the original project icon. Run workspace fmt/clippy, locked native Rust tests, Node UI tests, production frontend build and packaging tests.
5. Build production desktop features without the `smoke` feature. NSIS is the Windows target; Apple Silicon uses .app/DMG. The existing Linux development configuration is not a Windows/macOS helper substitute.
6. Extract/inspect packages; pair native packages with exact FFmpeg source archive, build configuration, scripts, notices, dependency inventories, runner evidence and `checksums.json`.

## Media-test contract

`DITTO_FIXTURE_FFMPEG` points to the full-feature **test-only generator/independent decoder**. `DITTO_FFMPEG` and `DITTO_FFPROBE` point to the scoped pair under test; `DITTO_TEST_*` overrides take precedence. Never use the narrow shipping pair to generate the lossy corpus, and never ship the fixture generator instead.

`npm test`, `node --test scripts/verify-bundle.test.mjs scripts/ci/native.test.mjs`, and `cargo test --workspace --features desktop --locked` are separate checks. Unix shell/Python and ownership-specific cases are platform-gated; real audio corpus tests remain enabled on Windows/macOS. All-platform native Rust child fixtures cover process descendants, cancellation and deadlines without shell prerequisites.

## Resources, billing and removal

CI uses read-only repository permissions, SHA-pinned actions, no persisted checkout credentials, no dependency cache, bounded jobs/concurrency, and one-day artifacts. Standard public hosted-runner minutes and storage have different billing rules; an account-owner hard stop must protect against billable overage. Repository package-upload authorization is explicit; pull requests do not publish native packages.

CI-generated Rust/Cargo/npm/temp/build state is beneath `.work/ci`; a one-command local removal is `node -e "require('node:fs').rmSync('.work/ci',{recursive:true,force:true})"`. It does not remove source, node_modules or previously staged helpers. Hosted runner package-manager changes disappear with the disposable runner. Disable the workflow in repository Actions settings to stop remote execution; never delete a repository as automatic cleanup.
