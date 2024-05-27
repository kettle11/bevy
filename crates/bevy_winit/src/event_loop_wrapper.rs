use crate::UserEvent;

use winit::event::Event;
use winit::event_loop::{EventLoop, EventLoopBuilder, EventLoopWindowTarget};

#[cfg(feature = "bevy_dedicated_web_worker")]
mod dedicated_web_worker {
    use super::*;

    use std::cell::RefCell;
    use std::sync::mpsc::{channel, Receiver, Sender};

    use bevy_wasm_threads::{wake_bevy_main, BrowserMainType};
    use wasm_bindgen::JsCast;

    thread_local! {
        /// Called when a winit event occurs or an animation frame occurs.
        static ENTRY_POINT: RefCell<Box<dyn FnMut()>> = RefCell::new(Box::new(|| {}));
    }

    pub struct BevyEventLoopWrapper {
        browser_main_event_loop: BrowserMainType<EventLoop<UserEvent>>,
        to_browser_main_sender:
            Sender<Box<dyn FnOnce(&EventLoopWindowTarget<UserEvent>) + Send + 'static>>,
        to_browser_main_receiver:
            Option<Receiver<Box<dyn FnOnce(&EventLoopWindowTarget<UserEvent>) + Send + 'static>>>,
    }

    impl BevyEventLoopWrapper {
        pub fn new(
            mut event_loop_builder: EventLoopBuilder<UserEvent>,
        ) -> Result<Self, winit::error::EventLoopError> {
            let browser_main_event_loop =
                bevy_wasm_threads::run_and_block_on_browser_main::<
                    Result<
                        BrowserMainType<winit::event_loop::EventLoop<UserEvent>>,
                        winit::error::EventLoopError,
                    >,
                >(move || Ok(BrowserMainType::new(event_loop_builder.build()?)))?;

            let (to_browser_main_sender, to_browser_main_receiver) = channel();

            Ok(Self {
                to_browser_main_sender,
                to_browser_main_receiver: Some(to_browser_main_receiver),
                browser_main_event_loop,
            })
        }

        /// Runs the winit event loop
        /// Returns immediately and `event_handler` is called as new events occur.
        pub fn run<F>(mut self, mut event_handler: F) -> Result<(), winit::error::EventLoopError>
        where
            F: FnMut(Event<UserEvent>, &BevyEventLoopWrapper) + 'static,
        {
            let (to_bevy_main_sender, to_bevy_main_receiver) = channel();

            let to_browser_main_receiver = self.to_browser_main_receiver.take().unwrap();

            let event_loop = self.browser_main_event_loop.clone();

            bevy_wasm_threads::run_on_browser_main(move || {
                let event_loop = event_loop.take().unwrap();
                let _ = event_loop.run(|event, event_loop_window| {
                    // Process all calls from other threads that need to access
                    // the event_loop_window_target
                    while let Ok(f) = to_browser_main_receiver.try_recv() {
                        f(event_loop_window);
                    }

                    // Send an event to the bevy main thread
                    to_bevy_main_sender.send(event).unwrap();

                    // Wake up the Bevy main thread to process the event.
                    wake_bevy_main();
                });

                // TODO: Forward termination result to bevy main
            });

            ENTRY_POINT.with(|entry_point| {
                *entry_point.borrow_mut() = Box::new(move || {
                    // Respond to incoming winit events
                    while let Ok(event) = to_bevy_main_receiver.try_recv() {
                        event_handler(event, &self);
                    }

                    // TODO: Respond to animation frames.
                });

                todo!()
            });

            let callback = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
                ENTRY_POINT.with(|entry_point| {
                    (entry_point.borrow_mut())();
                });
            }) as Box<dyn FnMut()>);

            let self_ = web_sys::js_sys::eval("self")
                .unwrap()
                .dyn_into::<web_sys::DedicatedWorkerGlobalScope>()
                .unwrap();

            // When the browser main thread sends a message wake up this thread.
            self_.set_onmessage(Some(callback.as_ref().unchecked_ref()));
            callback.forget();
            Ok(())
        }

        /// Accesses the inner [EventLoopWindowTarget]
        /// Internally this runs the event on the correct thread.
        pub fn call<T: Send + 'static>(
            &self,
            f: impl FnOnce(&EventLoopWindowTarget<UserEvent>) -> T + Send + 'static,
        ) -> T {
            let browser_main_event_loop = self.browser_main_event_loop.clone();
            let (sender, receiver) = channel();

            let to_browser_main_sender = self.to_browser_main_sender.clone();
            bevy_wasm_threads::run_on_browser_main(move || {
                browser_main_event_loop.with_mut(move |e| {
                    if let Some(event_loop) = e {
                        web_sys::js_sys::eval("console.log('HERE HERE')").unwrap();

                        let result = f(&event_loop);
                        sender.send(result).unwrap();
                    } else {
                        web_sys::js_sys::eval("console.log('HERE HERE 1')").unwrap();

                        to_browser_main_sender
                            .send(Box::new(move |event_loop| {
                                let result = f(event_loop);
                                sender.send(result).unwrap();
                            }))
                            .unwrap();

                        // TODO: The winit event loop needs to be woken here.
                    }
                });
            });

            receiver.recv().unwrap()
        }

        /// Accesses the inner [EventLoopWindowTarget] but does not wait for the return value.
        pub fn call_non_blocking(
            &self,
            f: impl FnOnce(&EventLoopWindowTarget<UserEvent>) + Send + 'static,
        ) {
            let browser_main_event_loop = self.browser_main_event_loop.clone();

            let to_browser_main_sender = self.to_browser_main_sender.clone();
            bevy_wasm_threads::run_on_browser_main(move || {
                browser_main_event_loop.with_mut(move |e| {
                    if let Some(event_loop) = e {
                        f(&event_loop);
                    } else {
                        to_browser_main_sender.send(Box::new(f)).unwrap();
                    }
                });
            });
        }

        pub fn request_redraw(&self, window: &winit::window::Window) {
            todo!()
        }
    }
}
#[cfg(feature = "bevy_dedicated_web_worker")]
pub use dedicated_web_worker::*;

