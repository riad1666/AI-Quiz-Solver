use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::slice;

use thiserror::Error;
use windows::core::PWSTR;
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

const OPENROUTER_TARGET_NAME: &str = "KNUDiagnosis/OpenRouterApiKey";

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("Windows Credential Manager error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("stored credential is not valid UTF-8")]
    InvalidUtf8,
}

fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// Stores the OpenRouter API key in Windows Credential Manager (never in SQLite or logs).
pub fn save_openrouter_api_key(api_key: &str) -> Result<(), SecretError> {
    save_secret(OPENROUTER_TARGET_NAME, api_key)
}

/// Loads the OpenRouter API key from Windows Credential Manager, returning `None` if absent.
pub fn load_openrouter_api_key() -> Result<Option<String>, SecretError> {
    load_secret(OPENROUTER_TARGET_NAME)
}

/// Deletes the OpenRouter API key from Windows Credential Manager.
pub fn delete_openrouter_api_key() -> Result<(), SecretError> {
    delete_secret(OPENROUTER_TARGET_NAME)
}

pub fn save_secret(target_name: &str, secret: &str) -> Result<(), SecretError> {
    let mut wide_target = to_wide_null(target_name);
    let mut wide_user = to_wide_null("KNUDiagnosis");
    let mut bytes = secret.as_bytes().to_vec();

    let cred = CREDENTIALW {
        Flags: Default::default(),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(wide_target.as_mut_ptr()),
        Comment: PWSTR::null(),
        LastWritten: Default::default(),
        CredentialBlobSize: bytes.len() as u32,
        CredentialBlob: bytes.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: PWSTR::null(),
        UserName: PWSTR(wide_user.as_mut_ptr()),
    };

    unsafe { CredWriteW(&cred, 0)? };
    Ok(())
}

pub fn load_secret(target_name: &str) -> Result<Option<String>, SecretError> {
    let wide_target = to_wide_null(target_name);
    let mut cred_ptr: *mut CREDENTIALW = std::ptr::null_mut();

    let result = unsafe {
        CredReadW(
            windows::core::PCWSTR(wide_target.as_ptr()),
            CRED_TYPE_GENERIC,
            0,
            &mut cred_ptr,
        )
    };

    if let Err(err) = result {
        // ERROR_NOT_FOUND (1168) -> Win32 HRESULT 0x80070490
        if err.code() == windows::core::HRESULT::from_win32(1168) {
            return Ok(None);
        }
        return Err(SecretError::Windows(err));
    }

    if cred_ptr.is_null() {
        return Ok(None);
    }

    let secret_string = unsafe {
        let cred = &*cred_ptr;
        let slice = slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize);
        let parsed = std::str::from_utf8(slice)
            .map(|s| s.to_string())
            .map_err(|_| SecretError::InvalidUtf8);
        CredFree(cred_ptr as *const _);
        parsed
    }?;

    Ok(Some(secret_string))
}

pub fn delete_secret(target_name: &str) -> Result<(), SecretError> {
    let wide_target = to_wide_null(target_name);
    let result = unsafe {
        CredDeleteW(
            windows::core::PCWSTR(wide_target.as_ptr()),
            CRED_TYPE_GENERIC,
            0,
        )
    };
    if let Err(err) = result {
        if err.code() == windows::core::HRESULT::from_win32(1168) {
            return Ok(());
        }
        return Err(SecretError::Windows(err));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_manager_roundtrip() {
        let target = "KNUDiagnosis/TestSecretRoundtrip";
        save_secret(target, "test-secret-value").expect("save secret");
        let loaded = load_secret(target).expect("load secret");
        assert_eq!(loaded.as_deref(), Some("test-secret-value"));
        delete_secret(target).expect("delete secret");
        let after_delete = load_secret(target).expect("load after delete");
        assert_eq!(after_delete, None);
    }
}
