#![cfg(windows)]
//! Native Windows tests; no helpers, elevation, symlink privilege or network needed.
use ditto_core::recover_staging;
mod common;
use std::{
    fs,
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
            SE_FILE_OBJECT,
        },
        GetSecurityDescriptorDacl, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    },
};

fn current_user_sid() -> String {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TokenUser, TOKEN_QUERY,
            TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    unsafe {
        let mut token = ptr::null_mut();
        assert_ne!(
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token),
            0
        );
        let token = OwnedHandle::from_raw_handle(token);
        let mut len = 0;
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut len,
        );
        let mut data = vec![0usize; (len as usize).div_ceil(std::mem::size_of::<usize>())];
        assert_ne!(
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                data.as_mut_ptr().cast(),
                len,
                &mut len
            ),
            0
        );
        let mut sid_text = ptr::null_mut();
        assert_ne!(
            ConvertSidToStringSidW(
                (*(data.as_ptr() as *const TOKEN_USER)).User.Sid,
                &mut sid_text
            ),
            0
        );
        let mut len = 0;
        while *sid_text.add(len) != 0 {
            len += 1;
        }
        let result = String::from_utf16(std::slice::from_raw_parts(sid_text, len)).unwrap();
        LocalFree(sid_text.cast());
        result
    }
}

fn dacl(path: &Path, sddl: &str) {
    let sddl = sddl.replace(";;;OW", &format!(";;;{}", current_user_sid()));
    use std::os::windows::ffi::OsStrExt;
    let text: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let mut sd = ptr::null_mut();
        assert_ne!(
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                1,
                &mut sd,
                ptr::null_mut()
            ),
            0
        );
        let mut acl = ptr::null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        assert_ne!(
            GetSecurityDescriptorDacl(sd, &mut present, &mut acl, &mut defaulted),
            0
        );
        let status = SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl,
            ptr::null_mut(),
        );
        LocalFree(sd);
        assert_eq!(status, 0);
    }
}

struct Fixture {
    root: tempfile::TempDir,
    journal: PathBuf,
    stage: PathBuf,
    marker: PathBuf,
    raw: PathBuf,
    wav: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap();
        let root = PathBuf::from(root.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
        let root = tempfile::tempdir_in(root).unwrap();
        // Establish a deterministic private Windows fixture, independent of the
        // machine's temp-root inheritance. No ACL repair occurs in production.
        dacl(
            root.path(),
            "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)",
        );
        let stage = root.path().join(".ditto-Ab1234");
        fs::create_dir(&stage).unwrap();
        dacl(
            &stage,
            "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)",
        );
        let marker_name = format!(".ditto-owner-{}", "A".repeat(32));
        let marker = stage.join(&marker_name);
        fs::write(&marker, &marker_name).unwrap();
        let raw = stage.join("audio.pcm");
        fs::write(&raw, b"preserve PCM").unwrap();
        let wav = stage.join(format!(".ditto-wav-{}", "B".repeat(16)));
        fs::write(&wav, b"partial WAV").unwrap();
        let journal = root.path().join("journal.json");
        let record = serde_json::json!({"version": 1, "stage": stage.canonicalize().unwrap(), "marker": marker_name, "wav": format!(".ditto-wav-{}", "B".repeat(16))});
        fs::write(&journal, serde_json::to_vec(&record).unwrap()).unwrap();
        // Keep adversarial parent-ACL edits from altering our ability to inspect
        // byte preservation in child files. The owner SID is never changed.
        for entry in fs::read_dir(&stage).unwrap() {
            dacl(
                &entry.unwrap().path(),
                "D:P(A;;FA;;;OW)(A;;FA;;;BA)(A;;FA;;;SY)",
            );
        }
        dacl(&journal, "D:P(A;;FA;;;OW)(A;;FA;;;BA)(A;;FA;;;SY)");
        Self {
            root,
            journal,
            stage,
            marker,
            raw,
            wav,
        }
    }
    fn refused(&self) {
        let before: Vec<_> = [&self.journal, &self.marker, &self.raw, &self.wav]
            .into_iter()
            .map(|path| (path, fs::read(path).unwrap()))
            .collect();
        assert!(recover_staging(&self.journal).is_err());
        for (path, bytes) in before {
            assert_eq!(fs::read(path).unwrap(), bytes, "changed {}", path.display());
        }
    }
}

