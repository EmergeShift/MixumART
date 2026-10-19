//! MixumART B0: a minimal headless Wayland compositor.
//!
//! Clients connect to the `mixum-art-0` socket. The compositor does not render
//! to an output; on every toplevel commit it reads the `wl_shm` buffer, computes
//! a damage bounding box against the previous frame, PNG-encodes it, and sends
//! it to the host viewer over TCP. This is the B-route counterpart of mixcomp's
//! A-route capture stream.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("mixum-env-compositor is Linux-only (run it inside the mixumOS container)");
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::run() {
        eprintln!("mixum-env-compositor: {error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        collections::HashMap,
        io::Write,
        net::{TcpListener, TcpStream},
        sync::{
            mpsc::{self, Receiver, Sender},
            Arc, Mutex,
        },
        time::{Duration, Instant},
    };

    use mixum_env_surface::{
        encode_message, read_message, Frame, InputEvent, SurfaceDestroy, SurfaceInfo, FORMAT_PNG,
        MSG_FRAME, MSG_INPUT, MSG_SURFACE_CREATE, MSG_SURFACE_DESTROY,
    };
    use smithay::backend::renderer::utils::{
        on_commit_buffer_handler, RendererSurfaceStateUserData,
    };
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
    use smithay::reexports::wayland_server::Resource;
    use smithay::{
        backend::input::{ButtonState, KeyState},
        input::{
            keyboard::{FilterResult, Keycode},
            pointer::{ButtonEvent, MotionEvent},
            Seat, SeatHandler, SeatState,
        },
        output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
        reexports::wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_buffer, wl_shm, wl_surface::WlSurface},
            Client, Display, DisplayHandle, ListeningSocket,
        },
        utils::{Point, Serial, Size, Transform},
        wayland::{
            buffer::BufferHandler,
            compositor::{
                CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes,
            },
            output::OutputHandler,
            selection::{
                data_device::{
                    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
                },
                SelectionHandler,
            },
            shell::xdg::{
                PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
                XdgToplevelSurfaceData,
            },
            shm::{with_buffer_contents, ShmHandler, ShmState},
        },
    };

    const WINDOW_WIDTH: i32 = 1000;
    const WINDOW_HEIGHT: i32 = 700;
    const TCP_PORT: u16 = 7603;

    #[derive(Default)]
    struct ClientState {
        compositor_state: CompositorClientState,
    }

    impl ClientData for ClientState {
        fn initialized(&self, _client_id: ClientId) {}
        fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
    }

    struct SurfaceEntry {
        /// Kept alive so xdg configure/ping handling keeps working.
        _toplevel: ToplevelSurface,
        wl_surface: WlSurface,
        title: String,
        app_id: String,
        announced: bool,
        width: usize,
        height: usize,
        prev: Vec<u8>,
        have_prev: bool,
    }

    struct Art {
        start: Instant,
        compositor_state: CompositorState,
        xdg_shell_state: XdgShellState,
        shm_state: ShmState,
        data_device_state: DataDeviceState,
        seat_state: SeatState<Self>,
        seat: Seat<Self>,
        _output: Output,
        surfaces: HashMap<u32, SurfaceEntry>,
        next_id: u32,
        clients: Arc<Mutex<Vec<TcpStream>>>,
        input_rx: Receiver<InputEvent>,
        serial: u32,
    }

    impl Art {
        fn new(
            display_handle: &DisplayHandle,
            clients: Arc<Mutex<Vec<TcpStream>>>,
            input_rx: Receiver<InputEvent>,
        ) -> Self {
            let compositor_state = CompositorState::new::<Self>(display_handle);
            let xdg_shell_state = XdgShellState::new::<Self>(display_handle);
            let shm_state = ShmState::new::<Self>(display_handle, vec![]);
            let data_device_state = DataDeviceState::new::<Self>(display_handle);

            let mut seat_state = SeatState::new();
            let mut seat = seat_state.new_wl_seat(display_handle, "mixum-art");
            seat.add_keyboard(Default::default(), 200, 25)
                .expect("keyboard");
            seat.add_pointer();

            let output = Output::new(
                "mixum-art".into(),
                PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: Subpixel::Unknown,
                    make: "Mixum".into(),
                    model: "Art".into(),
                },
            );
            output.create_global::<Self>(display_handle);
            let mode = Mode {
                size: (WINDOW_WIDTH, WINDOW_HEIGHT).into(),
                refresh: 60_000,
            };
            output.change_current_state(
                Some(mode),
                Some(Transform::Normal),
                Some(Scale::Integer(1)),
                Some((0, 0).into()),
            );
            output.set_preferred(mode);

            Self {
                start: Instant::now(),
                compositor_state,
                xdg_shell_state,
                shm_state,
                data_device_state,
                seat_state,
                seat,
                _output: output,
                surfaces: HashMap::new(),
                next_id: 1,
                clients,
                input_rx,
                serial: 0,
            }
        }

        fn entry_for(&self, surface: &WlSurface) -> Option<u32> {
            self.surfaces
                .iter()
                .find(|(_, entry)| entry.wl_surface.id() == surface.id())
                .map(|(id, _)| *id)
        }

        fn announce(&self, id: u32, width: usize, height: usize, title: &str, app_id: &str) {
            let info = SurfaceInfo {
                id,
                app_id: app_id.to_owned(),
                title: if title.is_empty() {
                    "MixumART surface".into()
                } else {
                    title.to_owned()
                },
                width: width as u32,
                height: height as u32,
            };
            if let Ok(payload) = serde_json::to_vec(&info) {
                broadcast(&self.clients, MSG_SURFACE_CREATE, &payload);
            }
        }

        fn send_frame(&self, frame: Frame) {
            broadcast(&self.clients, MSG_FRAME, &frame.encode());
        }

        fn handle_input(&mut self, event: InputEvent) {
            let id = match &event {
                InputEvent::PointerMotion { surface, .. }
                | InputEvent::PointerButton { surface, .. }
                | InputEvent::Key { surface, .. }
                | InputEvent::Focus { surface } => *surface,
            };
            let surface = self
                .surfaces
                .get(&id)
                .or_else(|| self.surfaces.values().next())
                .map(|entry| entry.wl_surface.clone());
            let Some(surface) = surface else {
                return;
            };
            let time = self.start.elapsed().as_millis() as u32;
            self.serial = self.serial.wrapping_add(1);
            let serial: Serial = self.serial.into();

            match event {
                InputEvent::PointerMotion { x, y, .. } => {
                    if let Some(pointer) = self.seat.get_pointer() {
                        let location: Point<f64, smithay::utils::Logical> = (x, y).into();
                        pointer.motion(
                            self,
                            Some((surface, location)),
                            &MotionEvent {
                                location,
                                serial,
                                time,
                            },
                        );
                    }
                }
                InputEvent::PointerButton { button, action, .. } => {
                    if let Some(keyboard) = self.seat.get_keyboard() {
                        keyboard.set_focus(self, Some(surface.clone()), serial);
                    }
                    let code = match button.as_str() {
                        "right" => 0x111,
                        "middle" => 0x112,
                        _ => 0x110,
                    };
                    if let Some(pointer) = self.seat.get_pointer() {
                        if action == "press" || action == "click" {
                            pointer.button(
                                self,
                                &ButtonEvent {
                                    serial,
                                    time,
                                    button: code,
                                    state: ButtonState::Pressed,
                                },
                            );
                        }
                        if action == "release" || action == "click" {
                            pointer.button(
                                self,
                                &ButtonEvent {
                                    serial,
                                    time,
                                    button: code,
                                    state: ButtonState::Released,
                                },
                            );
                        }
                    }
                }
                InputEvent::Key { keysym, action, .. } => {
                    let Some(keycode) = keysym_to_keycode(&keysym) else {
                        return;
                    };
                    if let Some(keyboard) = self.seat.get_keyboard() {
                        keyboard.set_focus(self, Some(surface), serial);
                        let mut send = |state| {
                            keyboard.input::<(), _>(
                                self,
                                keycode,
                                state,
                                serial,
                                time,
                                |_, _, _| FilterResult::Forward,
                            );
                        };
                        if action == "press" || action == "click" {
                            send(KeyState::Pressed);
                        }
                        if action == "release" || action == "click" {
                            send(KeyState::Released);
                        }
                    }
                }
                InputEvent::Focus { .. } => {
                    if let Some(keyboard) = self.seat.get_keyboard() {
                        keyboard.set_focus(self, Some(surface), serial);
                    }
                }
            }
        }

        fn send_frame_callbacks(&self, surface: &WlSurface, time: u32) {
            smithay::wayland::compositor::with_states(surface, |states| {
                for callback in states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .frame_callbacks
                    .drain(..)
                {
                    callback.done(time);
                }
            });
        }

        fn on_commit(&mut self, surface: &WlSurface) {
            let Some(id) = self.entry_for(surface) else {
                return;
            };
            let time = self.start.elapsed().as_millis() as u32;

            // on_commit_buffer_handler moves the attached buffer into the
            // renderer surface state; read it from there.
            let buffer = smithay::wayland::compositor::with_states(surface, |states| {
                states
                    .data_map
                    .get::<RendererSurfaceStateUserData>()
                    .and_then(|data| data.lock().ok())
                    .and_then(|data| data.buffer().map(|buffer| (**buffer).clone()))
            });
            let Some(buffer) = buffer else {
                self.send_frame_callbacks(surface, time);
                return;
            };

            let captured = with_buffer_contents(&buffer, |ptr, len, data| {
                let width = data.width.max(0) as usize;
                let height = data.height.max(0) as usize;
                let stride = data.stride.max(0) as usize;
                let offset = data.offset.max(0) as usize;
                if width == 0 || height == 0 || stride < width * 4 {
                    return None;
                }
                if offset.checked_add(stride.checked_mul(height)?)? > len {
                    return None;
                }
                let opaque = match data.format {
                    wl_shm::Format::Argb8888 => false,
                    wl_shm::Format::Xrgb8888 => true,
                    _ => return None,
                };
                let source =
                    unsafe { std::slice::from_raw_parts(ptr.add(offset), stride * height) };
                let mut rgba = vec![0u8; width * height * 4];
                for row in 0..height {
                    let src = &source[row * stride..row * stride + width * 4];
                    let dst = &mut rgba[row * width * 4..(row + 1) * width * 4];
                    for col in 0..width {
                        let b = src[col * 4];
                        let g = src[col * 4 + 1];
                        let r = src[col * 4 + 2];
                        let a = if opaque { 255 } else { src[col * 4 + 3] };
                        dst[col * 4] = r;
                        dst[col * 4 + 1] = g;
                        dst[col * 4 + 2] = b;
                        dst[col * 4 + 3] = a;
                    }
                }
                Some((width, height, rgba))
            })
            .ok()
            .flatten();
            buffer.release();

            let Some((width, height, rgba)) = captured else {
                self.send_frame_callbacks(surface, time);
                return;
            };

            let (title, app_id) = toplevel_meta(surface);
            let mut announce: Option<(u32, usize, usize, String, String)> = None;
            let mut outgoing: Option<(usize, usize, usize, usize, Vec<u8>)> = None;
            {
                let entry = self.surfaces.get_mut(&id).expect("surface entry");
                if !title.is_empty() {
                    entry.title = title;
                }
                if !app_id.is_empty() {
                    entry.app_id = app_id;
                }
                if !entry.announced {
                    entry.announced = true;
                    announce = Some((id, width, height, entry.title.clone(), entry.app_id.clone()));
                }
                if !entry.have_prev || entry.width != width || entry.height != height {
                    if let Some(png) = encode_png(&rgba, width, height) {
                        outgoing = Some((0, 0, width, height, png));
                    }
                    entry.width = width;
                    entry.height = height;
                    entry.prev = rgba;
                    entry.have_prev = true;
                } else if let Some((x, y, w, h)) = diff_bbox(&entry.prev, &rgba, width, height) {
                    let patch = extract(&rgba, width, x, y, w, h);
                    if let Some(png) = encode_png(&patch, w, h) {
                        outgoing = Some((x, y, w, h, png));
                    }
                    entry.prev = rgba;
                } else {
                    // Nothing changed. B0 has no cursor overlay yet, so there is
                    // nothing to send; frame callbacks still go out below.
                }
            }
            if let Some((id, width, height, title, app_id)) = announce {
                self.announce(id, width, height, &title, &app_id);
            }
            if let Some((x, y, w, h, png)) = outgoing {
                let (full_w, full_h) = self
                    .surfaces
                    .get(&id)
                    .map(|entry| (entry.width, entry.height))
                    .unwrap_or((width, height));
                self.send_frame(Frame {
                    surface: id,
                    full_w: full_w as u32,
                    full_h: full_h as u32,
                    x: x as u32,
                    y: y as u32,
                    w: w as u32,
                    h: h as u32,
                    format: FORMAT_PNG,
                    data: png,
                });
            }
            self.send_frame_callbacks(surface, time);
        }
    }

    impl CompositorHandler for Art {
        fn compositor_state(&mut self) -> &mut CompositorState {
            &mut self.compositor_state
        }

        fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
            &client.get_data::<ClientState>().unwrap().compositor_state
        }

        fn commit(&mut self, surface: &WlSurface) {
            on_commit_buffer_handler::<Self>(surface);
            self.on_commit(surface);
        }
    }

    impl BufferHandler for Art {
        fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
    }

    impl ShmHandler for Art {
        fn shm_state(&self) -> &ShmState {
            &self.shm_state
        }
    }

    impl XdgShellHandler for Art {
        fn xdg_shell_state(&mut self) -> &mut XdgShellState {
            &mut self.xdg_shell_state
        }

        fn new_toplevel(&mut self, surface: ToplevelSurface) {
            let wl_surface = surface.wl_surface().clone();
            let (title, app_id) = toplevel_meta(&wl_surface);
            let id = self.next_id;
            self.next_id += 1;
            surface.with_pending_state(|state| {
                state.size = Some(Size::from((WINDOW_WIDTH, WINDOW_HEIGHT)));
                state.states.set(xdg_toplevel::State::Activated);
            });
            surface.send_configure();
            self.surfaces.insert(
                id,
                SurfaceEntry {
                    _toplevel: surface,
                    wl_surface,
                    title,
                    app_id,
                    announced: false,
                    width: 0,
                    height: 0,
                    prev: Vec::new(),
                    have_prev: false,
                },
            );
        }

        fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

        fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
            let id = self
                .surfaces
                .iter()
                .find(|(_, entry)| entry.wl_surface.id() == surface.wl_surface().id())
                .map(|(id, _)| *id);
            if let Some(id) = id {
                self.surfaces.remove(&id);
                if let Ok(payload) = serde_json::to_vec(&SurfaceDestroy { id }) {
                    broadcast(&self.clients, MSG_SURFACE_DESTROY, &payload);
                }
            }
        }

        fn grab(
            &mut self,
            _surface: PopupSurface,
            _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
            _serial: smithay::utils::Serial,
        ) {
        }

        fn reposition_request(
            &mut self,
            _surface: PopupSurface,
            _positioner: PositionerState,
            _token: u32,
        ) {
        }
    }

    impl OutputHandler for Art {}

    impl SelectionHandler for Art {
        type SelectionUserData = ();
    }

    impl DataDeviceHandler for Art {
        fn data_device_state(&self) -> &DataDeviceState {
            &self.data_device_state
        }
    }

    impl ClientDndGrabHandler for Art {}
    impl ServerDndGrabHandler for Art {}

    impl SeatHandler for Art {
        type KeyboardFocus = WlSurface;
        type PointerFocus = WlSurface;
        type TouchFocus = WlSurface;

        fn seat_state(&mut self) -> &mut SeatState<Self> {
            &mut self.seat_state
        }

        fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}

        fn cursor_image(
            &mut self,
            _seat: &Seat<Self>,
            _image: smithay::input::pointer::CursorImageStatus,
        ) {
        }
    }

    fn toplevel_meta(surface: &WlSurface) -> (String, String) {
        smithay::wayland::compositor::with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|data| data.lock().ok())
                .map(|data| {
                    (
                        data.title.clone().unwrap_or_default(),
                        data.app_id.clone().unwrap_or_default(),
                    )
                })
        })
        .unwrap_or_default()
    }

    fn encode_png(rgba: &[u8], width: usize, height: usize) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().ok()?;
            writer.write_image_data(rgba).ok()?;
        }
        Some(out)
    }

    fn diff_bbox(
        previous: &[u8],
        current: &[u8],
        width: usize,
        height: usize,
    ) -> Option<(usize, usize, usize, usize)> {
        let mut min_x = width;
        let mut min_y = height;
        let mut max_x = 0usize;
        let mut max_y = 0usize;
        let mut found = false;
        for y in 0..height {
            let a = &previous[y * width * 4..(y + 1) * width * 4];
            let b = &current[y * width * 4..(y + 1) * width * 4];
            if a == b {
                continue;
            }
            let mut first = None;
            let mut last = 0usize;
            for x in 0..width {
                if a[x * 4..x * 4 + 4] != b[x * 4..x * 4 + 4] {
                    first.get_or_insert(x);
                    last = x;
                }
            }
            if let Some(first) = first {
                found = true;
                min_x = min_x.min(first);
                max_x = max_x.max(last);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
        found.then_some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
    }

    fn extract(rgba: &[u8], width: usize, x: usize, y: usize, w: usize, h: usize) -> Vec<u8> {
        let mut patch = vec![0u8; w * h * 4];
        for row in 0..h {
            let src = &rgba[((y + row) * width + x) * 4..((y + row) * width + x + w) * 4];
            patch[row * w * 4..(row + 1) * w * 4].copy_from_slice(src);
        }
        patch
    }

    fn broadcast(clients: &Arc<Mutex<Vec<TcpStream>>>, kind: u8, payload: &[u8]) {
        let message = encode_message(kind, payload);
        let mut list = clients.lock().expect("clients");
        list.retain_mut(|stream| stream.write_all(&message).is_ok());
    }

    fn spawn_tcp(
        clients: Arc<Mutex<Vec<TcpStream>>>,
        input_tx: Sender<InputEvent>,
    ) -> std::io::Result<()> {
        let listener = TcpListener::bind(("0.0.0.0", TCP_PORT))?;
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                stream.set_nodelay(true).ok();
                if let Ok(mut reader) = stream.try_clone() {
                    let tx = input_tx.clone();
                    std::thread::spawn(move || loop {
                        match read_message(&mut reader) {
                            Ok((MSG_INPUT, payload)) => {
                                match serde_json::from_slice::<InputEvent>(&payload) {
                                    Ok(event) => {
                                        if tx.send(event).is_err() {
                                            return;
                                        }
                                    }
                                    Err(error) => eprintln!("input json: {error}"),
                                }
                            }
                            Ok(_) => {}
                            Err(_) => return,
                        }
                    });
                }
                clients.lock().expect("clients").push(stream);
            }
        });
        Ok(())
    }

    fn keysym_to_keycode(name: &str) -> Option<Keycode> {
        let evdev = match name {
            "Return" | "Enter" => 28,
            "BackSpace" => 14,
            "Tab" => 15,
            "space" => 57,
            "Escape" => 1,
            "Left" => 105,
            "Right" => 106,
            "Up" => 103,
            "Down" => 108,
            "minus" => 12,
            "equal" => 13,
            "comma" => 51,
            "period" => 52,
            "slash" => 53,
            "semicolon" => 39,
            "apostrophe" => 40,
            value if value.len() == 1 => {
                let ch = value.as_bytes()[0];
                match ch {
                    b'a'..=b'z' => 30 + (ch - b'a') as u32,
                    b'0'..=b'9' => {
                        if ch == b'0' {
                            11
                        } else {
                            (ch - b'0') as u32 + 1
                        }
                    }
                    _ => return None,
                }
            }
            _ => return None,
        };
        Some(Keycode::new(evdev + 8))
    }

    smithay::delegate_compositor!(Art);
    smithay::delegate_shm!(Art);
    smithay::delegate_xdg_shell!(Art);
    smithay::delegate_seat!(Art);
    smithay::delegate_output!(Art);
    smithay::delegate_data_device!(Art);

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let mut display = Display::<Art>::new()?;
        let mut display_handle = display.handle();

        let clients = Arc::new(Mutex::new(Vec::new()));
        let (input_tx, input_rx) = mpsc::channel();
        spawn_tcp(clients.clone(), input_tx)?;

        // Remove a stale socket from a previous run; the lock file is handled
        // by wayland-server itself.
        if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
            let _ = std::fs::remove_file(format!("{runtime}/mixum-art-0"));
            let _ = std::fs::remove_file(format!("{runtime}/mixum-art-0.lock"));
        }
        let listener = ListeningSocket::bind("mixum-art-0")?;
        println!(
            "mixum-art ready: wayland={:?} tcp=0.0.0.0:{TCP_PORT}",
            listener.socket_name()
        );

        let mut state = Art::new(&display_handle, clients, input_rx);
        loop {
            while let Ok(event) = state.input_rx.try_recv() {
                state.handle_input(event);
            }
            while let Some(stream) = listener.accept()? {
                let _ = display_handle.insert_client(stream, Arc::new(ClientState::default()));
            }
            display.dispatch_clients(&mut state)?;
            display.flush_clients()?;
            std::thread::sleep(Duration::from_millis(4));
        }
    }
}
