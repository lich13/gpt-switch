use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path};

pub type Result<T> = std::result::Result<T, AppError>;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}
impl AppError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            line: None,
            column: None,
        }
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for AppError {}
pub fn io_error(_: std::io::Error) -> AppError {
    AppError::new("IO", "文件操作失败，请检查路径、权限和磁盘空间")
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(AppError::new(
                "SYMLINK",
                "目标文件是符号链接，请选择实际文件所在目录",
            ))
        }
        Ok(m) if !m.is_file() || m.len() > 2 * 1024 * 1024 => {
            return Err(AppError::new(
                "FILE_TYPE",
                "文件必须是小于 2 MiB 的普通文件",
            ))
        }
        Ok(_) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(e)),
    }
    fs::read(path).map(Some).map_err(io_error)
}
pub fn revision(bytes: Option<&[u8]>) -> String {
    bytes.map(digest).unwrap_or_else(|| "missing".into())
}
pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(io_error)?;
    protect(path, true)
}
#[cfg(unix)]
pub fn protect(path: &Path, directory: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
    )
    .map_err(io_error)
}
#[cfg(windows)]
pub fn protect(path: &Path, _directory: bool) -> Result<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
                SE_FILE_OBJECT,
            },
            GetSecurityDescriptorDacl, DACL_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION,
        },
    };
    // OWNER RIGHTS resolves to the creator/owner of this private object. A protected
    // DACL prevents broader permissions inherited from AppData or temp directories.
    let sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)\0".encode_utf16().collect();
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let mut descriptor = ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        ) == 0
        {
            return Err(AppError::new("PERMISSIONS", "无法限制文件访问权限"));
        }
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = ptr::null_mut();
        let valid = GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted);
        let code = if valid != 0 && present != 0 {
            SetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                acl,
                ptr::null_mut(),
            )
        } else {
            1
        };
        LocalFree(descriptor);
        if code != 0 {
            return Err(AppError::new("PERMISSIONS", "无法限制文件访问权限"));
        }
    }
    Ok(())
}
pub fn atomic_write(path: &Path, bytes: &[u8], expected: Option<&str>) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::new("PATH", "目标路径无效"))?;
    if !parent.exists() {
        private_dir(parent)?;
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(io_error)?;
    protect(temp.path(), false)?;
    temp.write_all(bytes).map_err(io_error)?;
    temp.as_file().sync_all().map_err(io_error)?;
    let current = read_optional(path)?;
    if expected.is_some_and(|e| e != revision(current.as_deref())) {
        return Err(AppError::new(
            "CONFLICT",
            "文件已被其他程序修改，请重新读取后再操作",
        ));
    }
    temp.persist(path).map_err(|e| io_error(e.error))?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(io_error)?;
    let actual = read_optional(path)?;
    if actual.as_deref() != Some(bytes) {
        return Err(AppError::new(
            "VERIFY",
            "文件写入后发生变化，请重新读取确认",
        ));
    }
    Ok(())
}
