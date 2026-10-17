//! Conservative local-NTFS recovery. There is deliberately no AccessCheck-based
//! "I can write this" ownership shortcut, no ACL repair and no recursive delete.
//!
//! Trust boundary: the process user, SYSTEM, Windows Modules Installer and local
//! Administrators (which can already take ownership) are trusted. Other users, groups and writable output
//! locations are not. TokenOwner=Administrators is accepted only for a process
//! whose own default owner is that SID, to support elevated Windows processes.
//! Impersonation, network/removable volumes, reparse points (including ancestors),
//! unknown ACE kinds and ambiguous/noncanonical paths fail closed. Same-user or
//! administrator attacks, kernel/filter drivers, and previously granted handles
//! whose rights were subsequently revoked are outside the threat model.
//!
//! API contracts (Microsoft Learn, consulted 2026-10-17):
//! https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-createfilew
//! https://learn.microsoft.com/windows/win32/api/aclapi/nf-aclapi-getsecurityinfo
//! https://learn.microsoft.com/windows/win32/secauthz/owner-of-a-new-object
//! https://learn.microsoft.com/windows/win32/api/winsvc/ns-winsvc-service_sid_info
//! https://learn.microsoft.com/windows/win32/api/securitybaseapi/nf-securitybaseapi-getace
//! https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-getfileinformationbyhandle
//! https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle
//! https://learn.microsoft.com/openspecs/windows_protocols/ms-fsa/4aeefef8-92c3-4abc-af7a-a610caf8a165
use super::{random_name, Record, LIMIT};
use crate::{Error, Result};
use std::{
    ffi::c_void,
    fs::{self, File},
    io::Read,
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Component, Path, PathBuf, Prefix},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{
        LocalFree, ERROR_NO_TOKEN, GENERIC_ALL, GENERIC_EXECUTE, GENERIC_READ, GENERIC_WRITE,
        INVALID_HANDLE_VALUE,
    },
    Security::{
        Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
        *,
    },
    Storage::FileSystem::*,
    System::{
        SystemServices::{
            ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE, FILE_PERSISTENT_ACLS, MAXIMUM_ALLOWED,
        },
        Threading::{GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken},
        WindowsProgramming::DRIVE_FIXED,
    },
};

fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn win(ok: i32) -> Result<()> {
    if ok == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

struct Identity {
    user: Vec<u32>,
    admin_owner: bool,
    installer: Option<Vec<u32>>,
}
impl Identity {
    fn current() -> Result<Self> {
        unsafe {
            let mut token = ptr::null_mut();
            if OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) != 0 {
                let _token = OwnedHandle::from_raw_handle(token);
                return Err(invalid("staging recovery does not support impersonation"));
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(ERROR_NO_TOKEN as i32) {
                return Err(std::io::Error::last_os_error().into());
            }
            win(OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY,
                &mut token,
            ))?;
            let token = OwnedHandle::from_raw_handle(token);
            let user = token_info(&token, TokenUser)?;
            let sid = (*(user.as_ptr() as *const TOKEN_USER)).User.Sid;
            if IsValidSid(sid) == 0 {
                return Err(invalid("invalid process user SID"));
            }
            let bytes = GetLengthSid(sid);
            let mut copy = vec![0u32; (bytes as usize).div_ceil(size_of::<u32>())];
            win(CopySid(bytes, copy.as_mut_ptr().cast(), sid))?;
            let owner = token_info(&token, TokenOwner)?;
            let owner = (*(owner.as_ptr() as *const TOKEN_OWNER)).Owner;
            Ok(Self {
                user: copy,
                admin_owner: IsWellKnownSid(owner, WinBuiltinAdministratorsSid) != 0,
                installer: installer_sid(),
            })
        }
    }
    fn user(&self, sid: PSID) -> bool {
        unsafe { IsValidSid(sid) != 0 && EqualSid(self.user.as_ptr() as PSID, sid) != 0 }
    }
    fn admin(sid: PSID) -> bool {
        unsafe { IsValidSid(sid) != 0 && IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0 }
    }
    fn trusted(&self, sid: PSID) -> bool {
        self.user(sid)
            || Self::admin(sid)
            || self.installer.as_ref().is_some_and(|known| unsafe {
                IsValidSid(sid) != 0 && EqualSid(known.as_ptr() as PSID, sid) != 0
            })
            || unsafe { IsValidSid(sid) != 0 && IsWellKnownSid(sid, WinLocalSystemSid) != 0 }
    }
    fn owner(&self, sid: PSID) -> bool {
        self.user(sid) || (self.admin_owner && Self::admin(sid))
    }
}
// Resolve only the qualified local OS-service identity, not a same-named user
// or an arbitrary enabled group. If unavailable, its objects remain untrusted.
fn installer_sid() -> Option<Vec<u32>> {
    let account: Vec<u16> = r"NT SERVICE\TrustedInstaller"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let mut bytes = 0;
        let mut chars = 0;
        let mut kind = 0;
        LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            ptr::null_mut(),
            &mut bytes,
            ptr::null_mut(),
            &mut chars,
            &mut kind,
        );
        if bytes == 0 || bytes > 1024 || chars > 1024 {
            return None;
        }
        let mut sid = vec![0u32; (bytes as usize).div_ceil(size_of::<u32>())];
        let mut domain = vec![0u16; chars as usize];
        if LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut bytes,
            domain.as_mut_ptr(),
            &mut chars,
            &mut kind,
        ) == 0
            || IsValidSid(sid.as_mut_ptr().cast()) == 0
        {
            return None;
        }
        Some(sid)
    }
}