#[test]
fn tracked_export_to_readable_destination_does_not_apply_recovery_stage_policy() {
    let f = Fixture::new();
    let output = f.root.path().join("public-output");
    fs::create_dir(&output).unwrap();
    dacl(
        &output,
        "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)(A;OICI;GRGX;;;WD)",
    );
    let input = f.root.path().join("source.wav");
    let status = std::process::Command::new(common::fixture_ffmpeg())
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.5",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&input)
        .status()
        .unwrap();
    assert!(status.success());
    let original = fs::read(&input).unwrap();
    let journal = f.root.path().join("export.json");
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let exported = common::engine()
        .convert_tracked(
            &input,
            &output,
            "track",
            0.0,
            None,
            &cancel,
            |_| {},
            &journal,
        )
        .unwrap();
    assert!(
        ditto_core::audio::validate_wav(&exported, &cancel)
            .unwrap()
            .samples
            > 0
    );
    assert_eq!(fs::read(input).unwrap(), original);
    assert!(!journal.exists());
}

#[test]
fn native_recovery_accepts_dos_and_extended_canonical_paths() {
    for extended in [false, true] {
        let f = Fixture::new();
        let journal = if extended {
            f.journal.canonicalize().unwrap()
        } else {
            f.journal.clone()
        };
        assert!(recover_staging(&journal).unwrap());
        assert!(!f.stage.exists());
        assert!(!f.journal.exists());
        assert!(!recover_staging(&journal).unwrap());
    }
}

#[test]
fn native_recovery_refuses_hardlinks_before_removing_anything() {
    for target in ["journal", "marker", "raw", "wav"] {
        let f = Fixture::new();
        let path = match target {
            "journal" => &f.journal,
            "marker" => &f.marker,
            "wav" => &f.wav,
            _ => &f.raw,
        };
        fs::hard_link(path, f.root.path().join("outside-link")).unwrap();
        f.refused();
    }
}

#[test]
fn native_recovery_refuses_untrusted_acl_grants_and_null_dacl() {
    // OW in test SDDL is replaced with the process user SID so each attack
    // retains that user's full access. Everyone grants are rejected regardless
    // of whether a deny ACE would negate them: this is deliberately conservative.
    for acl in [
        "D:P(A;;FA;;;WD)",
        "D:P(A;;FA;;;OW)(A;;FA;;;BA)(A;;FA;;;SY)(A;;GW;;;WD)",
        "D:NO_ACCESS_CONTROL",
    ] {
        for target in ["journal", "stage", "marker", "raw", "wav"] {
            let f = Fixture::new();
            let path = match target {
                "journal" => &f.journal,
                "stage" => &f.stage,
                "marker" => &f.marker,
                "wav" => &f.wav,
                _ => &f.raw,
            };
            dacl(path, acl);
            f.refused();
        }
    }
}

#[test]
fn native_recovery_refuses_insecure_journal_parents_and_future_file_inheritance() {
    for parent in [false, true] {
        let f = Fixture::new();
        if parent {
            dacl(f.root.path(), "D:P(A;;FA;;;OW)(A;;GW;;;WD)");
        } else {
            dacl(&f.stage, "D:P(A;;FA;;;OW)(A;OIIO;GW;;;WD)");
        }
        f.refused();
    }
}

#[test]
fn native_recovery_refuses_readable_private_metadata_but_allows_readable_audio() {
    for target in ["journal", "stage", "marker"] {
        let f = Fixture::new();
        let path = match target {
            "journal" => &f.journal,
            "stage" => &f.stage,
            _ => &f.marker,
        };
        dacl(path, "D:P(A;;FA;;;OW)(A;;GR;;;WD)");
        f.refused();
    }
    let f = Fixture::new();
    dacl(&f.raw, "D:P(A;;FA;;;OW)(A;;GR;;;WD)");
    assert!(recover_staging(&f.journal).unwrap());
    assert!(!f.stage.exists());
}

