//! Shared client pieces for the standalone viewer and the Tauri host.
//!
//! The TCP wire is owned by `mixum-env-surface`; this module turns it into
//! typed events and writes host input back to the compositor.

use std::{
    io::Write,
    net::TcpStream,
    sync::{mpsc::Sender, Arc, Mutex},
};

use mixum_env_surface::{
    read_message, Frame, InputEvent, SurfaceDestroy, SurfaceInfo, MSG_FRAME, MSG_INPUT,
    MSG_SURFACE_CREATE, MSG_SURFACE_DESTROY,
};

#[derive(Debug)]
pub enum Event {
    Surface(SurfaceInfo),
    Frame(Frame),
    Destroy(u32),
    Disconnected(String),
}

/// Read compositor messages on a background thread and forward typed events.
pub fn spawn_reader(mut stream: TcpStream, tx: Sender<Event>) {
    std::thread::spawn(move || loop {
        match read_message(&mut stream) {
            Ok((MSG_SURFACE_CREATE, payload)) => {
                match serde_json::from_slice::<SurfaceInfo>(&payload) {
                    Ok(info) => {
                        if tx.send(Event::Surface(info)).is_err() {
                            return;
                        }
                    }
                    Err(error) => eprintln!("surface.create: {error}"),
                }
            }
            Ok((MSG_FRAME, payload)) => match Frame::decode(&payload) {
                Ok(frame) => {
                    if tx.send(Event::Frame(frame)).is_err() {
                        return;
                    }
                }
                Err(error) => eprintln!("frame: {error}"),
            },
            Ok((MSG_SURFACE_DESTROY, payload)) => {
                match serde_json::from_slice::<SurfaceDestroy>(&payload) {
                    Ok(destroy) => {
                        if tx.send(Event::Destroy(destroy.id)).is_err() {
                            return;
                        }
                    }
                    Err(error) => eprintln!("surface.destroy: {error}"),
                }
            }
            Ok((kind, _)) => eprintln!("unknown message {kind}"),
            Err(error) => {
                let _ = tx.send(Event::Disconnected(error.to_string()));
                return;
            }
        }
    });
}

/// Serialize one `InputEvent` as a MART input message.
pub fn send_input(writer: &Arc<Mutex<TcpStream>>, event: &InputEvent) {
    if let Ok(payload) = serde_json::to_vec(event) {
        let message = mixum_env_surface::encode_message(MSG_INPUT, &payload);
        if let Ok(mut stream) = writer.lock() {
            let _ = stream.write_all(&message);
        }
    }
}
