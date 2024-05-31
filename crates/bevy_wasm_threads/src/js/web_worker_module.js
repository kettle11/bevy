// synchronously, using the browser, import wasm_bindgen shim JS scripts
import init, { bevy_web_worker_entry_point } from "WASM_BINDGEN_SHIM_URL";

// Wait for the main thread to send us the shared module/memory and work context.
// Once we've got it, initialize it all with the `wasm_bindgen` global we imported via
// `importScripts`.

function runWorker(entryPtr) {
    let complete = bevy_web_worker_entry_point(entryPtr);
    if (complete == 1) {
        // Once done, terminate web worker
        close();
    }
}

self.onmessage = event => {
    let [module, memory, ptr] = event.data;

    init(module, memory).catch(err => {
        // Propagate to main `onerror`:
        setTimeout(() => {
            throw err;
        });
        // Rethrow to keep promise rejected and prevent execution of further commands:
        throw err;
    }).then(() => {
        // Enter rust code by calling entry point defined in `lib.rs`.
        // This executes closure defined by work context.
        runWorker(ptr);
    });

};