#[cfg(not(feature = "bevy_dedicated_web_worker"))]
mod non_dedicated_web_worker {
    use super::*;

    pub struct BevyEventLoopWrapper {
        event_loop: EventLoop<UserEvent>,
    }

    impl BevyEventLoopWrapper {
        pub fn new(
            mut event_loop_builder: EventLoopBuilder<UserEvent>,
        ) -> Result<Self, winit::error::EventLoopError> {
            todo!()
        }

        /// Runs the winit event loop
        /// Returns immediately and `event_handler` is called as new events occur.
        pub fn run<F>(mut self, mut event_handler: F) -> Result<(), EventLoopError>
        where
            F: FnMut(Event<UserEvent>, &BevyEventLoopWrapper) + 'static,
        {
            todo!()
        }

        /// Accesses the inner [EventLoopWindowTarget]
        /// On Wasm with this runs the event on the browser's main thread and returns
        /// the value to the Bevy main thread.
        pub fn call<T: Send + 'static>(
            &self,
            f: impl FnOnce(&EventLoopWindowTarget<UserEvent>) -> T + Send + 'static,
        ) -> T {
            todo!()
        }

        /// Accesses the inner [EventLoopWindowTarget]
        /// On Wasm
        pub fn call_non_blocking(
            &self,
            f: impl FnOnce(&EventLoopWindowTarget<UserEvent>) + Send + 'static,
        ) {
            todo!()
        }

        pub fn request_redraw(&self, window: &winit::window::Window) {
            todo!()
        }
    }
}

#[cfg(not(feature = "bevy_dedicated_web_worker"))]
pub use non_dedicated_web_worker::*;

impl BevyEventLoopWrapper {
    /// Calls [winit::event_loop::EventLoop::create_proxy]
    pub fn create_proxy(&self) -> winit::event_loop::EventLoopProxy<UserEvent> {
        todo!()
    }

    /// Calls [winit::event_loop::EventLoop::exit]
    pub fn exit(&self) {
        self.call(|e| e.exit());
    }

    /// Calls [winit::event_loop::EventLoop::set_control_flow]
    pub fn set_control_flow(&self, control_flow: winit::event_loop::ControlFlow) {
        self.call_non_blocking(move |e| e.set_control_flow(control_flow));
    }

    /// Calls [winit::event_loop::EventLoop::primary_monitor]
    pub fn primary_monitor(&self) -> Option<winit::monitor::MonitorHandle> {
        self.call(|e| e.primary_monitor())
    }

    /// Calls [winit::event_loop::EventLoop::available_monitors]
    pub fn available_monitors(&self) -> impl Iterator<Item = winit::monitor::MonitorHandle> {
        let result: Vec<_> = self.call(|e| e.available_monitors().collect());
        result.into_iter()
    }

    /// Calls [winit::window::WindowBuilder::build]
    pub fn build_window(
        &self,
        winit_window_builder: winit::window::WindowBuilder,
    ) -> Result<winit::window::Window, winit::error::OsError> {
        // TODO: May need to do some canvas shenanigans here.
        self.call(move |e| winit_window_builder.build(e))
    }
}
