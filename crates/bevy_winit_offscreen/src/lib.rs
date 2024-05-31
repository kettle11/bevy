//! This crate wraps Winit to more easily allow a browser OffScreenCanvas to be used.

use bevy_utils::hashbrown::{HashMap, HashSet};
use std::cell::{OnceCell, RefCell};
use wasm_bindgen::JsValue;
use web_sys::Window;
use winit::error::EventLoopError;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::{self, ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::platform::web::{self, EventLoopExtWebSys};
use winit::window::{CustomCursorSource, WindowAttributes, WindowId};

pub use bevy_wasm_threads::{run_and_block_on_browser_main, run_on_browser_main};

#[cfg(target_arch = "wasm32")]
use std::sync::mpsc::{channel, Receiver, Sender};

enum EntryReason {
    AnimationFrame { window_id: winit::window::WindowId },
    WorkerMessage,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// Called when a winit event occurs or an animation frame occurs.
    static ENTRY_POINT: RefCell<Box<dyn FnMut(EntryReason)>> = RefCell::new(Box::new(|_| {}));
    /// Stores OffscreenCanvases that have been sent to this worker.
    static OFFSCREEN_CANVASES: RefCell<HashMap<String, web_sys::OffscreenCanvas>> = RefCell::new(HashMap::new());
}

struct EventLoopWaker {
    wake_fn: Box<dyn Fn()>,
}

impl EventLoopWaker {
    pub fn new<T: 'static>(event_loop_proxy: EventLoopProxy<UserEvent<T>>) -> Self {
        Self {
            wake_fn: Box::new(move || {
                let _ = event_loop_proxy.send_event(UserEvent::WakeUp);
            }),
        }
    }

    pub fn wake(&self) {
        (self.wake_fn)()
    }
}

#[cfg(target_arch = "wasm32")]
pub struct WinitOffscreen {}

#[cfg(target_arch = "wasm32")]
impl WinitOffscreen {
    pub fn init(canvas_selector: &str, f: impl FnOnce() + Send + 'static) -> Self {
        bevy_wasm_threads::run_bevy_main(move || {
            let mut f = Some(f);

            // Wait for the first transferred canvas before running setup.
            bevy_wasm_threads::set_self_on_message(Box::new(move |event| {
                web_sys::js_sys::eval("console.log('RECEIVED CANVAS')").unwrap();

                check_for_transferred_canvas(&event);
                (f.take().unwrap())();
            }));
        });
        let s = Self {};
        s.transfer_canvas_to_offscreen(canvas_selector);
        s
    }

