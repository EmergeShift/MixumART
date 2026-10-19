//! MixumART B1 host viewer: one native window per remote surface.
//!
//! Connects to the container-side `mixum-env-compositor` over TCP, receives
//! per-surface PNG frames (full + damage patches), composites each surface in
//! its own window, and forwards pointer/keyboard events back to the seat.

use std::{
    collections::{HashMap, HashSet},
    net::TcpStream,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};
use mixum_env_display::{send_input, spawn_reader, Event};
use mixum_env_surface::{InputEvent, SurfaceInfo, FORMAT_NONE};

struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<u32>,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![0x0020_2020; width * height],
        }
    }

    fn clear(&mut self) {
        self.pixels.fill(0x0020_2020);
    }

    fn blit(&mut self, x: usize, y: usize, w: usize, h: usize, image: &[u32]) {
        for row in 0..h {
            let dst_y = y + row;
            if dst_y >= self.height {
                break;
            }
            let dst = dst_y * self.width;
            let src = row * w;
            for col in 0..w {
                let dst_x = x + col;
                if dst_x >= self.width {
                    break;
                }
                self.pixels[dst + dst_x] = image[src + col];
            }
        }
    }
}

struct SurfaceWindow {
    info: SurfaceInfo,
    window: Window,
    canvas: Canvas,
    last_mouse: Option<(f32, f32)>,
    last_buttons: [bool; 3],
    last_keys: HashSet<Key>,
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut addr = "127.0.0.1:7603".to_string();
    while let Some(arg) = args.next() {
        if arg == "--addr" {
            addr = args.next().context("--addr needs a value")?;
        }
    }

    let stream = connect_with_retry(&addr)?;
    stream.set_nodelay(true).ok();
    println!("mixum-art-viewer connected to {addr}");
    let writer = Arc::new(Mutex::new(stream.try_clone()?));

    let (tx, rx) = mpsc::channel();
    spawn_reader(stream, tx);

    let mut surfaces: HashMap<u32, SurfaceWindow> = HashMap::new();
    let mut cascade = 0usize;
    let mut surface_seen = false;

    loop {
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Surface(info) => {
                    if let Some(existing) = surfaces.get_mut(&info.id) {
                        existing.info = info;
                        continue;
                    }
                    surface_seen = true;
                    let title = if info.title.is_empty() {
                        format!("MixumART · surface {}", info.id)
                    } else {
                        format!("MixumART · {}", info.title)
                    };
                    eprintln!(
                        "surface {} {:?} {}x{}",
                        info.id, info.title, info.width, info.height
                    );
                    let mut window = Window::new(
                        &title,
                        info.width.max(1) as usize,
                        info.height.max(1) as usize,
                        WindowOptions {
                            resize: false,
                            ..WindowOptions::default()
                        },
                    )
                    .with_context(|| format!("open window {}x{}", info.width, info.height))?;
                    window.set_target_fps(60);
                    let offset = 40 + (cascade % 6) as isize * 28;
                    window.set_position(offset, offset);
                    cascade += 1;
                    surfaces.insert(
                        info.id,
                        SurfaceWindow {
                            canvas: Canvas::new(info.width as usize, info.height as usize),
                            info,
                            window,
                            last_mouse: None,
                            last_buttons: [false; 3],
                            last_keys: HashSet::new(),
                        },
                    );
                }
                Event::Frame(frame) => {
                    let Some(surface) = surfaces.get_mut(&frame.surface) else {
                        continue;
                    };
                    if frame.format == FORMAT_NONE || frame.data.is_empty() {
                        continue;
                    }
                    let (w, h, image) = decode_png(&frame.data)?;
                    let full = frame.x == 0
                        && frame.y == 0
                        && frame.w == frame.full_w
                        && frame.h == frame.full_h;
                    if full {
                        if surface.canvas.width != frame.full_w as usize
                            || surface.canvas.height != frame.full_h as usize
                        {
                            // minifb windows cannot be resized; keep the canvas
                            // in sync and let the next window creation pick it up.
                            surface.canvas =
                                Canvas::new(frame.full_w as usize, frame.full_h as usize);
                        }
                        surface.canvas.clear();
                    }
                    surface
                        .canvas
                        .blit(frame.x as usize, frame.y as usize, w, h, &image);
                }
                Event::Destroy(id) => {
                    surfaces.remove(&id);
                }
                Event::Disconnected(reason) => bail!("compositor disconnected: {reason}"),
            }
        }

        let mut closed = Vec::new();
        for (id, surface) in surfaces.iter_mut() {
            if !surface.window.is_open() || surface.window.is_key_down(Key::Escape) {
                closed.push(*id);
                continue;
            }
            pump_input(surface, &writer);
            surface.window.update_with_buffer(
                &surface.canvas.pixels,
                surface.canvas.width,
                surface.canvas.height,
            )?;
        }
        for id in closed {
            eprintln!("window closed for surface {id}");
            surfaces.remove(&id);
        }
        if surface_seen && surfaces.is_empty() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn pump_input(surface: &mut SurfaceWindow, writer: &Arc<Mutex<TcpStream>>) {
    let surface_id = surface.info.id;
    if let Some(position) = surface.window.get_mouse_pos(MouseMode::Discard) {
        if surface.last_mouse != Some(position) {
            surface.last_mouse = Some(position);
            send_input(
                writer,
                &InputEvent::PointerMotion {
                    surface: surface_id,
                    x: position.0 as f64,
                    y: position.1 as f64,
                },
            );
        }
    }
    for (index, button) in [MouseButton::Left, MouseButton::Middle, MouseButton::Right]
        .into_iter()
        .enumerate()
    {
        let down = surface.window.get_mouse_down(button);
        if down != surface.last_buttons[index] {
            surface.last_buttons[index] = down;
            send_input(
                writer,
                &InputEvent::PointerButton {
                    surface: surface_id,
                    button: match button {
                        MouseButton::Right => "right".into(),
                        MouseButton::Middle => "middle".into(),
                        MouseButton::Left => "left".into(),
                    },
                    action: if down { "press" } else { "release" }.into(),
                },
            );
        }
    }
    let pressed: HashSet<Key> = surface.window.get_keys().into_iter().collect();
    for key in pressed.difference(&surface.last_keys) {
        if let Some(name) = key_name(*key) {
            send_input(
                writer,
                &InputEvent::Key {
                    surface: surface_id,
                    keysym: name.into(),
                    action: "press".into(),
                },
            );
        }
    }
    for key in surface.last_keys.difference(&pressed) {
        if let Some(name) = key_name(*key) {
            send_input(
                writer,
                &InputEvent::Key {
                    surface: surface_id,
                    keysym: name.into(),
                    action: "release".into(),
                },
            );
        }
    }
    surface.last_keys = pressed;
}

