//! In-process JSON bridge for `web/calculator-wasm.js`.

use crate::{calculate_json, MAX_REQUEST_BYTES};
use std::cell::RefCell;

struct Buffers {
    request: [u8; MAX_REQUEST_BYTES + 1],
    response: String,
}

thread_local! {
    // The last request byte is reserved so oversized requests can use the normal JSON error.
    static BUFFERS: RefCell<Buffers> = const { RefCell::new(Buffers {
        request: [0; MAX_REQUEST_BYTES + 1],
        response: String::new(),
    }) };
}

#[unsafe(no_mangle)]
pub extern "C" fn calculator_max_request_bytes() -> usize {
    MAX_REQUEST_BYTES
}

#[unsafe(no_mangle)]
pub extern "C" fn calculator_request_ptr() -> *mut u8 {
    BUFFERS.with_borrow_mut(|buffers| buffers.request.as_mut_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn calculator_run(request_len: usize) -> usize {
    BUFFERS.with_borrow_mut(|buffers| {
        let request_len = request_len.min(buffers.request.len());
        buffers.response = calculate_json(&buffers.request[..request_len]);
        buffers.response.len()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn calculator_response_ptr() -> *const u8 {
    BUFFERS.with_borrow(|buffers| buffers.response.as_ptr())
}
