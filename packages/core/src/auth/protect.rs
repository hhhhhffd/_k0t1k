//! Encryption of `accounts.json` with Windows DPAPI (`CryptProtectData`, current user scope).
//!
//! DPAPI keys are derived from the Windows user's credentials, so nothing is stored in the system and the portable
//! folder stays self-contained; the price is that the tokens only decrypt for the same user on the same PC (moving
//! the folder elsewhere means signing in again). Other platforms are development targets only and keep the file
//! as plain JSON readable by the owner alone (see `settings::write_json_private`).

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    use crate::error::{Error, Result};

    /// Mixed into the key so other programs of the same user cannot decrypt the file with a bare `CryptUnprotectData`.
    const ENTROPY: &[u8] = b"k0t1k-accounts-v1";

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr().cast_mut() }
    }

    /// Copies the system-allocated output blob and frees it.
    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: on success the API returns `cbData` bytes at `pbData`, allocated with LocalAlloc.
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: the buffer came from the API and is freed exactly once.
        unsafe { LocalFree(out.pbData.cast()) };
        bytes
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>> {
        let (input, entropy) = (blob(data), blob(ENTROPY));
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: the input blobs point at live slices for the duration of the call; `out` is written by the API.
        let ok = unsafe {
            CryptProtectData(&input, std::ptr::null(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        if ok == 0 {
            return Err(Error::Other(format!("cannot encrypt the account tokens: {}", std::io::Error::last_os_error())));
        }
        Ok(take(out))
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
        let (input, entropy) = (blob(data), blob(ENTROPY));
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: as in `protect`; no description is requested.
        let ok = unsafe {
            CryptUnprotectData(&input, std::ptr::null_mut(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        if ok == 0 {
            return Err(Error::Other(format!("cannot decrypt the account tokens: {}", std::io::Error::last_os_error())));
        }
        Ok(take(out))
    }
}

#[cfg(windows)]
pub use imp::{protect, unprotect};