    /// Sends the canvas specified to the OffScreen thread.
    /// This must be called on any canvas that will be used by Bevy
    /// *before* Bevy itself is initialized and run.
    pub fn transfer_canvas_to_offscreen(&self, selector: &str) {
        use wasm_bindgen::JsCast;

        let window: web_sys::Window = web_sys::window().unwrap();
        let document: web_sys::Document = window.document().unwrap();

        let canvas = document
            .query_selector(selector)
            .expect("Cannot query for canvas element.");
        if let Some(canvas) = canvas {
            let canvas = canvas
                .dyn_into::<web_sys::HtmlCanvasElement>()
                .expect("Expected a canvas selector");

            let offscreen_canvas: web_sys::OffscreenCanvas =
                canvas.transfer_control_to_offscreen().unwrap();

            let message = web_sys::js_sys::Object::new();
            web_sys::js_sys::Reflect::set(&message, &"offscreen_canvas".into(), &offscreen_canvas)
                .unwrap();

            web_sys::js_sys::Reflect::set(
                &message,
                &"offscreen_canvas_name".into(),
                &selector.into(),
            )
            .unwrap();

            bevy_wasm_threads::post_message_to_bevy_main(&message, &[&offscreen_canvas]);
        } else {
            panic!("Cannot find canvas to transfer: {}.", selector);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn check_for_transferred_canvas(event: &web_sys::MessageEvent) {
    use wasm_bindgen::JsCast;

    let data = event.data();
    if let Ok(value) = web_sys::js_sys::Reflect::get(&data, &"offscreen_canvas".into()) {
        let offscreen_canvas = value
            .dyn_into::<web_sys::OffscreenCanvas>()
            .expect("Expected an OffscreenCanvas message");

        let name = web_sys::js_sys::Reflect::get(&data, &"offscreen_canvas_name".into()).unwrap();
        let name = name.as_string().unwrap();
        OFFSCREEN_CANVASES.with(|w| w.borrow_mut().insert(name, offscreen_canvas));
    }
}

#[cfg(target_arch = "wasm32")]
/// Gets an arbitrary OffscreenCanvas if one has been sent
/// with [transfer_canvas_to_offscreen]
pub fn get_offscreen_canvas() -> web_sys::OffscreenCanvas {
    OFFSCREEN_CANVASES.with(|w| {
        w.borrow_mut()
            .values()
            .next()
            .expect("No offscreen canvas available. Use `transfer_canvas_to_offscreen` first.")
            .clone()
    })
}

#[cfg(target_arch = "wasm32")]
struct EventLoopForwarder<T> {
    to_browser_main_receiver: Receiver<Box<dyn FnOnce(&ActiveEventLoop) + Send + 'static>>,
    to_bevy_main_sender: Sender<WinitEvent<T>>,
}

#[cfg(target_arch = "wasm32")]
impl<T> EventLoopForwarder<T> {
    fn send_event(&mut self, event_loop: &event_loop::ActiveEventLoop, event: WinitEvent<T>) {
        // Make sure other events sent to the main thread are handled.
        bevy_wasm_threads::handle_browser_main_tasks();

        // Process incoming events that need the ActiveEventLoop.
        while let Ok(action) = self.to_browser_main_receiver.try_recv() {
            action(event_loop);
        }

        self.to_bevy_main_sender.send(event).unwrap();
        bevy_wasm_threads::wake_bevy_main();
    }
}

#[cfg(target_arch = "wasm32")]
impl<T: 'static> winit::application::ApplicationHandler<T> for EventLoopForwarder<T> {
    fn resumed(&mut self, event_loop: &event_loop::ActiveEventLoop) {
        self.send_event(event_loop, WinitEvent::Resumed);
    }

    fn window_event(
        &mut self,
        event_loop: &event_loop::ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.send_event(event_loop, WinitEvent::WindowEvent { window_id, event });
    }

    fn new_events(&mut self, event_loop: &event_loop::ActiveEventLoop, cause: StartCause) {
        self.send_event(event_loop, WinitEvent::NewEvents { cause });
    }

    fn user_event(&mut self, event_loop: &event_loop::ActiveEventLoop, event: T) {
        self.send_event(event_loop, WinitEvent::UserEvent { event });
    }

    fn device_event(
        &mut self,
        event_loop: &event_loop::ActiveEventLoop,
        device_id: DeviceId,
        event: DeviceEvent,
    ) {
        self.send_event(event_loop, WinitEvent::DeviceEvent { device_id, event });
    }

    fn about_to_wait(&mut self, event_loop: &event_loop::ActiveEventLoop) {
        self.send_event(event_loop, WinitEvent::AboutToWait);
    }

    fn suspended(&mut self, event_loop: &event_loop::ActiveEventLoop) {
        self.send_event(event_loop, WinitEvent::Suspended);
    }

    fn exiting(&mut self, event_loop: &event_loop::ActiveEventLoop) {
        self.send_event(event_loop, WinitEvent::Exiting);
    }

    fn memory_warning(&mut self, event_loop: &event_loop::ActiveEventLoop) {
        self.send_event(event_loop, WinitEvent::MemoryWarning);
    }
}

enum WinitEvent<T> {
    NewEvents {
        cause: StartCause,
    },
    Resumed,
    UserEvent {
        event: T,
    },
    WindowEvent {
        window_id: WindowId,
        event: WindowEvent,
    },
    DeviceEvent {
        device_id: DeviceId,
        event: DeviceEvent,
    },
    AboutToWait,
    Suspended,
    Exiting,
    MemoryWarning,
}

/// A custom winit event to be sent to the event loop.
pub enum UserEvent<T> {
    /// Wakes up the event loop
    WakeUp,
    /// A custom user event
    Custom(T),
}

/// Corresponds to [winit::event_loop::EventLoop]
/// but `WrappedEventLoop` can be used from off the browser's main thread
pub struct WrappedEventLoop<T: 'static> {
    #[cfg(target_arch = "wasm32")]
    browser_main_event_loop: bevy_wasm_threads::BrowserMainType<EventLoop<UserEvent<T>>>,

    #[cfg(not(target_arch = "wasm32"))]
    event_loop: winit::event_loop::EventLoop<T>,
    event_loop_waker: EventLoopWaker,
}

impl<T: Send> WrappedEventLoop<T> {
    /// Builds the `WrappedEventLoop`
    pub fn build(
        mut event_loop_builder: winit::event_loop::EventLoopBuilder<UserEvent<T>>,
    ) -> Result<WrappedEventLoop<T>, EventLoopError> {
        #[cfg(target_arch = "wasm32")]
        {
            let (browser_main_event_loop, event_loop_proxy) =
                bevy_wasm_threads::run_and_block_on_browser_main::<
                    Result<
                        (
                            bevy_wasm_threads::BrowserMainType<
                                winit::event_loop::EventLoop<UserEvent<T>>,
                            >,
                            winit::event_loop::EventLoopProxy<UserEvent<T>>,
                        ),
                        winit::error::EventLoopError,
                    >,
                >(move || {
                    let event_loop = event_loop_builder.build()?;

                    let event_loop_proxy = event_loop.create_proxy();
                    Ok((
                        bevy_wasm_threads::BrowserMainType::new(event_loop),
                        event_loop_proxy,
                    ))
                })?;

            Ok(Self {
                browser_main_event_loop,
                event_loop_waker: EventLoopWaker::new(event_loop_proxy),
            })
        }

        #[cfg(not(target_arch = "wasm32"))]
        event_loop_builder.build()
    }

