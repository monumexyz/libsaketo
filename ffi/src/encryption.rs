use std::ffi::{c_char, CStr, CString};
use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, KeyInit, Nonce};
use aes_gcm::aead::rand_core::RngCore;
use argon2::Argon2;
use base64::Engine;

#[unsafe(no_mangle)]
pub extern "C" fn encrypt_data(password: *const c_char, data: *const c_char) -> *mut c_char {
    let password = unsafe { CStr::from_ptr(password) }.to_str().unwrap_or("").to_string();
    let data = unsafe { CStr::from_ptr(data) }.to_str().unwrap_or("").to_string();
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let mut key = [0u8; 32];
    Argon2::default().hash_password_into(password.as_bytes(), &salt, &mut key).unwrap();
    let key = Key::<Aes256Gcm>::from_slice(&key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = match cipher.encrypt(&nonce, data.as_bytes()) {
        Ok(ct) => ct,
        Err(_) => return CString::new("ENCRYPTION_FAILED").unwrap().into_raw(),
    };
    // Combine: ciphertext + nonce + salt
    let mut combined = Vec::new();
    combined.extend_from_slice(&ciphertext);
    combined.extend_from_slice(&nonce);
    combined.extend_from_slice(&salt);
    let encoded = base64::engine::general_purpose::STANDARD.encode(&combined);
    CString::new(encoded).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub extern "C" fn decrypt_data(password: *const c_char, encrypted_data: *const c_char) -> *mut c_char {
    let password = unsafe { CStr::from_ptr(password) }.to_str().unwrap_or("").to_string();
    let encrypted_data = unsafe { CStr::from_ptr(encrypted_data) }.to_str().unwrap_or("").to_string();
    let encrypted_data = match base64::engine::general_purpose::STANDARD.decode(encrypted_data) {
        Ok(data) => data,
        Err(_) => return CString::new("INVALID_BASE64").unwrap().into_raw(),
    };

    if encrypted_data.len() < 28 {
        return CString::new("INVALID_DATA").unwrap().into_raw();
    }

    let ciphertext_len = encrypted_data.len() - 28;
    let (ciphertext, rest) = encrypted_data.split_at(ciphertext_len);
    let (nonce, salt) = rest.split_at(12);
    let mut key = [0u8; 32];
    Argon2::default().hash_password_into(password.as_bytes(), salt, &mut key).unwrap();
    let key = Key::<Aes256Gcm>::from_slice(&key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce);
    let plaintext = match cipher.decrypt(nonce, ciphertext) {
        Ok(data) => data,
        Err(_) => {
            return CString::new("DECRYPTION_FAILED").unwrap().into_raw();
        }
    };
    CString::new(plaintext).unwrap().into_raw()
}