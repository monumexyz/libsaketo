use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic;
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use monero_fastscan::service::{FastscanService, ScannedInput, ScannedOutput, ServiceCommand, ServiceResponse};

#[repr(i32)]
#[allow(dead_code)]
pub enum DartCObjectType {
    DartNull = 0,
    DartBool = 1,
    DartInt32 = 2,
    DartInt64 = 3,
}

#[repr(C)]
pub union DartCObjectValue {
    pub as_bool: bool,
    pub as_int32: i32,
    pub as_int64: i64,
}

#[repr(C)]
pub struct DartCObject {
    pub ty: DartCObjectType,
    pub value: DartCObjectValue,
}

type DartPostCObjectFnType = unsafe extern "C" fn(port_id: i64, message: *mut DartCObject) -> bool;

static mut DART_POST_COBJECT: Option<DartPostCObjectFnType> = None;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn register_dart_post_cobject(ptr: DartPostCObjectFnType) {
    unsafe {
        DART_POST_COBJECT = Some(ptr);
    }
}

pub struct FFIService {
    rt: Runtime,
    tx: mpsc::Sender<(ServiceCommand, oneshot::Sender<ServiceResponse>)>,
    _handle: JoinHandle<()>,
    _notify_handle: JoinHandle<()>,
}

unsafe fn safe_cstr_to_string(ptr: *const c_char) -> Result<String, String> { unsafe {
        if ptr.is_null() {
            return Err("Null pointer received".to_string());
        }
        CStr::from_ptr(ptr)
            .to_str()
            .map(|s| s.to_string())
            .map_err(|e| format!("UTF-8 error: {}", e))
    }
}

fn return_json_error(msg: &str) -> *mut c_char {
    let json = format!(r#"{{"type":"Error","error":"{}"}}"#, msg);
    CString::new(json).unwrap().into_raw()
}

#[repr(C)]
pub struct FFIServiceResult {
    pub service_ptr: *mut FFIService,
    pub error_msg: *mut c_char,
}

#[unsafe(no_mangle)]
pub extern "C" fn fastscan_service_new(
    priv_spend_hex: *const c_char,
    start_height: u64,
    inputs: *const c_char,
    outputs: *const c_char,
    rpc_url: *const c_char,
    rpc_port: u16,
    dart_port: i64,
) -> FFIServiceResult {
    let result = panic::catch_unwind(|| {
        let make_err = |msg: &str| FFIServiceResult {
            service_ptr: std::ptr::null_mut(),
            error_msg: CString::new(msg).unwrap().into_raw(),
        };

        let priv_spend_str = match unsafe { safe_cstr_to_string(priv_spend_hex) } {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let rpc_url_str = match unsafe { safe_cstr_to_string(rpc_url) } {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let inputs_str = match unsafe { safe_cstr_to_string(inputs) } {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let outputs_str = match unsafe { safe_cstr_to_string(outputs) } {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };

        let inputs_vec: Vec<ScannedInput> = match serde_json::from_str(&inputs_str) {
            Ok(v) => v,
            Err(_) => return make_err("Invalid inputs JSON"),
        };
        let outputs_vec: Vec<ScannedOutput> = match serde_json::from_str(&outputs_str) {
            Ok(v) => v,
            Err(_) => return make_err("Invalid outputs JSON"),
        };

        let priv_spend_bytes: [u8; 32] = match hex::decode(&priv_spend_str) {
            Ok(vec) => match vec.try_into() {
                Ok(arr) => arr,
                Err(_) => return make_err("Private spend key must be 32 bytes"),
            },
            Err(_) => return make_err("Invalid hex for private spend key"),
        };

        let rt = match Runtime::new() {
            Ok(r) => r,
            Err(e) => return make_err(&format!("Failed to create runtime: {}", e)),
        };

        let (notify_tx, mut notify_rx) = mpsc::channel::<monero_fastscan::service::ServiceNotification>(100);
        let service_init_result = rt.block_on(async {
            FastscanService::new(
                priv_spend_bytes,
                start_height,
                Some(inputs_vec),
                Some(outputs_vec),
                format!("http://{}", rpc_url_str.trim_start_matches("http://").trim_start_matches("https://")), // TODO: Handle https properly, SSL etc.
                rpc_port,
                200,
                Some(notify_tx)
            ).await
        });

        match service_init_result {
            Ok(service) => {
                let (tx, rx) = mpsc::channel(32);
                let handle = rt.spawn(async move { service.run(rx).await; });

                let notify_handle = rt.spawn(async move {
                    while let Some(notification) = notify_rx.recv().await {
                        unsafe {
                            if let Some(post_fn) = DART_POST_COBJECT {
                                let mut obj = match notification {
                                    monero_fastscan::service::ServiceNotification::FoundTransaction => DartCObject {
                                        ty: DartCObjectType::DartInt32,
                                        value: DartCObjectValue { as_int32: 0 },
                                    },
                                };
                                post_fn(dart_port, &mut obj);
                            }
                        }
                    }
                });

                FFIServiceResult {
                    service_ptr: Box::into_raw(Box::new(FFIService {
                        rt,
                        tx,
                        _handle: handle,
                        _notify_handle: notify_handle,
                    })),
                    error_msg: std::ptr::null_mut(),
                }
            },
            Err(e) => make_err(&format!("{}", e)),
        }
    });

    result.unwrap_or_else(|_| FFIServiceResult {
        service_ptr: std::ptr::null_mut(),
        error_msg: CString::new("Critical FFI Panic").unwrap().into_raw(),
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn free_error_msg(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
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
            4 => ServiceCommand::Data,
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