unsafe fn token_info(token: &OwnedHandle, class: TOKEN_INFORMATION_CLASS) -> Result<Vec<usize>> {
    let mut length = 0;
    GetTokenInformation(
        token.as_raw_handle(),
        class,
        ptr::null_mut(),
        0,
        &mut length,
    );
    if length == 0 || length > 64 * 1024 {
        return Err(invalid("invalid token information size"));
    }
    let mut buffer = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    win(GetTokenInformation(
        token.as_raw_handle(),
        class,
        buffer.as_mut_ptr().cast(),
        length,
        &mut length,
    ))?;
    Ok(buffer)
}

// Normalize only the documented DOS/extended-DOS prefix difference. Do not
// canonicalize away aliases, junctions, dot segments or ADS before validation.
fn absolute(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let text: Vec<u16> = path.as_os_str().encode_wide().collect();
    if text.contains(&0) || text.contains(&(b'/' as u16)) {
        return Err(invalid("ambiguous Windows staging path"));
    }
    // Path::components normalizes interior '.', so inspect the original spelling too.
    let spelling = path
        .to_str()
        .ok_or_else(|| invalid("non-Unicode Windows staging path"))?;
    if spelling.split('\\').any(|c| c == "." || c == "..") {
        return Err(invalid("noncanonical Windows staging path"));
    }
    let mut components = path.components();
    let drive = match components.next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => d,
            _ => return Err(invalid("staging recovery requires a local DOS drive")),
        },
        _ => return Err(invalid("staging recovery requires an absolute DOS path")),
    };
    if components.next() != Some(Component::RootDir) {
        return Err(invalid("drive-relative staging path"));
    }
    let mut result = PathBuf::from(format!(
        "\\\\?\\{}:\\",
        char::from(drive.to_ascii_uppercase())
    ));
    for component in components {
        let Component::Normal(name) = component else {
            return Err(invalid("noncanonical staging path"));
        };
        let name = name
            .to_str()
            .ok_or_else(|| invalid("non-Unicode Windows staging path"))?;
        if name.ends_with(['.', ' ']) || name.contains(':') {
            return Err(invalid("ambiguous Windows staging component"));
        }
        result.push(name);
    }
    Ok(result)
}

