use std::ffi::{c_char, CStr, CString};
use aes_gcm::aead::OsRng;
use curve25519_dalek::Scalar;
use monero_serai::primitives::keccak256;
use monero_wallet::address::{AddressType, Network};
use zeroize::Zeroizing;
use crate::types::{c_str_to_string, get_block_height_from_unix_time, ResultWithMessage};

#[unsafe(no_mangle)]
extern "C" fn generate_polyseed_mnemonic() -> *mut c_char {
    let seed = polyseed::Polyseed::new(&mut OsRng, polyseed::Language::English);
    let c_string = CString::new(seed.to_string().to_string()).unwrap();
    c_string.into_raw()
}

#[unsafe(no_mangle)]
extern "C" fn is_valid_polyseed_mnemonic(mnemonic: *const c_char, language_code: *const c_char) -> ResultWithMessage {
    let language = match &c_str_to_string(language_code)[..] {
        "en" => polyseed::Language::English,
        "es" => polyseed::Language::Spanish,
        "fr" => polyseed::Language::French,
        "it" => polyseed::Language::Italian,
        "ja" => polyseed::Language::Japanese,
        "ko" => polyseed::Language::Korean,
        "cs" => polyseed::Language::Czech,
        "pt" => polyseed::Language::Portuguese,
        "zh-CN" => polyseed::Language::ChineseSimplified,
        "zh-TW" => polyseed::Language::ChineseTraditional,
        _ => polyseed::Language::English,
    };
    let seed = polyseed::Polyseed::from_string(language, zeroize::Zeroizing::new(c_str_to_string(mnemonic)));
    let message = if seed.is_ok() {
        ""
    } else {
        match seed.clone().err().unwrap() {
            // TODO: Make this error messages local.
            polyseed::PolyseedError::InvalidSeed => "Invalid seed. Please check your mnemonic.",
            polyseed::PolyseedError::InvalidEntropy => "Invalid entropy. Please check your mnemonic.",
            polyseed::PolyseedError::InvalidChecksum => "Invalid checksum. Please check your mnemonic.",
            polyseed::PolyseedError::UnsupportedFeatures => "Unsupported features. Please check your mnemonic.",
        }
    };
    ResultWithMessage::new(seed.is_ok(), message)
}

#[unsafe(no_mangle)]
pub extern "C" fn get_primary_address_polyseed(mnemonic: *const c_char) -> *mut c_char {
    let mnemonic = unsafe { CStr::from_ptr(mnemonic) }.to_str().unwrap_or("").to_string();
    let seed = polyseed::Polyseed::from_string(polyseed::Language::English, Zeroizing::new(mnemonic)).unwrap();
    let priv_spend = Scalar::from_bytes_mod_order(*seed.key()).to_bytes();
    let priv_view = keccak256(priv_spend);
    let pub_spend = Scalar::from_bytes_mod_order(priv_spend) * curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    let pub_view = Scalar::from_bytes_mod_order(priv_view) * curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    let address = monero_wallet::address::MoneroAddress::new(Network::Mainnet, AddressType::Legacy, pub_spend, pub_view);
    CString::new(address.to_string()).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub extern "C" fn get_block_height_polyseed(mnemonic: *const c_char) -> i64 {
    let mnemonic = unsafe { CStr::from_ptr(mnemonic) }.to_str().unwrap_or("").to_string();
    let seed = polyseed::Polyseed::from_string(polyseed::Language::English, Zeroizing::new(mnemonic)).unwrap();
    get_block_height_from_unix_time(seed.birthday().try_into().unwrap())
}

#[unsafe(no_mangle)]
extern "C" fn generate_legacy_mnemonic() -> *mut c_char {
    let seed = monero_seed::Seed::new(&mut OsRng, monero_seed::Language::English);
    let c_string = CString::new(seed.to_string().to_string()).unwrap();
    c_string.into_raw()
}

#[unsafe(no_mangle)]
extern "C" fn is_valid_legacy_mnemonic(mnemonic: *const c_char, language_code: *const c_char) -> ResultWithMessage {
    let language = match &c_str_to_string(language_code)[..] {
        "zh" => monero_seed::Language::Chinese,
        "en" => monero_seed::Language::English,
        "nl" => monero_seed::Language::Dutch,
        "fr" => monero_seed::Language::French,
        "es" => monero_seed::Language::Spanish,
        "de" => monero_seed::Language::German,
        "it" => monero_seed::Language::Italian,
        "pt" => monero_seed::Language::Portuguese,
        "jp" => monero_seed::Language::Japanese,
        "ru" => monero_seed::Language::Russian,
        "eo" => monero_seed::Language::Esperanto,
        "lj" => monero_seed::Language::Lojban,
        "en_deprecated" => monero_seed::Language::DeprecatedEnglish,
        _ => monero_seed::Language::English,
    };
    let seed = monero_seed::Seed::from_string(language, zeroize::Zeroizing::new(c_str_to_string(mnemonic)));
    let message = if seed.is_ok() {
        ""
    } else {
        match seed.clone().err().unwrap() {
            monero_seed::SeedError::InvalidSeed => "Invalid seed. Please check your mnemonic.",
            monero_seed::SeedError::InvalidChecksum => "Invalid checksum. Please check your 25th word.",
            monero_seed::SeedError::DeprecatedEnglishWithChecksum => "Deprecated English language option included a checksum. Please check your mnemonic.",
        }
    };
    ResultWithMessage::new(seed.is_ok(), message)
}

#[unsafe(no_mangle)]
pub extern "C" fn get_primary_address_monero_seed(mnemonic: *const c_char) -> *mut c_char {
    let mnemonic = unsafe { CStr::from_ptr(mnemonic) }.to_str().unwrap_or("").to_string();
    let seed = monero_seed::Seed::from_string(monero_seed::Language::English, Zeroizing::new(mnemonic)).unwrap();
    let priv_spend = Scalar::from_bytes_mod_order(*seed.entropy()).to_bytes();
    let priv_view = keccak256(priv_spend);
    let pub_spend = Scalar::from_bytes_mod_order(priv_spend) * curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    let pub_view = Scalar::from_bytes_mod_order(priv_view) * curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    let address = monero_wallet::address::MoneroAddress::new(Network::Mainnet, AddressType::Legacy, pub_spend, pub_view);
    CString::new(address.to_string()).unwrap().into_raw()
}