fn key_name(key: Key) -> Option<&'static str> {
    Some(match key {
        Key::A => "a",
        Key::B => "b",
        Key::C => "c",
        Key::D => "d",
        Key::E => "e",
        Key::F => "f",
        Key::G => "g",
        Key::H => "h",
        Key::I => "i",
        Key::J => "j",
        Key::K => "k",
        Key::L => "l",
        Key::M => "m",
        Key::N => "n",
        Key::O => "o",
        Key::P => "p",
        Key::Q => "q",
        Key::R => "r",
        Key::S => "s",
        Key::T => "t",
        Key::U => "u",
        Key::V => "v",
        Key::W => "w",
        Key::X => "x",
        Key::Y => "y",
        Key::Z => "z",
        Key::Key0 => "0",
        Key::Key1 => "1",
        Key::Key2 => "2",
        Key::Key3 => "3",
        Key::Key4 => "4",
        Key::Key5 => "5",
        Key::Key6 => "6",
        Key::Key7 => "7",
        Key::Key8 => "8",
        Key::Key9 => "9",
        Key::Enter => "Return",
        Key::Backspace => "BackSpace",
        Key::Space => "space",
        Key::Tab => "Tab",
        Key::Escape => "Escape",
        Key::Left => "Left",
        Key::Right => "Right",
        Key::Up => "Up",
        Key::Down => "Down",
        Key::Minus => "minus",
        Key::Equal => "equal",
        Key::Comma => "comma",
        Key::Period => "period",
        Key::Slash => "slash",
        Key::Semicolon => "semicolon",
        Key::Apostrophe => "apostrophe",
        _ => return None,
    })
}

fn connect_with_retry(addr: &str) -> Result<TcpStream> {
    let mut last = None;
    for attempt in 0..50 {
        match TcpStream::connect(addr) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                last = Some(error);
                if attempt == 0 {
                    eprintln!("waiting for compositor at {addr}…");
                }
                thread::sleep(Duration::from_millis(200));
            }
        }
    }
    bail!(
        "cannot connect to {addr}: {}",
        last.map(|e| e.to_string()).unwrap_or_default()
    )
}

fn decode_png(data: &[u8]) -> Result<(usize, usize, Vec<u32>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer)?;
    let width = info.width as usize;
    let height = info.height as usize;
    let bytes = &buffer[..info.buffer_size()];
    let mut pixels = vec![0u32; width * height];
    match info.color_type {
        png::ColorType::Rgba => {
            for (index, pixel) in bytes.chunks_exact(4).enumerate() {
                pixels[index] =
                    ((pixel[0] as u32) << 16) | ((pixel[1] as u32) << 8) | pixel[2] as u32;
            }
        }
        png::ColorType::Rgb => {
            for (index, pixel) in bytes.chunks_exact(3).enumerate() {
                pixels[index] =
                    ((pixel[0] as u32) << 16) | ((pixel[1] as u32) << 8) | pixel[2] as u32;
            }
        }
        other => bail!("unsupported PNG color type {other:?}"),
    }
    Ok((width, height, pixels))
}
