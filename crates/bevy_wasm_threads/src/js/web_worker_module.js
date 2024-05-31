// synchronously, using the browser, import wasm_bindgen shim JS scripts
import init, { bevy_web_worker_entry_point } from "WASM_BINDGEN_SHIM_URL";

// Wait for the main thread to send us the shared module/memory and work context.
// Once we've got it, initialize it all with the `wasm_bindgen` global we imported via
// `importScripts`.

let first_event_received = false;
let initialized = false;
let enqueued_message_events = [];

// Messages can be received before this worker is initialized.
// Those need to be deferred and handled later.
function handleMessages() {
    for (const message_event of enqueued_message_events) {
        self.web_worker_message_event = message_event;
        runWorker(0);
    }
    enqueued_message_events = [];
}

function runWorker(entryPtr) {
    let complete = bevy_web_worker_entry_point(entryPtr);
    if (complete == 1) {
        // Once done, terminate web worker
        close();
    }
}

self.onmessage = event => {
    console.log("Message received: ");
    console.log(event.data);

    // The first message sets up the Wasm module.
    if (!first_event_received) {
        let [module, memory, ptr] = event.data;

        first_event_received = true;
        init(module, memory).catch(err => {
            console.log("ERROR: ");
            console.log(err);
            // Propagate to main `onerror`:
            setTimeout(() => {
                throw err;
            });
            // Rethrow to keep promise rejected and prevent execution of further commands:
            throw err;
        }).then(() => {
            // Enter rust code by calling entry point defined in `lib.rs`.
            // This executes closure defined by work context.
            console.log("HERE0");
            runWorker(ptr);
            handleMessages();
            console.log("HERE1");

            initialized = true;
        });
    } else if (!initialized) {
        enqueued_message_events.push(event);
    } else {
        enqueued_message_events.push(event);
        handleMessages();
    }
};