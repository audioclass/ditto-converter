# Exact build-local CoreFoundation reconstruction

Run from the project root before **any** Cargo metadata, tree, test or build:

```sh
node scripts/prepare-remediation.mjs
node scripts/prepare-remediation.mjs --verify
python3 scripts/test-remediation.py
```

Python 3 standard library is the only additional prerequisite. The Node wrapper
selects `python` on Windows and `python3` elsewhere. To use an already retained
archive: `python3 scripts/prepare-remediation.py --archive <exact.crate>`.
No global registry cache is read for reconstruction or changed in place.

`recipe.json` pins the immutable `objc2-core-foundation 0.3.2` crates.io URL and
archive SHA-256 `2a180dd8642fa45cdb7dd721cd4c11b1cadd4929ce112ebd8b9f5803cc79d536`.
The preparation code also pins the recipe itself, rejects archive traversal,
non-regular members and duplicate paths, verifies archive VCS identity and the
original `src/user_notification.rs` hash, and replaces **only** that file with
the byte-exact supplied `user_notification.rs`. All other 75 files are unchanged.

The replacement is plainly marked as an altered Ditto release-build variant.
It removes only the unused `CFUserNotification::check_box_checked`,
`secure_text_field` and `pop_up_selection` conveniences. Generated external API
declarations, ABI types, feature declarations, geometry/range implementations
and original Cargo manifests are retained byte-for-byte. It does not relicense
any other author's code or turn an SDK-wide uncertainty into blanket clearance.

Cargo.toml has an explicit `[patch.crates-io]` override to
`.work/dependency-remediation/objc2-core-foundation-0.3.2`. Cargo.lock intentionally
retains version 0.3.2 and dependencies but no registry source/checksum for this
path identity; the original archive checksum remains enforced by this recipe.
The notice collector permits **only** this exact reviewed path variant for this
package and re-verifies the whole source tree, recipe/archive/replacement pins
and lockfile source identity. No generic local-source exception is allowed.

Output, archive and `provenance.json` remain under `.work/dependency-remediation`.
Existing modified, extra or symlinked files fail closed; preparation does not
silently overwrite a tampered tree. `--verify` performs no network access or
writes. Remove the build-local reconstruction after stopping builds/tests:

```sh
node -e "require('node:fs').rmSync('.work/dependency-remediation',{recursive:true,force:true})"
```

Re-run preparation to restore it. The native CI workflow, desktop npm prebuild
and native smoke scripts prepare explicitly; raw Cargo cannot prepare a missing
path dependency during dependency resolution. Native builds/tests of the new
variant and exact packaged notice verification passed in the retained combined
private Mac build on October 21, 2026 EDT. New builds still require those checks;
fresh combined-source Windows regression and owner release acceptance remain open.

`notices/` contains unchanged authentic notice bytes, not dependency source.
`notices/provenance.json` maps immutable source URLs and SHA-256 values to exact
versions. Four MIT-only packages retain both the recovered historical and later
upstream grants; six already triple-option packages select Zlib without changing
their declared licenses. The Apple BSD notice is specific to identified
WKWebView header prose. Other unresolved objc2-family versions remain unresolved.

Publish this recipe, the two-line replacement, preparation scripts and notices;
**never** publish the full reconstructed crate, registry cache, SDK archive or
dependency rustdoc/vendor bundle. Existing LGPL/MPL corresponding-source pairing
is independent. These engineering changes do not clear an old binary, authorize
an upload or replace the owner's native package/release decision.