    /// Corresponds to [winit::event_loop::EventLoop::create_proxy]
    pub fn create_proxy(&self) -> EventLoopProxy<UserEvent<T>> {
        let browser_main_event_loop = self.browser_main_event_loop.clone();

        bevy_wasm_threads::run_and_block_on_browser_main(move || {
            browser_main_event_loop.with_mut(|w| w.as_ref().unwrap().create_proxy())
        })
        .unwrap()
    }

    /// Calls either [winit::event_loop::EventLoop::spawn_app] for OffscreenCanvas use or
    /// or [winit::event_loop::EventLoop::run_app] otherwise
    pub fn spawn_or_run_app<A: WrappedApplicationHandler<T> + 'static>(
        self,
        mut app: A,
    ) -> Result<(), EventLoopError> {
        #[cfg(target_arch = "wasm32")]
        {
            let (to_bevy_main_sender, to_bevy_main_receiver) = channel();

            let (to_browser_main_sender, to_browser_main_receiver) = channel();

            bevy_wasm_threads::run_on_browser_main(move || {
                let event_loop = self.browser_main_event_loop.take().unwrap();

                let event_loop_forwarder = EventLoopForwarder {
                    to_bevy_main_sender,
                    to_browser_main_receiver,
                };

                event_loop.spawn_app(event_loop_forwarder);
            });

            let mut event_loop = WrappedActiveEventLoop {
                event_loop_waker: self.event_loop_waker,
                to_browser_main_sender,
                redrawing: RefCell::new(HashSet::new()),
            };

            // Whenever the main thread winit is woken up it's
            // sent a WakeUp, which also causes an AboutToWait to be sent.
            // This can cause a back and forth, and many spurious wakeups,
            // so skim ones that follow a "WakeUp" call.
            let mut skip_next_about_to_wait = 0;

            ENTRY_POINT.with(|entry_point| {
                *entry_point.borrow_mut() = Box::new(move |entry_reason| {
                    web_sys::js_sys::eval("console.log('$RESPONDING TO BEVY MAIN EVENTS')")
                        .unwrap();

                    // Respond to incoming winit events
                    while let Ok(event) = to_bevy_main_receiver.try_recv() {
                        match event {
                            WinitEvent::NewEvents { cause } => {
                                app.new_events(&mut event_loop, cause)
                            }
                            WinitEvent::Resumed => app.resumed(&mut event_loop),
                            WinitEvent::UserEvent { event } => match event {
                                UserEvent::WakeUp => {
                                    skip_next_about_to_wait += 1;
                                }
                                UserEvent::Custom(t) => app.user_event(&mut event_loop, t),
                            },
                            WinitEvent::WindowEvent { window_id, event } => {
                                app.window_event(&mut event_loop, window_id, event)
                            }
                            WinitEvent::DeviceEvent { device_id, event } => {
                                app.device_event(&mut event_loop, device_id, event)
                            }
                            WinitEvent::AboutToWait => {
                                if skip_next_about_to_wait > 0 {
                                    skip_next_about_to_wait -= 1;
                                    continue;
                                }
                                app.about_to_wait(&mut event_loop)
                            }
                            WinitEvent::Suspended => app.suspended(&mut event_loop),
                            WinitEvent::Exiting => app.exiting(&mut event_loop),
                            WinitEvent::MemoryWarning => app.memory_warning(&mut event_loop),
                        }
                    }
                    web_sys::js_sys::eval("console.log('$DONE WITH BEVY MAIN EVENTS')").unwrap();

                    match entry_reason {
                        EntryReason::AnimationFrame { window_id } => {
                            web_sys::js_sys::eval("console.log('ANIMATION FRAME')").unwrap();

                            event_loop.redrawing.borrow_mut().remove(&window_id);

                            // Insert a synthetic redraw event.
                            app.window_event(
                                &mut event_loop,
                                window_id,
                                WindowEvent::RedrawRequested,
                            );
                            // Imitate normal winit events.
                            app.about_to_wait(&mut event_loop);
                        }
                        _ => {}
                    }
                });
            });

            // When this worker thread receives a message call the entry point.
            bevy_wasm_threads::set_self_on_message(Box::new(move |event| {
                web_sys::js_sys::eval("console.log('RECEIVED MESSAGE')").unwrap();

                check_for_transferred_canvas(&event);

                ENTRY_POINT.with(|entry_point| {
                    (entry_point.borrow_mut())(EntryReason::WorkerMessage);
                });
                web_sys::js_sys::eval("console.log('RETURNING TO TOP')").unwrap();
            }));

            Ok(())
            // // Throw an error that imitate's Winit's control flow hack.
            // // This prevents a return.
            // wasm_bindgen::throw_str(
            //     "Using exceptions for control flow, don't mind me. This isn't actually an error!",
            // );
        }

        #[cfg(not(target_arch = "wasm32"))]
        self.event_loop.run_app(&mut app)
    }
}

