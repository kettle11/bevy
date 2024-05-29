use winit::error::EventLoopError;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::window::WindowId;

pub enum WinitEvent<T> {
    NewEvents {
        cause: StartCause,
    },
    Resumed,
    UserEvent {
        event: T,
    },
    WindowEvent {
        window_id: WindowId,
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

pub struct OffscreenEventLoop<T> {}

pub trait WinitOffscreenCanvasBuilder<T> {
    fn build_offscreen(&mut self) -> Result<OffscreenEventLoop<T>, EventLoopError>;
}

impl<T> OffscreenEventLoop<T> {
    pub fn run_app<A: OffscreenApplicationHandler<T>>(
        self,
        app: &mut A,
    ) -> Result<(), EventLoopError> {
        todo!()
    }
}

trait OffscreenApplicationHandler<T> {
    fn new_events(&mut self, event_loop: &OffscreenEventLoop<T>, cause: StartCause) {
        let _ = (event_loop, cause);
    }

    fn resumed(&mut self, event_loop: &OffscreenEventLoop<T>);

    fn user_event(&mut self, event_loop: &OffscreenEventLoop<T>, event: T) {
        let _ = (event_loop, event);
    }

    fn window_event(
        &mut self,
        event_loop: &OffscreenEventLoop<T>,
        window_id: WindowId,
        event: WindowEvent,
    );

    fn device_event(
        &mut self,
        event_loop: &OffscreenEventLoop<T>,
        device_id: DeviceId,
        event: DeviceEvent,
    ) {
        let _ = (event_loop, device_id, event);
    }

    fn about_to_wait(&mut self, event_loop: &OffscreenEventLoop<T>) {
        let _ = event_loop;
    }

    fn suspended(&mut self, event_loop: &OffscreenEventLoop<T>) {
        let _ = event_loop;
    }

    fn exiting(&mut self, event_loop: &OffscreenEventLoop<T>) {
        let _ = event_loop;
    }

    fn memory_warning(&mut self, event_loop: &OffscreenEventLoop<T>) {
        let _ = event_loop;
    }
}
