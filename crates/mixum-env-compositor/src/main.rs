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
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        os::unix::net::UnixStream,
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
        desktop::{PopupKind, PopupManager},
        wayland::{
            buffer::BufferHandler,
            compositor::{
                get_parent, with_surface_tree_upward, CompositorClientState, CompositorHandler,
                CompositorState, SubsurfaceCachedState, SurfaceAttributes, SurfaceData,
                TraversalAction,
            },
            output::OutputHandler,
            selection::{
                data_device::{
                    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
                },
                SelectionHandler,
            },
            shell::xdg::{
                PopupSurface, PositionerState, ToplevelSurface, XdgPopupSurfaceData,
                XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
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

    enum MapEvent {
        Input(InputEvent),
        StreamStart(u32),
    }

    /// B compositor 作为 agentd 的 comp 客户端：JSON 控制 + 0x04 PNG 帧。
    struct MapTransport {
        writer: Arc<Mutex<UnixStream>>,
    }

    impl MapTransport {
        fn connect(path: &str) -> std::io::Result<(Self, Receiver<MapEvent>)> {
            let stream = UnixStream::connect(path)?;
            eprintln!("map connected to {path}");
            let mut reader = stream.try_clone()?;
            let writer = Arc::new(Mutex::new(stream));
            let (event_tx, event_rx) = mpsc::channel();
            std::thread::spawn(move || loop {
                let mut header = [0u8; 5];
                if reader.read_exact(&mut header).is_err() {
                    break;
                }
                let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
                if len == 0 || len > 16 * 1024 * 1024 {
                    break;
                }
                let mut payload = vec![0u8; len];
                if reader.read_exact(&mut payload).is_err() {
                    break;
                }
                if header[4] != 0x01 {
                    continue;
                }
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&payload) else {
                    continue;
                };
                let kind = value.get("type").and_then(serde_json::Value::as_str);
                match kind {
                    Some("input") => {
                        if let Some(event) = input_from_control(&value) {
                            let _ = event_tx.send(event);
                        }
                    }
                    Some("stream.start") => {
                        if let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) {
                            let _ = event_tx.send(MapEvent::StreamStart(id as u32));
                        }
                    }
                    _ => {}
                }
            });
            Ok((Self { writer }, event_rx))
        }

        fn send_json(&self, value: &serde_json::Value) {
            let payload = serde_json::to_vec(value).unwrap_or_default();
            if let Ok(mut stream) = self.writer.lock() {
                let _ = stream.write_all(&(payload.len() as u32).to_le_bytes());
                let _ = stream.write_all(&[0x01]);
                let _ = stream.write_all(&payload);
            }
        }

        fn window(&self, kind: &str, entry: &SurfaceEntry) {
            self.send_json(&serde_json::json!({
                "type": kind,
                "id": entry.id,
                "appId": entry.app_id,
                "title": entry.title,
                "width": entry.width as u32,
                "height": entry.height as u32,
                "focused": false,
            }));
        }

        fn send_frame(&self, frame: &Frame) {
            let full =
                frame.x == 0 && frame.y == 0 && frame.w == frame.full_w && frame.h == frame.full_h;
            let patch = if full { 0 } else { 20 };
            let size = patch + frame.data.len();
            if size > 16 * 1024 * 1024 - 27 {
                return;
            }
            let mut payload = vec![0u8; 27 + size];
            payload[0] = 0x01;
            payload[1..5].copy_from_slice(&frame.surface.to_le_bytes());
            payload[5..9].copy_from_slice(&frame.full_w.to_le_bytes());
            payload[9..13].copy_from_slice(&frame.full_h.to_le_bytes());
            payload[13..17].copy_from_slice(&(size as u32).to_le_bytes());
            let flags: u16 = if full { 0x0001 } else { 0x0004 };
            payload[17..19].copy_from_slice(&flags.to_le_bytes());
            if full {
                payload[27..].copy_from_slice(&frame.data);
            } else {
                payload[27..31].copy_from_slice(&frame.x.to_le_bytes());
                payload[31..35].copy_from_slice(&frame.y.to_le_bytes());
                payload[35..39].copy_from_slice(&frame.w.to_le_bytes());
                payload[39..43].copy_from_slice(&frame.h.to_le_bytes());
                payload[43..47].copy_from_slice(&2u32.to_le_bytes());
                payload[47..].copy_from_slice(&frame.data);
            }
            if let Ok(mut stream) = self.writer.lock() {
                let _ = stream.write_all(&(payload.len() as u32).to_le_bytes());
                let _ = stream.write_all(&[0x04]);
                let _ = stream.write_all(&payload);
            }
        }
    }

    fn input_from_control(value: &serde_json::Value) -> Option<MapEvent> {
        let id = value.get("id")?.as_u64()? as u32;
        let kind = value.get("kind")?.as_str()?;
        let action = value
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("click");
        let event = match kind {
            "pointer_motion" => InputEvent::PointerMotion {
                surface: id,
                x: value.get("x")?.as_f64()?,
                y: value.get("y")?.as_f64()?,
            },
            "pointer_button" => InputEvent::PointerButton {
                surface: id,
                button: value
                    .get("button")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("left")
                    .to_string(),
                action: action.to_string(),
            },
            "key" => InputEvent::Key {
                surface: id,
                keysym: value
                    .get("keysym")
                    .and_then(serde_json::Value::as_str)?
                    .to_string(),
                action: action.to_string(),
            },
            _ => return None,
        };
        Some(MapEvent::Input(event))
    }

    struct SurfaceEntry {
        id: u32,
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
        last_full: Option<Vec<u8>>,
    }

    struct Art {
        start: Instant,
        compositor_state: CompositorState,
        xdg_shell_state: XdgShellState,
        shm_state: ShmState,
        data_device_state: DataDeviceState,
        popup_manager: PopupManager,
        popup_parents: std::collections::HashMap<WlSurface, WlSurface>,
        pointer_pos: (f64, f64),
        pointer_root: Option<WlSurface>,
        active_popup: Option<WlSurface>,
        seat_state: SeatState<Self>,
        seat: Seat<Self>,
        _output: Output,
        surfaces: HashMap<u32, SurfaceEntry>,
        next_id: u32,
        clients: Arc<Mutex<Vec<TcpStream>>>,
        new_clients: Receiver<TcpStream>,
        input_rx: Receiver<InputEvent>,
        map: Option<MapTransport>,
        map_rx: Receiver<MapEvent>,
        serial: u32,
    }

    impl Art {
        fn new(
            display_handle: &DisplayHandle,
            clients: Arc<Mutex<Vec<TcpStream>>>,
            new_clients: Receiver<TcpStream>,
            input_rx: Receiver<InputEvent>,
            map: Option<MapTransport>,
            map_rx: Receiver<MapEvent>,
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
                popup_manager: PopupManager::default(),
                popup_parents: std::collections::HashMap::new(),
                pointer_pos: (0.0, 0.0),
                pointer_root: None,
                active_popup: None,
                seat_state,
                seat,
                _output: output,
                surfaces: HashMap::new(),
                next_id: 1000,
                clients,
                new_clients,
                input_rx,
                map,
                map_rx,
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
            if let Some(map) = &self.map {
                if let Some(entry) = self.surfaces.get(&id) {
                    map.window("window.open", entry);
                }
            }
        }

        fn send_frame(&self, frame: Frame) {
            if let Some(map) = &self.map {
                map.send_frame(&frame);
            }
            broadcast(&self.clients, MSG_FRAME, &frame.encode());
        }

        fn popup_hit(
            &self,
            root: &WlSurface,
            x: f64,
            y: f64,
        ) -> Option<(WlSurface, f64, f64)> {
            let mut hit = None;
            for (popup, location) in PopupManager::popups_for_surface(root) {
                let surface = popup.wl_surface();
                let size = smithay::wayland::compositor::with_states(surface, |states| {
                    states
                        .data_map
                        .get::<RendererSurfaceStateUserData>()
                        .and_then(|data| data.lock().ok())
                        .and_then(|data| data.buffer_size())
                });
                let Some(size) = size else {
                    continue;
                };
                let left = location.x as f64;
                let top = location.y as f64;
                let right = left + size.w as f64;
                let bottom = top + size.h as f64;
                if x >= left && x < right && y >= top && y < bottom {
                    hit = Some((surface.clone(), x - left, y - top));
                }
            }
            hit
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
                    self.pointer_pos = (x, y);
                    self.pointer_root = Some(surface.clone());
                    let (target, local_x, local_y) =
                        self.popup_hit(&surface, x, y).unwrap_or((surface, x, y));
                    if let Some(pointer) = self.seat.get_pointer() {
                        let location: Point<f64, smithay::utils::Logical> =
                            (local_x, local_y).into();
                        pointer.motion(
                            self,
                            Some((target, location)),
                            &MotionEvent {
                                location,
                                serial,
                                time,
                            },
                        );
                    }
                }
                InputEvent::PointerButton { button, action, .. } => {
                    let hit = self
                        .pointer_root
                        .as_ref()
                        .and_then(|root| self.popup_hit(root, self.pointer_pos.0, self.pointer_pos.1));
                    let (target, local_x, local_y) = hit
                        .clone()
                        .unwrap_or((surface.clone(), self.pointer_pos.0, self.pointer_pos.1));
                    self.active_popup = hit.map(|(surface, _, _)| surface.clone());
                    if let Some(keyboard) = self.seat.get_keyboard() {
                        keyboard.set_focus(self, Some(target.clone()), serial);
                    }
                    if let Some(pointer) = self.seat.get_pointer() {
                        let location: Point<f64, smithay::utils::Logical> =
                            (local_x, local_y).into();
                        pointer.motion(
                            self,
                            Some((target, location)),
                            &MotionEvent {
                                location,
                                serial,
                                time,
                            },
                        );
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
                        keyboard.set_focus(
                            self,
                            Some(self.active_popup.clone().unwrap_or(surface)),
                            serial,
                        );
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
                        keyboard.set_focus(
                            self,
                            Some(self.active_popup.clone().unwrap_or(surface)),
                            serial,
                        );
                    }
                }
            }
        }

        /// A new host connection gets the current surface table and the last
        /// full frame, so viewers can attach after the app has started.
        fn send_snapshot(&self, stream: &mut TcpStream) {
            for entry in self.surfaces.values().filter(|entry| entry.announced) {
                let info = SurfaceInfo {
                    id: entry.id,
                    app_id: entry.app_id.clone(),
                    title: entry.title.clone(),
                    width: entry.width as u32,
                    height: entry.height as u32,
                };
                if let Ok(payload) = serde_json::to_vec(&info) {
                    let message = encode_message(MSG_SURFACE_CREATE, &payload);
                    if stream.write_all(&message).is_err() {
                        return;
                    }
                }
                if let Some(png) = entry
                    .have_prev
                    .then(|| encode_png(&entry.prev, entry.width, entry.height))
                    .flatten()
                    .or_else(|| entry.last_full.clone())
                {
                    let frame = Frame {
                        surface: entry.id,
                        full_w: entry.width as u32,
                        full_h: entry.height as u32,
                        x: 0,
                        y: 0,
                        w: entry.width as u32,
                        h: entry.height as u32,
                        format: FORMAT_PNG,
                        data: png,
                    };
                    let message = encode_message(MSG_FRAME, &frame.encode());
                    if stream.write_all(&message).is_err() {
                        return;
                    }
                }
            }
        }

        fn send_last_full(&self, id: u32) {
            let Some(entry) = self.surfaces.get(&id) else {
                return;
            };
            // last_full is only refreshed on full frames; when a viewer
            // attaches after patch-only commits, encode the current composite
            // so subsurfaces/popups are present.
            let Some(png) = entry
                .have_prev
                .then(|| encode_png(&entry.prev, entry.width, entry.height))
                .flatten()
                .or_else(|| entry.last_full.clone())
            else {
                return;
            };
            let Some(map) = &self.map else {
                return;
            };
            map.send_frame(&Frame {
                surface: id,
                full_w: entry.width as u32,
                full_h: entry.height as u32,
                x: 0,
                y: 0,
                w: entry.width as u32,
                h: entry.height as u32,
                format: FORMAT_PNG,
                data: png,
            });
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

        fn root_surface(&self, surface: &WlSurface) -> WlSurface {
            let mut current = surface.clone();
            for _ in 0..16 {
                if let Some(parent) = get_parent(&current) {
                    current = parent;
                    continue;
                }
                if let Some(parent) = self.popup_parents.get(&current) {
                    current = parent.clone();
                    continue;
                }
                break;
            }
            current
        }

        fn offset_from_states(states: &SurfaceData) -> (i32, i32) {
            if let Some(data) = states.data_map.get::<XdgPopupSurfaceData>() {
                if let Ok(attributes) = data.lock() {
                    let geometry = attributes.current_server_state().geometry;
                    return (geometry.loc.x, geometry.loc.y);
                }
            }
            let mut cached = states.cached_state.get::<SubsurfaceCachedState>();
            let location = cached.current().location;
            (location.x, location.y)
        }

        fn read_states_rgba(states: &SurfaceData) -> Option<(usize, usize, Vec<u8>)> {
            let buffer = states
                .data_map
                .get::<RendererSurfaceStateUserData>()
                .and_then(|data| data.lock().ok())
                .and_then(|data| data.buffer().map(|buffer| (**buffer).clone()))?;
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
            captured
        }

        fn read_surface_rgba(surface: &WlSurface) -> Option<(usize, usize, Vec<u8>)> {
            smithay::wayland::compositor::with_states(surface, Self::read_states_rgba)
        }

        fn composite(root: &WlSurface, canvas_w: usize, canvas_h: usize) -> Option<Vec<u8>> {
            let mut canvas = vec![0u8; canvas_w * canvas_h * 4];
            let mut drew = false;
            with_surface_tree_upward(
                root,
                (0i32, 0i32),
                |_surface, states, location: &(i32, i32)| {
                    let (ox, oy) = Self::offset_from_states(states);
                    let loc = (location.0 + ox, location.1 + oy);
                    TraversalAction::DoChildren(loc)
                },
                |_surface, states, location: &(i32, i32)| {
                    let (ox, oy) = Self::offset_from_states(states);
                    let (x, y) = (location.0 + ox, location.1 + oy);
                    let Some((width, height, rgba)) = Self::read_states_rgba(states) else {
                        return;
                    };
                    drew = true;
                    for row in 0..height {
                        let dy = y + row as i32;
                        if dy < 0 || dy >= canvas_h as i32 {
                            continue;
                        }
                        for col in 0..width {
                            let dx = x + col as i32;
                            if dx < 0 || dx >= canvas_w as i32 {
                                continue;
                            }
                            let src = (row * width + col) * 4;
                            let dst = (dy as usize * canvas_w + dx as usize) * 4;
                            let alpha = rgba[src + 3] as u32;
                            if alpha == 0 {
                                continue;
                            }
                            if alpha == 255 {
                                canvas[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
                            } else {
                                let inv = 255 - alpha;
                                for channel in 0..3 {
                                    canvas[dst + channel] = ((rgba[src + channel] as u32 * alpha
                                        + canvas[dst + channel] as u32 * inv)
                                        / 255) as u8;
                                }
                                canvas[dst + 3] = 255;
                            }
                        }
                    }
                },
                |_, _, _| true,
            );
            // Popups are not part of the compositor surface tree; Smithay's
            // PopupManager tracks them with parent-relative locations.
            for (popup, location) in PopupManager::popups_for_surface(root) {
                let popup_surface = popup.wl_surface();
                let Some((width, height, rgba)) = Self::read_surface_rgba(popup_surface) else {
                    continue;
                };
                drew = true;
                let (x, y) = (location.x, location.y);
                for row in 0..height {
                    let dy = y + row as i32;
                    if dy < 0 || dy >= canvas_h as i32 {
                        continue;
                    }
                    for col in 0..width {
                        let dx = x + col as i32;
                        if dx < 0 || dx >= canvas_w as i32 {
                            continue;
                        }
                        let src = (row * width + col) * 4;
                        let dst = (dy as usize * canvas_w + dx as usize) * 4;
                        let alpha = rgba[src + 3] as u32;
                        if alpha == 0 {
                            continue;
                        }
                        if alpha == 255 {
                            canvas[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
                        } else {
                            let inv = 255 - alpha;
                            for channel in 0..3 {
                                canvas[dst + channel] = ((rgba[src + channel] as u32 * alpha
                                    + canvas[dst + channel] as u32 * inv)
                                    / 255) as u8;
                            }
                            canvas[dst + 3] = 255;
                        }
                    }
                }
            }
            drew.then_some(canvas)
        }

        fn on_commit(&mut self, surface: &WlSurface) {
            let time = self.start.elapsed().as_millis() as u32;
            let root = self.root_surface(surface);
            let Some(id) = self.entry_for(&root) else {
                self.send_frame_callbacks(surface, time);
                return;
            };
            let (mut width, mut height) = self
                .surfaces
                .get(&id)
                .map(|entry| (entry.width, entry.height))
                .unwrap_or((0, 0));
            if width == 0 || height == 0 {
                if let Some((w, h, _)) = Self::read_surface_rgba(&root) {
                    width = w;
                    height = h;
                }
            }
            if width == 0 || height == 0 {
                self.send_frame_callbacks(surface, time);
                return;
            }
            let Some(rgba) = Self::composite(&root, width, height) else {
                self.send_frame_callbacks(surface, time);
                return;
            };

            let (title, app_id) = toplevel_meta(&root);
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
                        entry.last_full = Some(png.clone());
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
            self.popup_manager.commit(surface);
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
                    id,
                    _toplevel: surface,
                    wl_surface,
                    title,
                    app_id,
                    announced: false,
                    width: 0,
                    height: 0,
                    prev: Vec::new(),
                    have_prev: false,
                    last_full: None,
                },
            );
        }

        fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
            if let Some(parent) = surface.get_parent_surface() {
                self.popup_parents
                    .insert(surface.wl_surface().clone(), parent);
            }
            let _ = self
                .popup_manager
                .track_popup(PopupKind::Xdg(surface.clone()));
            let _ = surface.send_configure();
        }

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
                if let Some(map) = &self.map {
                    map.send_json(&serde_json::json!({"type": "window.close", "id": id}));
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
        new_tx: Sender<TcpStream>,
        input_tx: Sender<InputEvent>,
        port: u16,
    ) -> std::io::Result<()> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
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
                if new_tx.send(stream).is_err() {
                    return;
                }
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
                // Linux evdev keycodes are not alphabetical.
                match ch {
                    b'a' => 30,
                    b'b' => 48,
                    b'c' => 46,
                    b'd' => 32,
                    b'e' => 18,
                    b'f' => 33,
                    b'g' => 34,
                    b'h' => 35,
                    b'i' => 23,
                    b'j' => 36,
                    b'k' => 37,
                    b'l' => 38,
                    b'm' => 50,
                    b'n' => 49,
                    b'o' => 24,
                    b'p' => 25,
                    b'q' => 16,
                    b'r' => 19,
                    b's' => 31,
                    b't' => 20,
                    b'u' => 22,
                    b'v' => 47,
                    b'w' => 17,
                    b'x' => 45,
                    b'y' => 21,
                    b'z' => 44,
                    b'0' => 11,
                    b'1' => 2,
                    b'2' => 3,
                    b'3' => 4,
                    b'4' => 5,
                    b'5' => 6,
                    b'6' => 7,
                    b'7' => 8,
                    b'8' => 9,
                    b'9' => 10,
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
        let mut map_socket: Option<String> = None;
        let mut tcp_port = TCP_PORT;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--map-socket" => {
                    map_socket = Some(args.next().ok_or("--map-socket 需要路径")?);
                }
                "--tcp-port" => {
                    tcp_port = args.next().ok_or("--tcp-port 需要端口")?.parse()?;
                }
                other => return Err(format!("未知参数 {other}").into()),
            }
        }

        let mut display = Display::<Art>::new()?;
        let mut display_handle = display.handle();

        let clients = Arc::new(Mutex::new(Vec::new()));
        let (input_tx, input_rx) = mpsc::channel();
        let (map, map_rx) = if let Some(path) = &map_socket {
            let (map, rx) = MapTransport::connect(path)?;
            (Some(map), rx)
        } else {
            let (_tx, rx) = mpsc::channel();
            (None, rx)
        };
        let (new_tx, new_rx) = mpsc::channel();
        if map.is_none() {
            spawn_tcp(new_tx, input_tx, tcp_port)?;
        }

        // Remove a stale socket from a previous run; the lock file is handled
        // by wayland-server itself.
        if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
            let _ = std::fs::remove_file(format!("{runtime}/mixum-art-0"));
            let _ = std::fs::remove_file(format!("{runtime}/mixum-art-0.lock"));
        }
        let listener = ListeningSocket::bind("mixum-art-0")?;
        println!(
            "mixum-art ready: wayland={:?} transport={}",
            listener.socket_name(),
            map_socket
                .as_deref()
                .map(|path| format!("map:{path}"))
                .unwrap_or_else(|| format!("tcp:0.0.0.0:{tcp_port}"))
        );

        let mut state = Art::new(&display_handle, clients, new_rx, input_rx, map, map_rx);
        loop {
            while let Ok(event) = state.input_rx.try_recv() {
                state.handle_input(event);
            }
            while let Ok(event) = state.map_rx.try_recv() {
                match event {
                    MapEvent::Input(event) => state.handle_input(event),
                    MapEvent::StreamStart(id) => state.send_last_full(id),
                }
            }
            while let Ok(mut stream) = state.new_clients.try_recv() {
                state.send_snapshot(&mut stream);
                state.clients.lock().expect("clients").push(stream);
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
