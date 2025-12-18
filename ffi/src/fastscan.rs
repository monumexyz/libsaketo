use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic;
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use monero_fastscan::service::{FastscanService, ServiceCommand, ServiceResponse};

pub struct FFIService {
    rt: Runtime,
    tx: mpsc::Sender<(ServiceCommand, oneshot::Sender<ServiceResponse>)>,
    _handle: JoinHandle<()>,
}

unsafe fn safe_cstr_to_string(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Err("Null pointer received".to_string());
    }
    CStr::from_ptr(ptr)
        .to_str()
        .map(|s| s.to_string())
        .map_err(|e| format!("UTF-8 error: {}", e))
}

fn return_json_error(msg: &str) -> *mut c_char {
    let json = format!(r#"{{"type":"Error","error":"{}"}}"#, msg);
    CString::new(json).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub extern "C" fn fastscan_service_new(
    priv_spend_hex: *const c_char,
    start_height: u64,
    rpc_url: *const c_char,
    rpc_port: u16,
) -> *mut FFIService {
    let result = panic::catch_unwind(|| {
        let priv_spend_str = unsafe {
            match safe_cstr_to_string(priv_spend_hex) {
                Ok(s) => s,
                Err(_) => return std::ptr::null_mut(),
            }
        };
        let rpc_url_str = unsafe {
            match safe_cstr_to_string(rpc_url) {
                Ok(s) => s,
                Err(_) => return std::ptr::null_mut(),
            }
        };

        let priv_spend_bytes: [u8; 32] = match hex::decode(&priv_spend_str) {
            Ok(vec) => match vec.try_into() {
                Ok(arr) => arr,
                Err(_) => return std::ptr::null_mut(), // Wrong length
            },
            Err(_) => return std::ptr::null_mut(), // Invalid Hex
        };

        let rt = match Runtime::new() {
            Ok(r) => r,
            Err(_) => return std::ptr::null_mut(),
        };


        let service_init_result = rt.block_on(async {
            FastscanService::new(priv_spend_bytes, start_height, rpc_url_str, rpc_port, 200).await
        });

        match service_init_result {
            Ok(service) => {
                let (tx, rx) = mpsc::channel(32);

                // Spawn the actor loop
                let handle = rt.spawn(async move {
                    service.run(rx).await;
                });
                
                let ffi_service = FFIService {
                    rt,
                    tx,
                    _handle: handle,
                };
                Box::into_raw(Box::new(ffi_service))
            },
            Err(e) => {
                eprintln!("FastscanService init failed: {}", e);
                std::ptr::null_mut()
            }
        }
    });

    result.unwrap_or_else(|_| std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn fastscan_service_send_command(
    service_ptr: *mut FFIService,
    command: u8,
) -> *mut c_char {
    if service_ptr.is_null() {
        return return_json_error("Null service pointer");
    }

    let result = panic::catch_unwind(|| {
        let service = unsafe { &mut *service_ptr };

        let cmd = match command {
            0 => ServiceCommand::Start,
            1 => ServiceCommand::Stop,
            2 => ServiceCommand::Status,
            3 => ServiceCommand::Transactions,
            _ => return return_json_error("Invalid command ID"),
        };

        let (resp_tx, resp_rx) = oneshot::channel();

        match service.rt.block_on(service.tx.send((cmd, resp_tx))) {
            Ok(_) => {
                match service.rt.block_on(resp_rx) {
                    Ok(response_enum) => {
                        match serde_json::to_string(&response_enum) {
                            Ok(json_str) => CString::new(json_str).unwrap().into_raw(),
                            Err(_) => return_json_error("JSON serialization failed"),
                        }
                    },
                    Err(_) => return_json_error("Service dropped the response channel (Task crashed?)"),
                }
            },
            Err(_) => return_json_error("Service channel closed (Service is dead)"),
        }
    });

    result.unwrap_or_else(|_| return_json_error("FFI Panic: Critical internal error"))
}

#[unsafe(no_mangle)]
pub extern "C" fn fastscan_service_free(service_ptr: *mut FFIService) {
    if !service_ptr.is_null() {
        let _ = panic::catch_unwind(|| {
            unsafe {
                let _ = Box::from_raw(service_ptr);
            };
        });
    }
}