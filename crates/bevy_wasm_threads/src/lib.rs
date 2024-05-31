//! This crate facilitates multithreading on Wasm for Bevy engine internals

use send_wrapper::SendWrapper;
use std::cell::RefCell;
use std::sync::{mpsc::*, Arc, Mutex, OnceLock};
use wasm_bindgen::prelude::*;
use web_sys::MessageEvent;

#[derive(Debug)]
struct BrowserMainThreadChannels {
    to_browser_main_waker_sender: Sender<()>,
    to_browser_main_tasks_sender: Sender<Box<dyn FnOnce() + Send + 'static>>,
}

static BROWSER_MAIN_CHANNELS: OnceLock<BrowserMainThreadChannels> = OnceLock::new();

thread_local! {
    static BEVY_MAIN_WORKER: RefCell<Option<web_sys::Worker>> = RefCell::new(None);
    static BROWSER_MAIN_TASKS: RefCell<Option<Receiver<Box<dyn FnOnce() + Send + 'static>>>> = RefCell::new(None);
}

/// Call this on the browser's main thread to dispatch tasks sent to it.
pub fn handle_browser_main_tasks() {
    //web_sys::js_sys::eval("console.log('RUNNING BROWSER MAIN TASKS')").unwrap();

    BROWSER_MAIN_TASKS.with(|v| {
        if let Ok(v) = v.try_borrow_mut() {
            let browser_main_tasks_receiver = v.as_ref().unwrap();
            while let Ok(task) = browser_main_tasks_receiver.try_recv() {
                web_sys::js_sys::eval("console.log('RUNNING BROWSER MAIN TASK')").unwrap();
                task();
            }
        }
    });
}

/// Call this with a function that wraps normal Bevy app initialization
/// to run Bevy off of the browser's main thread.
pub fn run_bevy_main(f: impl FnOnce() + Send + 'static) {
    bevy_utils::tracing::info!("Spawning Bevy's main web worker");

    let (to_browser_main_tasks_sender, browser_main_tasks_receiver) =
        std::sync::mpsc::channel::<Box<dyn FnOnce() + Send + 'static>>();

    BROWSER_MAIN_TASKS.with(|v| *v.borrow_mut() = Some(browser_main_tasks_receiver));

    // This runs tasks sent to the main thread by other threads.
    let run_browser_main_tasks = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
        handle_browser_main_tasks();
    }) as Box<dyn FnMut()>);

    // Setup the waker for the main thread.
    {
        let (to_browser_main_waker_sender, to_browser_main_waker_receiver) =
            std::sync::mpsc::channel::<()>();

        BROWSER_MAIN_CHANNELS
            .set(BrowserMainThreadChannels {
                to_browser_main_tasks_sender,
                to_browser_main_waker_sender,
            })
            .unwrap();

        // This creates a worker thread that waits until its signaled and then
        // wakes up the main thread by postMessage-ing it.
        let bevy_main_waker_worker = spawn_web_worker(
            move || {
                while let Ok(_) = to_browser_main_waker_receiver.recv() {
                    web_sys::js_sys::eval("self.postMessage(0)").unwrap();
                }
            },
            Some("bevy_browser_main_waker"),
        );

        // When the worker sends a message back run the main thread tasks.
        bevy_main_waker_worker.set_onmessage(Some(run_browser_main_tasks.as_ref().unchecked_ref()));
    }

    run_browser_main_tasks.forget();

    // Setup
    let bevy_main_worker = spawn_web_worker(f, Some("bevy_main"));

    BEVY_MAIN_WORKER.with(|m| {
        *m.borrow_mut() = Some(bevy_main_worker);
    });
}

/// This will panic if called off the browser's main thread.
pub fn wake_bevy_main() {
    post_message_to_bevy_main(&JsValue::NULL, &[])
}

/// Posts a message to the Bevy main thread.
pub fn post_message_to_bevy_main(
    value: &wasm_bindgen::JsValue,
    transferables: &[&wasm_bindgen::JsValue],
) {
    let transferring: web_sys::js_sys::Array = web_sys::js_sys::Array::new();
    for t in transferables {
        transferring.push(t);
    }
    BEVY_MAIN_WORKER.with(|m| {
        m.borrow()
            .as_ref()
            .unwrap()
            .post_message_with_transfer(value, &transferring)
            .unwrap();
    });
}

/// Runs a task on the browser main thread.
pub fn run_on_browser_main(f: impl FnOnce() + Send + 'static) {
    BROWSER_MAIN_CHANNELS
        .get()
        .expect("Browser main thread not initialized")
        .to_browser_main_tasks_sender
        .send(Box::new(f))
        .unwrap();

    wake_browser_main();
}

/// Runs a function on the browser's main thread and blocks to wait for
/// the return value.
/// Warning: Only call this from off the browser's main thread!
pub fn run_and_block_on_browser_main<Return: Send + 'static>(
    f: impl FnOnce() -> Return + Send + 'static,
) -> Return {
    web_sys::js_sys::eval("console.log('BLOCKING ON BROWSER MAIN')").unwrap();

    let (sender, receiver) = std::sync::mpsc::channel();
    run_on_browser_main(Box::new(move || {
        let result = f();
        sender.send(result).unwrap();
    }));
    let result = receiver.recv().unwrap();

    web_sys::js_sys::eval("console.log('FINISHED BLOCK ON BROWSER MAIN')").unwrap();
    result
}

/// Must be called on the browser's main thread!
/// Runs all tasks sent by other threads to run on the browser's main thread.
pub fn run_browser_main_thread_tasks(receiver: &RefCell<Receiver<Box<dyn FnOnce() + Send>>>) {
    while let Ok(task) = receiver.borrow_mut().try_recv() {
        task();
    }
}

