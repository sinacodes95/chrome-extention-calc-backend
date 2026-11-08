/** Load the compiled WebAssembly bytes and return a synchronous JSON calculator. */
export async function loadCalculator(bytes) {
  const { instance } = await WebAssembly.instantiate(bytes);
  const wasm = instance.exports;
  const maxRequestBytes = wasm.calculator_max_request_bytes();
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();

  return function calculateJson(request) {
    if (typeof request !== "string") throw new TypeError("The calculator expects a JSON string.");
    const requestPtr = wasm.calculator_request_ptr();
    const requestBuffer = new Uint8Array(wasm.memory.buffer, requestPtr, maxRequestBytes);
    const { read, written } = encoder.encodeInto(request, requestBuffer);
    const requestLength = read === request.length ? written : maxRequestBytes + 1;
    const responseLength = wasm.calculator_run(requestLength);

    // Rust may grow linear memory while calculating, so read the current buffer.
    const responseBuffer = new Uint8Array(wasm.memory.buffer, wasm.calculator_response_ptr(), responseLength);
    return decoder.decode(responseBuffer);
  };
}