#[test]
fn native_recovery_refuses_busy_writers_and_readonly_files_before_any_deletion() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    for target in ["journal", "marker", "raw", "wav"] {
        let f = Fixture::new();
        let path = match target {
            "journal" => &f.journal,
            "marker" => &f.marker,
            "wav" => &f.wav,
            _ => &f.raw,
        };
        let writer = fs::OpenOptions::new()
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(path)
            .unwrap();
        f.refused();
        drop(writer);
        assert!(recover_staging(&f.journal).unwrap());
    }
    let f = Fixture::new();
    let original = fs::metadata(&f.raw).unwrap().permissions();
    let mut permissions = original.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&f.raw, permissions).unwrap();
    f.refused();
    fs::set_permissions(&f.raw, original).unwrap();
    assert!(recover_staging(&f.journal).unwrap());
}

#[test]
fn native_recovery_refuses_directories_in_owned_file_slots_without_recursing() {
    for target in ["marker", "raw", "wav"] {
        let f = Fixture::new();
        let path = match target {
            "marker" => &f.marker,
            "raw" => &f.raw,
            _ => &f.wav,
        };
        let moved = f.root.path().join("saved-original");
        fs::rename(path, &moved).unwrap();
        fs::create_dir(path).unwrap();
        let nested = path.join("original.wav");
        fs::write(&nested, b"nested original must survive").unwrap();
        let before: Vec<_> = [&f.journal, &f.marker, &f.raw, &f.wav, &moved, &nested]
            .into_iter()
            .filter(|path| path.is_file())
            .map(|path| (path, fs::read(path).unwrap()))
            .collect();
        assert!(
            recover_staging(&f.journal).is_err(),
            "accepted directory in {target}"
        );
        assert!(path.is_dir());
        for (path, bytes) in before {
            assert_eq!(fs::read(path).unwrap(), bytes, "changed {}", path.display());
        }
    }
}

#[test]
fn native_recovery_refuses_unexpected_contents_and_path_escapes() {
    for scenario in [
        "extra",
        "marker_escape",
        "wav_escape",
        "noncanonical",
        "ads",
    ] {
        let f = Fixture::new();
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(&f.journal).unwrap()).unwrap();
        match scenario {
            "extra" => fs::write(f.stage.join("original.wav"), b"original").unwrap(),
            "marker_escape" => record["marker"] = "../outside".into(),
            "wav_escape" => record["wav"] = "../outside".into(),
            "noncanonical" => {
                record["stage"] = format!(
                    "{}\\..\\{}",
                    f.stage.display(),
                    f.stage.file_name().unwrap().to_str().unwrap()
                )
                .into()
            }
            "ads" => record["stage"] = format!("{}:stream", f.stage.display()).into(),
            _ => unreachable!(),
        }
        fs::write(&f.journal, serde_json::to_vec(&record).unwrap()).unwrap();
        f.refused();
    }
}

#[test]
fn native_recovery_refuses_junctions_without_symlink_privilege() {
    use std::process::Command;
    for target in ["stage", "ancestor"] {
        let mut f = Fixture::new();
        let link = f.root.path().join("junction");
        let destination = if target == "stage" {
            &f.stage
        } else {
            f.root.path()
        };
        // cmd's built-in mklink is used only with test-generated paths, not app input.
        let status = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&link)
            .arg(destination)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "junction creation must work; this test is not skipped"
        );
        if target == "stage" {
            let mut record: serde_json::Value =
                serde_json::from_slice(&fs::read(&f.journal).unwrap()).unwrap();
            // Give the junction an otherwise-valid staging name.
            let stage_link = f.root.path().join(".ditto-Xy1234");
            fs::rename(&link, &stage_link).unwrap();
            record["stage"] = serde_json::json!(stage_link);
            fs::write(&f.journal, serde_json::to_vec(&record).unwrap()).unwrap();
            f.refused();
            fs::remove_dir(stage_link).unwrap();
        } else {
            let real = f.journal.clone();
            f.journal = link.join("journal.json");
            f.refused();
            f.journal = real;
            fs::remove_dir(link).unwrap();
        }
    }
}
