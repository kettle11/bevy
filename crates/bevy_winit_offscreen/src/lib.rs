//! This crate wraps Winit to more easily allow a browser OffScreenCanvas to be used.

use std::cell::RefCell;
use winit::error::EventLoopError;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::{self, ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::platform::web::EventLoopExtWebSys;
use winit::window::{CustomCursorSource, WindowAttributes, WindowId};

#[cfg(target_arch = "wasm32")]
use std::sync::mpsc::{channel, Receiver, Sender};

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// Called when a winit event occurs or an animation frame occurs.
    static ENTRY_POINT: RefCell<Box<dyn FnMut()>> = RefCell::new(Box::new(|| {}));
}

#[cfg(target_arch = "wasm32")]
struct EventLoopForwarder<T> {
    to_browser_main_receiver: Receiver<Box<dyn FnOnce(&ActiveEventLoop) + Send + 'static>>,
    to_bevy_main_sender: Sender<WinitEvent<T>>,
}

#[cfg(target_arch = "wasm32")]
impl<T> EventLoopForwarder<T> {
    fn send_event(&mut self, event_loop: &event_loop::ActiveEventLoop, event: WinitEvent<T>) {
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

pub struct WrappedEventLoop<T: 'static> {
    #[cfg(target_arch = "wasm32")]
    browser_main_event_loop: bevy_wasm_threads::BrowserMainType<EventLoop<T>>,

    #[cfg(not(target_arch = "wasm32"))]
    event_loop: winit::event_loop::EventLoop<T>,
}

impl<T: Send> WrappedEventLoop<T> {
    pub fn build(
        mut event_loop_builder: winit::event_loop::EventLoopBuilder<T>,
    ) -> Result<WrappedEventLoop<T>, EventLoopError> {
        #[cfg(target_arch = "wasm32")]
        {
            let browser_main_event_loop = bevy_wasm_threads::run_and_block_on_browser_main::<
                Result<
                    bevy_wasm_threads::BrowserMainType<winit::event_loop::EventLoop<T>>,
                    winit::error::EventLoopError,
                >,
            >(move || {
                Ok(bevy_wasm_threads::BrowserMainType::new(
                    event_loop_builder.build()?,
                ))
            })?;

            Ok(Self {
                browser_main_event_loop,
            })
        }

        #[cfg(not(target_arch = "wasm32"))]
        event_loop_builder.build()
    }

    /// Corresponds to [winit::event_loop::EventLoop::create_proxy]
    pub fn create_proxy(&self) -> EventLoopProxy<T> {
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
                web_sys::js_sys::eval("console.log('RUNNING EVENT LOOP')").unwrap();

                let event_loop = self.browser_main_event_loop.take().unwrap();

                let event_loop_forwarder = EventLoopForwarder {
                    to_bevy_main_sender,
                    to_browser_main_receiver,
                };

                event_loop.spawn_app(event_loop_forwarder);
            });

            let mut event_loop = WrappedActiveEventLoop {
                to_browser_main_sender,
            };

            ENTRY_POINT.with(|entry_point| {
                *entry_point.borrow_mut() = Box::new(move || {
                    // Respond to incoming winit events
                    while let Ok(event) = to_bevy_main_receiver.try_recv() {
                        match event {
                            WinitEvent::NewEvents { cause } => {
                                app.new_events(&mut event_loop, cause)
                            }
                            WinitEvent::Resumed => app.resumed(&mut event_loop),
                            WinitEvent::UserEvent { event } => {
                                app.user_event(&mut event_loop, event)
                            }
                            WinitEvent::WindowEvent { window_id, event } => {
                                app.window_event(&mut event_loop, window_id, event)
                            }
                            WinitEvent::DeviceEvent { device_id, event } => {
                                app.device_event(&mut event_loop, device_id, event)
                            }
                            WinitEvent::AboutToWait => app.about_to_wait(&mut event_loop),
                            WinitEvent::Suspended => app.suspended(&mut event_loop),
                            WinitEvent::Exiting => app.exiting(&mut event_loop),
                            WinitEvent::MemoryWarning => app.memory_warning(&mut event_loop),
                        }
                    }

                    // TODO: Respond to animation frames.
                });
            });

            // Throw an error that imitate's Winit's control flow hack.
            // This unwinds the stack and prevents a return.
            wasm_bindgen::throw_str(
                "Using exceptions for control flow, don't mind me. This isn't actually an error!",
            );
        }

        #[cfg(not(target_arch = "wasm32"))]
        self.event_loop.run_app(&mut app)
    }
}

pub struct WrappedActiveEventLoop {
    to_browser_main_sender: Sender<Box<dyn FnOnce(&ActiveEventLoop) + Send + 'static>>,
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
        receiver.recv().unwrap()
    }

    fn call_non_blocking(&self, f: impl FnOnce(&ActiveEventLoop) + Send + 'static) {
        self.to_browser_main_sender
            .send(Box::new(move |event_loop| {
                f(event_loop);
            }))
            .unwrap();
    }

    /// Corresponds to [winit::event_loop::ActiveEventLoop::create_window]
    pub fn create_window(
        &self,
        window_attributes: WindowAttributes,
    ) -> Result<winit::window::Window, winit::error::OsError> {
        self.call(move |e| e.create_window(window_attributes))
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