fn info(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION> {
    unsafe {
        let mut info = zeroed();
        win(GetFileInformationByHandle(file.as_raw_handle(), &mut info))?;
        Ok(info)
    }
}
fn open(path: &Path, directory: bool, deletion: bool) -> Result<File> {
    let access = READ_CONTROL
        | FILE_READ_ATTRIBUTES
        | if directory {
            FILE_LIST_DIRECTORY
        } else {
            FILE_READ_DATA
        }
        | if deletion { DELETE } else { 0 };
    let native_path = wide(path);
    unsafe {
        // Omit SHARE_WRITE and SHARE_DELETE: retain every ancestor and every
        // target handle until validation completes; renames/replacements fail.
        let h = CreateFileW(
            native_path.as_ptr(),
            access,
            if deletion && !directory {
                0
            } else {
                FILE_SHARE_READ
            },
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = File::from_raw_handle(h);
        let i = info(&file)?;
        if GetFileType(h) != FILE_TYPE_DISK
            || i.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || (i.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
        {
            return Err(invalid(
                "staging target is a reparse point or has the wrong type",
            ));
        }
        if !directory && i.dwFileAttributes & FILE_ATTRIBUTE_READONLY != 0 {
            return Err(invalid("read-only Windows staging file"));
        }
        if !directory && i.nNumberOfLinks != 1 {
            return Err(invalid("hard-linked Windows staging file"));
        }
        // Canonicalization here is a comparison, never a source of an unchecked
        // path to delete. The object and its complete ancestor chain are pinned.
        if absolute(&file_path(&file)?)? != absolute(path)? {
            return Err(invalid("noncanonical Windows staging path"));
        }
        Ok(file)
    }
}

fn file_path(file: &File) -> Result<PathBuf> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let mut name = vec![0u16; 32768];
    let len = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            name.as_mut_ptr(),
            name.len() as u32,
            0,
        )
    };
    if len == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if len as usize >= name.len() {
        return Err(invalid("Windows staging path exceeds limit"));
    }
    Ok(PathBuf::from(OsString::from_wide(&name[..len as usize])))
}

fn security(file: &File, id: &Identity, private: bool, ancestor: bool) -> Result<()> {
    unsafe {
        let mut owner = ptr::null_mut();
        let mut acl = ptr::null_mut();
        let mut sd = ptr::null_mut();
        let status = GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut sd,
        );
        if status != 0 {
            return Err(std::io::Error::from_raw_os_error(status as i32).into());
        }
        let _sd = Local(sd);
        if owner.is_null()
            || !(if ancestor {
                id.trusted(owner)
            } else {
                id.owner(owner)
            })
            || acl.is_null()
            || IsValidAcl(acl) == 0
        {
            return Err(invalid("Windows staging ownership or DACL mismatch"));
        }
        let directory = file.metadata()?.is_dir();
        let mut details: ACL_SIZE_INFORMATION = zeroed();
        win(GetAclInformation(
            acl,
            (&mut details as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        ))?;
        let writes = FILE_WRITE_DATA
            | FILE_APPEND_DATA
            | FILE_WRITE_EA
            | FILE_WRITE_ATTRIBUTES
            | FILE_DELETE_CHILD
            | DELETE
            | WRITE_DAC
            | WRITE_OWNER
            | GENERIC_ALL
            | GENERIC_WRITE
            | MAXIMUM_ALLOWED;
        let reads = FILE_READ_DATA
            | FILE_READ_EA
            | FILE_EXECUTE
            | FILE_READ_ATTRIBUTES
            | GENERIC_READ
            | GENERIC_EXECUTE;
        for index in 0..details.AceCount {
            let mut ace = ptr::null_mut();
            win(GetAce(acl, index, &mut ace))?;
            let header = &*(ace as *const ACE_HEADER);
            if u32::from(header.AceType) == ACCESS_DENIED_ACE_TYPE {
                continue;
            }
            // Object/callback/conditional ACE semantics are not guessed.
            if u32::from(header.AceType) != ACCESS_ALLOWED_ACE_TYPE {
                return Err(invalid("unsupported Windows staging ACE"));
            }
            let inherit_only = header.AceFlags & INHERIT_ONLY_ACE as u8 != 0;
            if inherit_only && !(private && directory) {
                continue;
            }
            if usize::from(header.AceSize) < 16 {
                return Err(invalid("invalid Windows staging ACE"));
            }
            let ace = &*(ace as *const ACCESS_ALLOWED_ACE);
            let sid = ptr::addr_of!(ace.SidStart) as PSID;
            if 16 + 4 * u32::from(*(sid as *const u8).add(1)) > u32::from(header.AceSize)
                || IsValidSid(sid) == 0
            {
                return Err(invalid("invalid Windows staging ACE SID"));
            }
            // CREATOR OWNER is replaced with the newly created object owner;
            // it is not an unrelated trustee. Other inherited-only grants on
            // private directories must also be safe for future staging files.
            if IsWellKnownSid(sid, WinCreatorOwnerRightsSid) != 0 {
                continue;
            }
            if inherit_only && IsWellKnownSid(sid, WinCreatorOwnerSid) != 0 {
                continue;
            }
            let forbidden = if ancestor {
                FILE_WRITE_DATA
                    | FILE_WRITE_EA
                    | FILE_WRITE_ATTRIBUTES
                    | WRITE_DAC
                    | WRITE_OWNER
                    | GENERIC_WRITE
                    | GENERIC_ALL
                    | MAXIMUM_ALLOWED
            } else {
                writes | if private { reads } else { 0 }
            };
            if !id.trusted(sid) && ace.Mask & forbidden != 0 {
                return Err(invalid("untrusted Windows staging ACL grant"));
            }
        }
        Ok(())
    }
}