/// Wraps [winit::event_loop::ActiveEventLoop] so that it can be used from off the browser's main thread on web.
pub struct WrappedActiveEventLoop {
    to_browser_main_sender: Sender<Box<dyn FnOnce(&ActiveEventLoop) + Send + 'static>>,
    event_loop_waker: EventLoopWaker,
    redrawing: RefCell<HashSet<WindowId>>,
}

impl WrappedActiveEventLoop {
    fn call<T: Send + 'static>(&self, f: impl FnOnce(&ActiveEventLoop) -> T + Send + 'static) -> T {
        let (sender, receiver) = channel();
        self.to_browser_main_sender
            .send(Box::new(move |event_loop| {
                let v = f(event_loop);
                sender.send(v).unwrap();
            }))
            .unwrap();

        self.event_loop_waker.wake();

        web_sys::js_sys::eval("console.log('WAITING ON MAIN HERE1')").unwrap();
        let result = receiver.recv().unwrap();
        web_sys::js_sys::eval("console.log('DONE WAITING ON MAIN HERE1')").unwrap();
        result
    }

    fn call_non_blocking(&self, f: impl FnOnce(&ActiveEventLoop) + Send + 'static) {
        self.to_browser_main_sender
            .send(Box::new(move |event_loop| {
                f(event_loop);
            }))
            .unwrap();

        self.event_loop_waker.wake();

        // TODO: Wake up browser main here.
    }

    /// Corresponds to [winit::event_loop::ActiveEventLoop::create_window]
    pub fn create_window(
        &self,
        mut window_attributes: WindowAttributes,
        canvas_selector: Option<String>,
    ) -> Result<winit::window::Window, winit::error::OsError> {
        use wasm_bindgen::JsCast;
        use winit::platform::web::WindowAttributesExtWebSys;

        self.call(move |e| {
            if let Some(selector) = &canvas_selector {
                let window = web_sys::window().unwrap();
                let document = window.document().unwrap();
                let canvas = document
                    .query_selector(selector)
                    .expect("Cannot query for canvas element.");
                if let Some(canvas) = canvas {
                    let canvas = canvas.dyn_into::<web_sys::HtmlCanvasElement>().ok();
                    window_attributes = window_attributes.with_canvas(canvas);
                } else {
                    panic!("Cannot find element: {}.", selector);
                }
            }

            e.create_window(window_attributes)
        })
    }

    /// Corresponds to [winit::event_loop::ActiveEventLoop::create_custom_cursor]
    pub fn create_custom_cursor(
        &self,
        custom_cursor: CustomCursorSource,
    ) -> winit::window::CustomCursor {
        self.call(move |e: &ActiveEventLoop| e.create_custom_cursor(custom_cursor))
    }

    /// Corresponds to [winit::event_loop::ActiveEventLoop::available_monitors]
    pub fn available_monitors(&self) -> impl Iterator<Item = winit::monitor::MonitorHandle> {
        let available_monitors: Vec<_> = self.call(move |e| e.available_monitors().collect());
        available_monitors.into_iter()
    }

    /// Corresponds to [winit::event_loop::ActiveEventLoop::primary_monitor]
    pub fn primary_monitor(&self) -> Option<winit::monitor::MonitorHandle> {
        self.call(move |e: &ActiveEventLoop| e.primary_monitor())
    }

    /// Corresponds to [winit::event_loop::ActiveEventLoop::listen_device_events]
    pub fn listen_device_events(&self, allowed: winit::event_loop::DeviceEvents) {
        self.call_non_blocking(move |e: &ActiveEventLoop| e.listen_device_events(allowed))
    }

    /// Corresponds to [winit::event_loop::ActiveEventLoop::set_control_flow]
    pub fn set_control_flow(&self, control_flow: winit::event_loop::ControlFlow) {
        self.call_non_blocking(move |e: &ActiveEventLoop| e.set_control_flow(control_flow));
    }

    /// Corresponds to [winit::event_loop::ActiveEventLoop::control_flow]
    pub fn control_flow(&self) -> winit::event_loop::ControlFlow {
        self.call(move |e: &ActiveEventLoop| e.control_flow())
    }

    /// Corresponds to [winit::event_loop::ActiveEventLoop::exit]
    pub fn exit(&self) {
        self.call_non_blocking(move |e: &ActiveEventLoop| e.exit());
    }

    /// Corresponds to [winit::event_loop::ActiveEventLoop::exiting]
    pub fn exiting(&self) -> bool {
        self.call(move |e: &ActiveEventLoop| e.exiting())
    }

    /// Corresponds to [winit::event_loop::ActiveEventLoop::owned_display_handle]
    pub fn owned_display_handle(&self) -> winit::event_loop::OwnedDisplayHandle {
        self.call(move |e: &ActiveEventLoop| e.owned_display_handle())
    }

    /// Corresponds to [winit::window::Window::request_redraw]
    /// On web with `offscreen_canvas` enabled this will correctly request
    /// the animation frame on the local thread.
    pub fn request_redraw(&self, window: &winit::window::Window) {
        use wasm_bindgen::JsCast;

        thread_local! {
            static ANIMATION_FRAME_CLOSURE: OnceCell<wasm_bindgen::closure::Closure<dyn Fn()>> = OnceCell::new();
        }

        web_sys::js_sys::eval("console.log('REQUESTING REDRAW0')").unwrap();
        let window_id: WindowId = window.id();

        // Deduplicate multiple redraw requests.
        let to_redraw = self.redrawing.borrow_mut().insert(window_id);

        if to_redraw {
            ANIMATION_FRAME_CLOSURE.with(|w| {
                let closure = w.get_or_init(|| {
                    wasm_bindgen::closure::Closure::new(move || {
                        web_sys::js_sys::eval("console.log('REQUEST ANIMATION FRAME CALLED')")
                            .unwrap();

                        ENTRY_POINT.with(|entry_point| {
                            (entry_point.borrow_mut().as_mut())(EntryReason::AnimationFrame {
                                window_id,
                            });
                        });
                    })
                });

                web_sys::js_sys::eval("console.log('REQUESTING REDRAW1')").unwrap();

                web_sys::js_sys::eval("self")
                    .unwrap()
                    .dyn_into::<web_sys::DedicatedWorkerGlobalScope>()
                    .unwrap()
                    .request_animation_frame(closure.as_ref().unchecked_ref())
                    .unwrap();
            });
        } else {
            web_sys::js_sys::eval("console.log('DEDUPED ANIMATION FRAME')").unwrap();
        }
    }
}

