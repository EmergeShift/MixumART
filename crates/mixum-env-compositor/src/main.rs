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
    use smithay::reexports::calloop;
    use std::{
        collections::HashMap,
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        os::unix::net::UnixStream,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc::{self, Receiver, Sender},
            Arc, Mutex,
        },
        os::fd::OwnedFd,
        process::Stdio,
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
        utils::{Logical, Point, Rectangle, Serial, Size, Transform},
        desktop::{PopupKind, PopupManager},
        xwayland::{X11Surface, X11Wm, XWayland, XWaylandClientData, XWaylandEvent, XwmHandler},
        xwayland::xwm::{Reorder, ResizeEdge, WmWindowProperty, XwmId},
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
                    clear_data_device_selection, request_data_device_client_selection,
                    set_data_device_focus, set_data_device_selection, ClientDndGrabHandler,
                    DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
                },
                SelectionHandler, SelectionTarget,
            },
            shell::xdg::{
                PopupSurface, PositionerState, ToplevelSurface, XdgPopupSurfaceData,
                XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
            },
            shm::{with_buffer_contents, ShmHandler, ShmState},
            input_method::{
                InputMethodHandler, InputMethodManagerState, PopupSurface as InputMethodPopup,
            },
            text_input::{TextInputManagerState, TextInputSeat},
            xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
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
            "text" => InputEvent::Text {
                surface: id,
                text: value
                    .get("text")
                    .and_then(serde_json::Value::as_str)?
                    .to_string(),
            },
            _ => return None,
        };
        Some(MapEvent::Input(event))
    }

    #[allow(dead_code)]
    enum SurfaceRole {
        Xdg(ToplevelSurface),
        X11(X11Surface),
    }

    struct SurfaceEntry {
        id: u32,
        role: SurfaceRole,
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

    #[derive(Clone)]
    struct X11Overlay {
        wl_surface: WlSurface,
        window: X11Surface,
        parent: Option<u32>,
        rgba: Option<(usize, usize, Vec<u8>)>,
    }

    struct Art {
        start: Instant,
        display_handle: DisplayHandle,
        compositor_state: CompositorState,
        xdg_shell_state: XdgShellState,
        shm_state: ShmState,
        data_device_state: DataDeviceState,
        popup_manager: PopupManager,
        popup_parents: std::collections::HashMap<WlSurface, WlSurface>,
        pointer_pos: (f64, f64),
        pointer_root: Option<WlSurface>,
        active_popup: Option<WlSurface>,
        _text_input_state: TextInputManagerState,
        _input_method_state: InputMethodManagerState,
        xwayland_shell_state: XWaylandShellState,
        xwm: Option<X11Wm>,
        xwayland_display: Option<u32>,
        xim_tx: Option<Sender<String>>,
        xim_active: Arc<AtomicBool>,
        loop_handle: Option<calloop::LoopHandle<'static, Self>>,
        x11_windows: HashMap<u32, u32>,
        x11_overlays: HashMap<u32, X11Overlay>,
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
            let xwayland_shell_state = XWaylandShellState::new::<Self>(display_handle);
            let text_input_state = TextInputManagerState::new::<Self>(display_handle);
            let input_method_state =
                InputMethodManagerState::new::<Self, _>(display_handle, |_client| true);

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
                display_handle: display_handle.clone(),
                compositor_state,
                xdg_shell_state,
                shm_state,
                data_device_state,
                popup_manager: PopupManager::default(),
                popup_parents: std::collections::HashMap::new(),
                pointer_pos: (0.0, 0.0),
                pointer_root: None,
                active_popup: None,
                _text_input_state: text_input_state,
                _input_method_state: input_method_state,
                xwayland_shell_state,
                xwm: None,
                xwayland_display: None,
                xim_tx: None,
                xim_active: Arc::new(AtomicBool::new(false)),
                loop_handle: None,
                x11_windows: HashMap::new(),
                x11_overlays: HashMap::new(),
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

        fn focus_text_input(&mut self, surface: &WlSurface, serial: Serial) {
            let text_input = self.seat.text_input();
            text_input.set_focus(Some(surface.clone()));
            text_input.enter();
            let _ = serial;
        }

        fn overlay_hit(&self, root_id: u32, x: f64, y: f64) -> Option<(WlSurface, f64, f64)> {
            let parent = self.surfaces.get(&root_id)?;
            let parent_origin = match &parent.role {
                SurfaceRole::X11(window) => window.geometry().loc,
                SurfaceRole::Xdg(_) => (0, 0).into(),
            };
            let mut hit = None;
            for overlay in self.x11_overlays.values() {
                if self.overlay_parent_id(overlay) != Some(root_id) {
                    continue;
                }
                let Some((width, height, _)) = &overlay.rgba else {
                    continue;
                };
                let geometry = overlay.window.geometry();
                let left = (geometry.loc.x - parent_origin.x) as f64;
                let top = (geometry.loc.y - parent_origin.y) as f64;
                if x >= left
                    && x < left + *width as f64
                    && y >= top
                    && y < top + *height as f64
                {
                    hit = Some((overlay.wl_surface.clone(), x - left, y - top));
                }
            }
            hit
        }

        fn activate_surface(&self, id: u32) {
            if let Some(entry) = self.surfaces.get(&id) {
                if let SurfaceRole::X11(window) = &entry.role {
                    let _ = window.set_activated(true);
                }
            }
        }

        fn x11_type_text(&self, text: &str) {
            use x11rb::connection::Connection;
            use x11rb::protocol::xproto::ConnectionExt as _;
            use x11rb::protocol::xtest::ConnectionExt as _;

            let Some(display_number) = self.xwayland_display else {
                return;
            };
            let Ok((conn, screen)) =
                x11rb::rust_connection::RustConnection::connect(Some(&format!(
                    ":{display_number}"
                )))
            else {
                return;
            };
            let root = conn.setup().roots[screen].root;
            // The first synthetic key after a fresh X11 connection is often
            // consumed while XWayland resolves focus; send Shift as a no-op.
            let _ = conn.xtest_fake_input(2, 50, 0, root, 0, 0, 0);
            let _ = conn.xtest_fake_input(3, 50, 0, root, 0, 0, 0);
            let _ = conn.flush();
            std::thread::sleep(std::time::Duration::from_millis(20));
            let chars: Vec<char> = text.chars().collect();
            for chunk in chars.chunks(16) {
                let keysyms: Vec<u32> = chunk
                    .iter()
                    .map(|ch| 0x0100_0000u32 | *ch as u32)
                    .collect();
                let count = keysyms.len() as u8;
                let first = 240u8;
                let Ok(mapping) =
                    conn.change_keyboard_mapping(count, first, 1, &keysyms)
                else {
                    continue;
                };
                if mapping.check().is_err() {
                    continue;
                }
                if let Ok(cookie) = conn.get_input_focus() {
                    let _ = cookie.reply();
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
                for index in 0..count {
                    let keycode = first + index;
                    let _ = conn.xtest_fake_input(2, keycode, 0, root, 0, 0, 0);
                    let _ = conn.xtest_fake_input(3, keycode, 0, root, 0, 0, 0);
                    let _ = conn.flush();
                    std::thread::sleep(std::time::Duration::from_millis(8));
                }
                let _ = conn.change_keyboard_mapping(count, first, 1, &vec![0; keysyms.len()]);
                let _ = conn.flush();
            }
        }

        fn handle_input(&mut self, event: InputEvent) {
            let id = match &event {
                InputEvent::PointerMotion { surface, .. }
                | InputEvent::PointerButton { surface, .. }
                | InputEvent::Key { surface, .. }
                | InputEvent::Text { surface, .. }
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
                    let (target, local_x, local_y) = self
                        .overlay_hit(id, x, y)
                        .or_else(|| self.popup_hit(&surface, x, y))
                        .unwrap_or((surface, x, y));
                    let origin: Point<f64, smithay::utils::Logical> =
                        (x - local_x, y - local_y).into();
                    let location: Point<f64, smithay::utils::Logical> = (x, y).into();
                    if let Some(pointer) = self.seat.get_pointer() {
                        pointer.motion(
                            self,
                            Some((target, origin)),
                            &MotionEvent {
                                location,
                                serial,
                                time,
                            },
                        );
                    }
                }
                InputEvent::PointerButton { button, action, .. } => {
                    self.activate_surface(id);
                    let hit = self
                        .overlay_hit(id, self.pointer_pos.0, self.pointer_pos.1)
                        .or_else(|| {
                            self.pointer_root.as_ref().and_then(|root| {
                                self.popup_hit(root, self.pointer_pos.0, self.pointer_pos.1)
                            })
                        });
                    let (target, local_x, local_y) = hit
                        .clone()
                        .unwrap_or((surface.clone(), self.pointer_pos.0, self.pointer_pos.1));
                    self.active_popup = hit.map(|(surface, _, _)| surface.clone());
                    if let Some(keyboard) = self.seat.get_keyboard() {
                        keyboard.set_focus(self, Some(target.clone()), serial);
                    }
                    self.focus_text_input(&target, serial);
                    let origin: Point<f64, smithay::utils::Logical> =
                        (self.pointer_pos.0 - local_x, self.pointer_pos.1 - local_y).into();
                    let location: Point<f64, smithay::utils::Logical> = self.pointer_pos.into();
                    if let Some(pointer) = self.seat.get_pointer() {
                        pointer.motion(
                            self,
                            Some((target, origin)),
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
                    self.activate_surface(id);
                    let Some(keycode) = keysym_to_keycode(&keysym) else {
                        return;
                    };
                    if let Some(keyboard) = self.seat.get_keyboard() {
                        let target = self.active_popup.clone().unwrap_or(surface);
                        keyboard.set_focus(self, Some(target.clone()), serial);
                        self.focus_text_input(&target, serial);
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
                InputEvent::Text { text, .. } => {
                    let is_x11 = self
                        .surfaces
                        .get(&id)
                        .map(|entry| matches!(entry.role, SurfaceRole::X11(_)))
                        .unwrap_or(false);
                    if is_x11 {
                        if self.xim_active.load(Ordering::SeqCst) {
                            if let Some(tx) = &self.xim_tx {
                                if tx.send(text.clone()).is_ok() {
                                    return;
                                }
                            }
                        }
                        self.x11_type_text(&text);
                        return;
                    }
                    self.focus_text_input(&surface, serial);
                    let text_input = self.seat.text_input();
                    if !text.is_empty() {
                        text_input.with_focused_text_input(|input, _| {
                            input.commit_string(Some(text.clone()));
                        });
                        text_input.done(false);
                    }
                }
                InputEvent::Focus { .. } => {
                    self.activate_surface(id);
                    if let Some(keyboard) = self.seat.get_keyboard() {
                        let target = self.active_popup.clone().unwrap_or(surface);
                        keyboard.set_focus(self, Some(target.clone()), serial);
                        self.focus_text_input(&target, serial);
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

        fn composite(&self, root: &WlSurface, canvas_w: usize, canvas_h: usize) -> Option<Vec<u8>> {
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
            // X11 override-redirect windows are not in the Wayland surface
            // tree. Draw them into their parent host window at their X11
            // screen offset relative to the parent's origin.
            for overlay in self.x11_overlays.values() {
                let Some(parent_id) = self.overlay_parent_id(overlay) else {
                    continue;
                };
                let Some(parent) = self.surfaces.get(&parent_id) else {
                    continue;
                };
                if parent.wl_surface.id() != root.id() {
                    continue;
                }
                let parent_origin = match &parent.role {
                    SurfaceRole::X11(window) => window.geometry().loc,
                    SurfaceRole::Xdg(_) => (0, 0).into(),
                };
                let geometry = overlay.window.geometry();
                let x = geometry.loc.x - parent_origin.x;
                let y = geometry.loc.y - parent_origin.y;
                let Some((width, height, rgba)) = overlay.rgba.as_ref() else {
                    continue;
                };
                let (width, height) = (*width, *height);
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
            }
            drew.then_some(canvas)
        }

        fn overlay_parent_id(&self, overlay: &X11Overlay) -> Option<u32> {
            if let Some(parent) = overlay.parent {
                if let Some(id) = self.x11_windows.get(&parent) {
                    return Some(*id);
                }
            }
            self.surfaces
                .iter()
                .find(|(_, entry)| matches!(entry.role, SurfaceRole::X11(_)))
                .map(|(id, _)| *id)
        }

        fn on_commit(&mut self, surface: &WlSurface) {
            let time = self.start.elapsed().as_millis() as u32;
            if let Some(overlay) = self
                .x11_overlays
                .get(&surface.id().protocol_id())
                .cloned()
            {
                if let Some(cached) = Self::read_surface_rgba(surface) {
                    if let Some(overlay_mut) = self
                        .x11_overlays
                        .get_mut(&surface.id().protocol_id())
                    {
                        overlay_mut.rgba = Some(cached);
                    }
                }
                if let Some(parent_id) = self.overlay_parent_id(&overlay) {
                    if let Some(parent_root) = self
                        .surfaces
                        .get(&parent_id)
                        .map(|entry| entry.wl_surface.clone())
                    {
                        self.on_commit(&parent_root);
                    }
                }
                self.send_frame_callbacks(surface, time);
                return;
            }
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
            let Some(rgba) = self.composite(&root, width, height) else {
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
            if let Some(state) = client.get_data::<ClientState>() {
                return &state.compositor_state;
            }
            if let Some(state) = client.get_data::<XWaylandClientData>() {
                return &state.compositor_state;
            }
            panic!("client has no compositor state")
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
            // Focus the newly mapped toplevel so clipboard owners can set a
            // selection (data_device requires keyboard focus on the source).
            self.serial = self.serial.wrapping_add(1);
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.set_focus(self, Some(wl_surface.clone()), self.serial.into());
            }
            self.surfaces.insert(
                id,
                SurfaceEntry {
                    id,
                    role: SurfaceRole::Xdg(surface),
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

    impl InputMethodHandler for Art {
        fn new_popup(&mut self, _surface: InputMethodPopup) {}
        fn dismiss_popup(&mut self, _surface: InputMethodPopup) {}
        fn popup_repositioned(&mut self, _surface: InputMethodPopup) {}
        fn parent_geometry(&self, _parent: &WlSurface) -> Rectangle<i32, Logical> {
            Rectangle::default()
        }
    }

    impl XwmHandler for Art {
        fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
            self.xwm.as_mut().expect("X11 WM not started")
        }

        fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {
        }
        fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
        fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
            let _ = window.set_mapped(true);
        }
        fn mapped_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
        fn unmapped_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
        fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
            let Some(wl_surface) = window.wl_surface() else {
                return;
            };
            let entry_id = self.entry_for(&wl_surface);
            if let Some(id) = entry_id {
                self.surfaces.remove(&id);
                if let Ok(payload) = serde_json::to_vec(&SurfaceDestroy { id }) {
                    broadcast(&self.clients, MSG_SURFACE_DESTROY, &payload);
                }
                if let Some(map) = &self.map {
                    map.send_json(&serde_json::json!({"type": "window.close", "id": id}));
                }
            }
            self.x11_windows.retain(|_, host_id| Some(*host_id) != entry_id);
            self.x11_overlays.remove(&wl_surface.id().protocol_id());
        }
        fn configure_request(
            &mut self,
            _xwm: XwmId,
            window: X11Surface,
            x: Option<i32>,
            y: Option<i32>,
            w: Option<u32>,
            h: Option<u32>,
            _reorder: Option<Reorder>,
        ) {
            let mut rect = window.geometry();
            if let Some(x) = x {
                rect.loc.x = x;
            }
            if let Some(y) = y {
                rect.loc.y = y;
            }
            if let Some(w) = w {
                rect.size.w = w as i32;
            }
            if let Some(h) = h {
                rect.size.h = h as i32;
            }
            let _ = window.configure(Some(rect));
        }
        fn configure_notify(
            &mut self,
            _xwm: XwmId,
            _window: X11Surface,
            _geometry: Rectangle<i32, Logical>,
            _above: Option<u32>,
        ) {
        }
        fn resize_request(
            &mut self,
            _xwm: XwmId,
            _window: X11Surface,
            _button: u32,
            _edge: ResizeEdge,
        ) {
        }
        fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}
        fn allow_selection_access(
            &mut self,
            _xwm: XwmId,
            selection: SelectionTarget,
        ) -> bool {
            selection == SelectionTarget::Clipboard
        }

        fn send_selection(
            &mut self,
            _xwm: XwmId,
            selection: SelectionTarget,
            mime_type: String,
            fd: OwnedFd,
        ) {
            if selection != SelectionTarget::Clipboard {
                return;
            }
            let _ = request_data_device_client_selection(&self.seat, mime_type, fd);
        }
        fn property_notify(&mut self, _xwm: XwmId, _window: X11Surface, _property: WmWindowProperty) {}

        fn new_selection(
            &mut self,
            _xwm: XwmId,
            selection: SelectionTarget,
            mime_types: Vec<String>,
        ) {
            if selection == SelectionTarget::Clipboard {
                set_data_device_selection(
                    &self.display_handle,
                    &self.seat,
                    mime_types,
                    SelectionTarget::Clipboard,
                );
            }
        }

        fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
            if selection == SelectionTarget::Clipboard {
                clear_data_device_selection(&self.display_handle, &self.seat);
            }
        }
    }

    impl XWaylandShellHandler for Art {
        fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
            &mut self.xwayland_shell_state
        }

        fn surface_associated(
            &mut self,
            _xwm: XwmId,
            wl_surface: WlSurface,
            window: X11Surface,
        ) {
            if window.is_override_redirect() {
                // Override-redirect windows are menus/tooltips: composite them
                // into their parent instead of creating a host window.
                let overlay = X11Overlay {
                    wl_surface: wl_surface.clone(),
                    window: window.clone(),
                    parent: window.is_transient_for(),
                    rgba: Self::read_surface_rgba(&wl_surface),
                };
                self.x11_overlays
                    .insert(wl_surface.id().protocol_id(), overlay.clone());
                if let Some(parent_id) = self.overlay_parent_id(&overlay) {
                    if let Some(parent_root) = self
                        .surfaces
                        .get(&parent_id)
                        .map(|entry| entry.wl_surface.clone())
                    {
                        self.on_commit(&parent_root);
                    }
                }
                return;
            }
            if self.entry_for(&wl_surface).is_some() {
                return;
            }
            let id = self.next_id;
            self.next_id += 1;
            let geometry = window.geometry();
            self.x11_windows.insert(window.window_id(), id);
            self.surfaces.insert(
                id,
                SurfaceEntry {
                    id,
                    role: SurfaceRole::X11(window.clone()),
                    wl_surface: wl_surface.clone(),
                    title: window.title(),
                    app_id: if window.class().is_empty() {
                        window.instance()
                    } else {
                        window.class()
                    },
                    announced: false,
                    width: geometry.size.w.max(1) as usize,
                    height: geometry.size.h.max(1) as usize,
                    prev: Vec::new(),
                    have_prev: false,
                    last_full: None,
                },
            );
            // XWayland often commits the buffer before the X11 surface is
            // associated with its wl_surface. Capture once now; a later commit
            // will refresh the frame.
            self.on_commit(&wl_surface);
        }
    }

    impl OutputHandler for Art {}

    impl SelectionHandler for Art {
        type SelectionUserData = SelectionTarget;

        fn new_selection(
            &mut self,
            ty: SelectionTarget,
            source: Option<smithay::wayland::selection::SelectionSource>,
            _seat: Seat<Self>,
        ) {
            let mime_types = source
                .as_ref()
                .map(|source| source.mime_types())
                .unwrap_or_default();
            if let Some(xwm) = self.xwm.as_mut() {
                if let Err(error) = xwm.new_selection(
                    ty,
                    source.as_ref().map(|_| mime_types),
                ) {
                    eprintln!("xwm new_selection: {error}");
                }
            }
        }

        fn send_selection(
            &mut self,
            ty: SelectionTarget,
            mime_type: String,
            fd: OwnedFd,
            _seat: Seat<Self>,
            _user_data: &SelectionTarget,
        ) {
            let Some(handle) = self.loop_handle.clone() else {
                return;
            };
            if let Some(xwm) = self.xwm.as_mut() {
                let _ = xwm.send_selection(ty, mime_type, fd, handle);
            }
        }
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

        fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
            let client = focused.and_then(|surface| surface.client());
            set_data_device_focus(&self.display_handle, seat, client);
        }

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

    smithay::delegate_text_input_manager!(Art);
    smithay::delegate_xwayland_shell!(Art);
    smithay::delegate_input_method_manager!(Art);

    /// Minimal input-method v2 client. Smithay's text-input handler ignores
    /// client enable/commit unless an input-method instance exists; this client
    /// provides that instance so host IME commits can reach the focused app.
    fn spawn_input_method() {
        std::thread::spawn(|| {
            use wayland_client::{
                globals::{registry_queue_init, GlobalListContents},
                protocol::{wl_registry, wl_seat},
                Connection, Dispatch, QueueHandle,
            };
            use wayland_protocols_misc::zwp_input_method_v2::client::{
                zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
                zwp_input_method_v2::{self, ZwpInputMethodV2},
            };

            struct State;
            impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
                fn event(
                    _: &mut Self,
                    _: &wl_registry::WlRegistry,
                    _: wl_registry::Event,
                    _: &GlobalListContents,
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }
            impl Dispatch<wl_seat::WlSeat, ()> for State {
                fn event(
                    _: &mut Self,
                    _: &wl_seat::WlSeat,
                    _: wl_seat::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }
            impl Dispatch<ZwpInputMethodManagerV2, ()> for State {
                fn event(
                    _: &mut Self,
                    _: &ZwpInputMethodManagerV2,
                    _: <ZwpInputMethodManagerV2 as wayland_client::Proxy>::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }
            impl Dispatch<ZwpInputMethodV2, ()> for State {
                fn event(
                    _: &mut Self,
                    _: &ZwpInputMethodV2,
                    _: zwp_input_method_v2::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }

            let connection = match Connection::connect_to_env() {
                Ok(connection) => connection,
                Err(error) => {
                    eprintln!("input-method client connect: {error}");
                    return;
                }
            };
            let (globals, mut queue) = match registry_queue_init(&connection) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("input-method registry: {error}");
                    return;
                }
            };
            let qh = queue.handle();
            let seat: wl_seat::WlSeat = match globals.bind(&qh, 1..=9, ()) {
                Ok(seat) => seat,
                Err(error) => {
                    eprintln!("input-method seat: {error}");
                    return;
                }
            };
            let manager: ZwpInputMethodManagerV2 = match globals.bind(&qh, 1..=1, ()) {
                Ok(manager) => manager,
                Err(error) => {
                    eprintln!("input-method manager: {error}");
                    return;
                }
            };
            let _input_method = manager.get_input_method(&seat, &qh, ());
            loop {
                if let Err(error) = queue.blocking_dispatch(&mut State) {
                    eprintln!("input-method dispatch: {error}");
                    break;
                }
            }
        });
    }

    fn start_xim(display_number: u32, active: Arc<AtomicBool>) -> Sender<String> {
        use xim::{
            InputContext, Server, ServerError, ServerHandler, UserInputContext, XimConnections,
            ALL_LOCALES,
        };
        use xim::x11rb::X11rbServer;

        struct XimHandler {
            active: bool,
            focus: Option<(u32, u16, u16, String)>,
            active_flag: Arc<AtomicBool>,
        }
        impl<S: Server<XEvent = x11rb::protocol::xproto::KeyPressEvent>> ServerHandler<S>
            for XimHandler
        {
            type InputContextData = ();
            type InputStyleArray = [xim::InputStyle; 4];
            fn new_ic_data(
                &mut self,
                _server: &mut S,
                _style: xim::InputStyle,
            ) -> Result<(), ServerError> {
                Ok(())
            }
            fn input_styles(&self) -> Self::InputStyleArray {
                [
                    xim::InputStyle::PREEDIT_CALLBACKS | xim::InputStyle::STATUS_NOTHING,
                    xim::InputStyle::PREEDIT_NOTHING | xim::InputStyle::STATUS_NOTHING,
                    xim::InputStyle::PREEDIT_POSITION | xim::InputStyle::STATUS_NOTHING,
                    xim::InputStyle::PREEDIT_POSITION | xim::InputStyle::STATUS_NONE,
                ]
            }
            fn filter_events(&self) -> u32 { 1 }
            fn handle_connect(&mut self, _server: &mut S) -> Result<(), ServerError> {
                self.active = true;
                Ok(())
            }
            fn handle_create_ic(
                &mut self,
                server: &mut S,
                user_ic: &mut UserInputContext<()>,
            ) -> Result<(), ServerError> {
                server.set_event_mask(&user_ic.ic, 1, 0)
            }
            fn handle_set_focus(
                &mut self,
                _server: &mut S,
                user_ic: &mut UserInputContext<()>,
            ) -> Result<(), ServerError> {
                self.focus = Some((
                    user_ic.ic.client_win(),
                    user_ic.ic.input_method_id().get(),
                    user_ic.ic.input_context_id().get(),
                    user_ic.ic.locale().to_string(),
                ));
                self.active_flag.store(true, Ordering::SeqCst);
                Ok(())
            }
            fn handle_unset_focus(
                &mut self,
                _server: &mut S,
                _user_ic: &mut UserInputContext<()>,
            ) -> Result<(), ServerError> {
                self.focus = None;
                self.active_flag.store(false, Ordering::SeqCst);
                Ok(())
            }
            fn handle_forward_event(
                &mut self,
                _server: &mut S,
                _user_ic: &mut UserInputContext<()>,
                _xev: &x11rb::protocol::xproto::KeyPressEvent,
            ) -> Result<bool, ServerError> {
                Ok(false)
            }
            fn handle_set_ic_values(
                &mut self,
                _server: &mut S,
                _user_ic: &mut UserInputContext<()>,
            ) -> Result<(), ServerError> {
                Ok(())
            }
            fn handle_destroy_ic(
                &mut self,
                _server: &mut S,
                _user_ic: UserInputContext<()>,
            ) -> Result<(), ServerError> {
                self.focus = None;
                self.active_flag.store(false, Ordering::SeqCst);
                Ok(())
            }
            fn handle_reset_ic(
                &mut self,
                _server: &mut S,
                _user_ic: &mut UserInputContext<()>,
            ) -> Result<String, ServerError> {
                Ok(String::new())
            }
        }

        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            use x11rb::connection::Connection;
            let Ok((conn, screen)) =
                x11rb::rust_connection::RustConnection::connect(Some(&format!(
                    ":{display_number}"
                )))
            else {
                return;
            };
            let Ok(mut server) =
                X11rbServer::init(&conn, screen, "mixum", ALL_LOCALES)
            else {
                return;
            };
            let mut connections = XimConnections::new();
            let mut handler = XimHandler {
                active: false,
                focus: None,
                active_flag: active,
            };
            loop {
                while let Ok(text) = rx.try_recv() {
                    if let Some((client, im, ic, locale)) = handler.focus.clone() {
                        let context = InputContext::new(
                            client,
                            std::num::NonZeroU16::new(im).unwrap_or(std::num::NonZeroU16::MIN),
                            std::num::NonZeroU16::new(ic).unwrap_or(std::num::NonZeroU16::MIN),
                            locale,
                        );
                        let _ = server.commit(&context, &text);
                    }
                }
                match conn.poll_for_event() {
                    Ok(Some(event)) => {
                        let _ = server.filter_event(&event, &mut connections, &mut handler);
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
                let _ = conn.flush();
                std::thread::sleep(std::time::Duration::from_millis(4));
            }
        });
        tx
    }

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
        spawn_input_method();

        // XWayland runs as a Wayland client of this compositor; its X11 WM
        // socket is serviced by calloop on the same thread.
        let mut event_loop = calloop::EventLoop::<'static, Art>::try_new()?;
        let loop_handle = event_loop.handle();
        state.loop_handle = Some(loop_handle.clone());
        match XWayland::spawn(
            &display_handle,
            None,
            std::iter::empty::<(String, String)>(),
            true,
            Stdio::null(),
            Stdio::null(),
            |_| (),
        ) {
            Ok((xwayland, client)) => {
                let wm_handle = loop_handle.clone();
                if let Err(error) = loop_handle.insert_source(
                    xwayland,
                    move |event, _, state: &mut Art| match event {
                        XWaylandEvent::Ready {
                            x11_socket,
                            display_number,
                        } => {
                            eprintln!("XWayland ready on :{display_number}");
                            let _ = std::fs::write(
                                "/run/mixum/xwayland.display",
                                format!("{display_number}\n"),
                            );
                            match X11Wm::start_wm(
                                wm_handle.clone(),
                                x11_socket,
                                client.clone(),
                            ) {
                                Ok(wm) => {
                                    eprintln!("X11 WM started");
                                    state.xwm = Some(wm);
                                    state.xwayland_display = Some(display_number);
                                    state.xim_tx = Some(start_xim(
                                        display_number,
                                        state.xim_active.clone(),
                                    ));
                                }
                                Err(error) => eprintln!("X11 WM start: {error}"),
                            }
                        }
                        XWaylandEvent::Error => eprintln!("XWayland failed to start"),
                    },
                ) {
                    eprintln!("XWayland source: {error}");
                }
            }
            Err(error) => eprintln!("XWayland spawn: {error}"),
        }

        loop {
            event_loop.dispatch(Duration::from_millis(0), &mut state)?;
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