// Open from the volume root down, rejecting each reparse point before the next
// path is resolved. Ancestors may permit creating unrelated children (e.g. C:\\),
// but cannot have an untrusted owner, security-descriptor writer or reparse
// setter (FILE_WRITE_DATA / FILE_WRITE_ATTRIBUTES). Pinned handles
// prevent delete-child/rename attacks without rejecting normal Windows roots.
fn chain(path: &Path, id: &Identity, private_leaf: bool) -> Result<Vec<File>> {
    let path = absolute(path)?;
    let mut ancestors: Vec<_> = path.ancestors().map(Path::to_owned).collect();
    ancestors.reverse();
    let mut handles = Vec::new();
    for (index, ancestor) in ancestors.iter().enumerate() {
        let file = open(ancestor, true, false)?;
        if index == 0 {
            volume(&file, ancestor)?;
        }
        security(
            &file,
            id,
            private_leaf && index + 1 == ancestors.len(),
            !(private_leaf && index + 1 == ancestors.len()),
        )?;
        handles.push(file);
    }
    Ok(handles)
}
fn volume(file: &File, root: &Path) -> Result<()> {
    unsafe {
        if GetDriveTypeW(wide(root).as_ptr()) != DRIVE_FIXED {
            return Err(invalid("staging recovery requires a fixed local volume"));
        }
        let mut flags = 0;
        let mut name = [0u16; 32];
        win(GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut flags,
            name.as_mut_ptr(),
            name.len() as u32,
        ))?;
        if flags & FILE_PERSISTENT_ACLS == 0
            || name[..5] != [b'N' as u16, b'T' as u16, b'F' as u16, b'S' as u16, 0]
        {
            return Err(invalid("staging recovery requires NTFS security semantics"));
        }
        Ok(())
    }
}