/// Wakes up the browser's main thread to process tasks.
fn wake_browser_main() {
    // Posts a message to a web worker that was spawned
    // by the main thread but is indefinitely blocked.
    // When it receives a message it posts a message to the main thread.
    BROWSER_MAIN_CHANNELS
        .get()
        .unwrap()
        .to_browser_main_waker_sender
        .send(())
        .unwrap();
}

struct WorkerContext(Box<dyn FnOnce()>);

fn spawn_web_worker(
    entry_point: impl FnOnce() + 'static + Send,
    worker_name: Option<&str>,
) -> web_sys::Worker {
    let ptr = Box::leak(Box::new(WorkerContext(Box::new(entry_point)))) as *mut _;

    let init = web_sys::js_sys::Array::new();
    init.push(&wasm_bindgen::module());
    init.push(&wasm_bindgen::memory());
    init.push(&wasm_bindgen::JsValue::from(ptr as u32));

    let mut options = web_sys::WorkerOptions::new();
    options.type_(web_sys::WorkerType::Module);
    if let Some(worker_name) = worker_name {
        options.name(worker_name);
    }

    let worker = web_sys::Worker::new_with_options(&get_worker_script(), &options).unwrap();

    let worker = match worker.post_message(&init) {
        Ok(()) => Ok(worker),
        Err(e) => Err(e),
    }
    .unwrap();

    worker
}

/// Sets this web workers message handler
pub fn set_self_on_message(f: Box<dyn FnMut(web_sys::MessageEvent) + 'static>) {
    ENTRY_POINT.with(|w| {
        *w.borrow_mut() = Some(Box::new(f));
    });
}

thread_local! {
    static ENTRY_POINT: RefCell<Option<Box<dyn FnMut(web_sys::MessageEvent)>>> = RefCell::new(None);
}

#[wasm_bindgen]
/// The WebWorker will call this to call the user provided entry point.
pub fn bevy_web_worker_entry_point(ptr: u32) -> u32 {
    // The initial entry can only be called once but after
    // the worker may assign a new entry point.
    let mut entry_point = ENTRY_POINT
        .with(|w: &RefCell<Option<Box<dyn FnMut(web_sys::MessageEvent)>>>| w.borrow_mut().take());

    if let Some(entry_point) = &mut entry_point {
        let message_event = web_sys::js_sys::eval("self.web_worker_message_event")
            .unwrap()
            .dyn_into::<MessageEvent>()
            .unwrap();

        entry_point(message_event);
    } else if ptr != 0 {
        // SAFETY: Ptr value is always initialized before being sent.
        #[allow(unsafe_code)]
        let p: Box<WorkerContext> = unsafe { Box::from_raw(ptr as *mut _) };
        (p.0)();
    }

    ENTRY_POINT.with(|w| {
        let mut w = w.borrow_mut();
        if w.is_none() {
            *w = entry_point;
        }
    });
    0
}

/// Generates worker entry script as URL encoded blob
pub fn get_worker_script() -> String {
    // This function is adapted from the wasm_thread crate.

    /// Extracts path of the `wasm_bindgen` generated .js shim script.
    ///
    /// Internally, this intentionally generates a javascript exception to obtain a stacktrace containing the current script
    /// URL.
    pub fn get_wasm_bindgen_shim_script_path() -> String {
        web_sys::js_sys::eval(include_str!("js/script_path.js"))
            .unwrap()
            .as_string()
            .unwrap()
    }

    use wasm_bindgen::JsValue;
    use web_sys::js_sys;

    // If wasm bindgen shim url is not provided, try to obtain one automatically
    let wasm_bindgen_shim_url = get_wasm_bindgen_shim_script_path();

    // Generate script from template
    let template = include_str!("js/web_worker_module.js");

    let script = template.replace("WASM_BINDGEN_SHIM_URL", &wasm_bindgen_shim_url);

    // Create url encoded blob
    let arr = js_sys::Array::new();
    arr.set(0, JsValue::from_str(&script));
    let blob = web_sys::Blob::new_with_str_sequence(&arr).unwrap();
    let url = web_sys::Url::create_object_url_with_blob(
        &blob
            .slice_with_f64_and_f64_and_content_type(0.0, blob.size(), "text/javascript")
            .unwrap(),
    )
    .unwrap();

    url
}

/// A wrapper that allows a reference to a non-thread type from the main thread to be safely held.
pub struct BrowserMainType<T: 'static> {
    inner: Arc<BrowserMainTypeInner<T>>,
}

struct BrowserMainTypeInner<T: 'static>(Mutex<Option<SendWrapper<T>>>);

impl<T: 'static> Clone for BrowserMainType<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: 'static> BrowserMainType<T> {
    /// Creates a new [BrowserMainType] instance.
    pub fn new(t: T) -> Self {
        Self {
            inner: Arc::new(BrowserMainTypeInner(Mutex::new(Some(SendWrapper::new(t))))),
        }
    }

    /// Tries to get the inner T
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut Option<SendWrapper<T>>) -> R) -> Option<R> {
        if let Ok(mut inner) = self.inner.0.lock() {
            Some(f(&mut inner))
        } else {
            None
        }
    }

    /// Panics if called off of the browser main thread.
    pub fn take(self) -> Option<T> {
        Some(self.inner.0.lock().unwrap().take().unwrap().take())
    }
}

/// The inner SendWrapper must always be dropped on the browser main thread.
impl<T> Drop for BrowserMainTypeInner<T> {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.0.lock() {
            if let Some(inner) = inner.take() {
                run_on_browser_main(move || {
                    std::mem::drop(inner);
                })
            }
        }
    }
}