/// Corresponds to [winit::application::ApplicationHandler]
/// This trait implements the same interface as `ApplicationHandler`
/// but when running in the browser defers actions to the browser's main thread.
pub trait WrappedApplicationHandler<T> {
    /// Corresponds to [winit::application::ApplicationHandler::new_events]
    fn new_events(&mut self, event_loop: &WrappedActiveEventLoop, cause: StartCause) {
        let _ = (event_loop, cause);
    }

    /// Corresponds to [winit::application::ApplicationHandler::resumed]
    fn resumed(&mut self, event_loop: &WrappedActiveEventLoop);

    /// Corresponds to [winit::application::ApplicationHandler::user_event]
    fn user_event(&mut self, event_loop: &WrappedActiveEventLoop, event: T) {
        let _ = (event_loop, event);
    }

    /// Corresponds to [winit::application::ApplicationHandler::window_event]
    fn window_event(
        &mut self,
        event_loop: &WrappedActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    );

    /// Corresponds to [winit::application::ApplicationHandler::device_event]
    fn device_event(
        &mut self,
        event_loop: &WrappedActiveEventLoop,
        device_id: DeviceId,
        event: DeviceEvent,
    ) {
        let _ = (event_loop, device_id, event);
    }

    /// Corresponds to [winit::application::ApplicationHandler::about_to_wait]
    fn about_to_wait(&mut self, event_loop: &WrappedActiveEventLoop) {
        let _ = event_loop;
    }

    /// Corresponds to [winit::application::ApplicationHandler::suspended]
    fn suspended(&mut self, event_loop: &WrappedActiveEventLoop) {
        let _ = event_loop;
    }

    /// Corresponds to [winit::application::ApplicationHandler::exiting]
    fn exiting(&mut self, event_loop: &WrappedActiveEventLoop) {
        let _ = event_loop;
    }

    /// Corresponds to [winit::application::ApplicationHandler::memory_warning]
    fn memory_warning(&mut self, event_loop: &WrappedActiveEventLoop) {
        let _ = event_loop;
    }
}