pub(super) fn registration(parent: &Path) -> Result<Vec<File>> {
    let id = Identity::current()?;
    // Registration records ownership; it does not delete output data. Validate
    // only app-private journal storage here. Recovery independently validates
    // the destination before deletion, preserving unsupported/shared stages.
    // Do not restrict ordinary exports to recovery's fixed-NTFS/private policy.
    chain(parent, &id, true)
}
fn remove(file: File) -> Result<()> {
    unsafe {
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        win(SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        ))?;
    }
    drop(file);
    Ok(())
}
pub(super) fn recover(path: &Path) -> Result<bool> {
    // Only genuine absence is success-with-no-work; all native API/ACL errors
    // preserve the journal rather than silently downgrading recovery.
    if let Err(e) = fs::symlink_metadata(path) {
        return if e.kind() == std::io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(e.into())
        };
    }
    let id = Identity::current()?;
    let path = absolute(path)?;
    let _journal_chain = chain(
        path.parent()
            .ok_or_else(|| invalid("journal parent missing"))?,
        &id,
        true,
    )?;
    let mut journal = open(&path, false, true)?;
    security(&journal, &id, true, false)?;
    if journal.metadata()?.len() > LIMIT {
        return Err(invalid("invalid staging journal"));
    }
    let mut bytes = Vec::new();
    (&mut journal).take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(invalid("invalid staging journal"));
    }
    let record: Record = serde_json::from_slice(&bytes)?;
    let stage_name = record
        .stage
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("");
    if record.version != 1
        || !record.stage.is_absolute()
        || !random_name(stage_name, ".ditto-", 6)
        || !random_name(&record.marker, ".ditto-owner-", 32)
        || !random_name(&record.wav, ".ditto-wav-", 16)
    {
        return Err(invalid("invalid staging journal record"));
    }
    let stage_path = absolute(&record.stage)?;
    let _stage_chain = chain(
        stage_path
            .parent()
            .ok_or_else(|| invalid("stage parent missing"))?,
        &id,
        false,
    )?;
    let stage = open(&stage_path, true, true)?;
    security(&stage, &id, true, false)?;
    let mut files = Vec::new();
    let mut marker = None;
    for entry in fs::read_dir(&stage_path)?.take(4) {
        let name = entry?.file_name();
        if name != "audio.pcm" && name != record.marker.as_str() && name != record.wav.as_str() {
            return Err(invalid("unexpected staging contents"));
        }
        let mut file = open(&stage_path.join(&name), false, true)?;
        let is_marker = name == record.marker.as_str();
        security(&file, &id, is_marker, false)?;
        if is_marker {
            if file.metadata()?.len() != record.marker.len() as u64 {
                return Err(invalid("staging marker mismatch"));
            }
            let mut content = Vec::new();
            (&mut file).take(LIMIT + 1).read_to_end(&mut content)?;
            if content != record.marker.as_bytes() {
                return Err(invalid("staging marker mismatch"));
            }
            marker = Some(file);
        } else {
            files.push(file);
        }
    }
    let marker = marker.ok_or_else(|| invalid("staging owner marker missing"))?;
    // No deletion starts until the full bounded set has passed native checks.
    for file in files {
        remove(file)?;
    }
    remove(marker)?;
    remove(stage)?;
    remove(journal)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_is_not_inferred_from_the_callers_ability_to_write() {
        let id = Identity::current().unwrap();
        assert!(id.owner(id.user.as_ptr() as PSID));
        for kind in [WinWorldSid, WinBuiltinUsersSid, WinAuthenticatedUserSid] {
            let mut sid = [0u32; 17];
            let mut len = std::mem::size_of_val(&sid) as u32;
            unsafe {
                win(CreateWellKnownSid(
                    kind,
                    ptr::null_mut(),
                    sid.as_mut_ptr().cast(),
                    &mut len,
                ))
                .unwrap();
            }
            assert!(
                !id.owner(sid.as_mut_ptr().cast()),
                "accepted unrelated owner SID type {kind}"
            );
        }
    }

    #[test]
    fn path_validation_only_normalizes_the_extended_dos_prefix() {
        assert_eq!(
            absolute(Path::new(r"C:\Users\name\.ditto-Ab1234")).unwrap(),
            absolute(Path::new(r"\\?\C:\Users\name\.ditto-Ab1234")).unwrap()
        );
        for path in [
            r"C:\Users\..\name",
            r"C:\Users\.\name",
            r"C:\Users\name:stream",
            r"C:\Users\name.",
            r"C:\Users\name ",
            r"\\server\share\stage",
            r"\\?\UNC\server\share\stage",
            r"\\.\C:\stage",
            "C:/Users/name",
        ] {
            assert!(absolute(Path::new(path)).is_err(), "accepted {path}");
        }
    }
}
