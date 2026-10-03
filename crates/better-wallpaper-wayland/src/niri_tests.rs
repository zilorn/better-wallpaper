//! Minimal Wayland peer for exercising lifecycle events without a desktop/GPU.
//! Surfaces deliberately remain unconfigured, so no EGL setup is necessary.
use super::*;
use std::{
    collections::HashMap,
    io::{Read, Write},
    os::unix::net::UnixStream,
    sync::mpsc,
    thread,
};

enum Command {
    AddOutput(u32, &'static str),
    RemoveOutput(u32),
    CloseLayers,
    Disconnect,
}

struct Peer {
    commands: mpsc::Sender<Command>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Disconnect);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn uint(value: u32) -> Vec<u8> {
    value.to_ne_bytes().to_vec()
}

fn string(value: &str) -> Vec<u8> {
    let mut bytes = uint(value.len() as u32 + 1);
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

fn send(stream: &mut UnixStream, object: u32, opcode: u16, args: &[u8]) {
    let mut bytes = uint(object);
    bytes.extend(uint(((args.len() as u32 + 8) << 16) | opcode as u32));
    bytes.extend(args);
    stream.write_all(&bytes).unwrap();
}

fn global(stream: &mut UnixStream, registry: u32, id: u32, interface: &str, version: u32) {
    let mut args = uint(id);
    args.extend(string(interface));
    args.extend(uint(version));
    send(stream, registry, 0, &args);
}

fn peer(target: Option<&str>) -> (Peer, NiriBackend) {
    let (client, mut server) = UnixStream::pair().unwrap();
    server.set_nonblocking(true).unwrap();
    let (commands, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut registry = 0;
        let mut objects = HashMap::new();
        let mut outputs = HashMap::new();
        let mut layers = Vec::new();
        let mut pending = Vec::new();
        loop {
            while let Ok(command) = rx.try_recv() {
                match command {
                    Command::AddOutput(id, name) => {
                        outputs.insert(id, name);
                        global(&mut server, registry, id, "wl_output", 4);
                    }
                    Command::RemoveOutput(id) => {
                        outputs.remove(&id);
                        send(&mut server, registry, 1, &uint(id));
                    }
                    Command::CloseLayers => {
                        for layer in &layers {
                            send(&mut server, *layer, 1, &[]);
                        }
                    }
                    Command::Disconnect => return,
                }
            }
            let mut bytes = [0; 4096];
            match server.read(&mut bytes) {
                Ok(0) => return,
                Ok(count) => pending.extend_from_slice(&bytes[..count]),
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(error) => panic!("mock Wayland read failed: {error}"),
            }
            while pending.len() >= 8 {
                let object = u32::from_ne_bytes(pending[..4].try_into().unwrap());
                let header = u32::from_ne_bytes(pending[4..8].try_into().unwrap());
                let size = (header >> 16) as usize;
                if pending.len() < size {
                    break;
                }
                let opcode = header as u16;
                let args = pending[8..size].to_vec();
                pending.drain(..size);
                let word = |offset: usize| {
                    u32::from_ne_bytes(args[offset..offset + 4].try_into().unwrap())
                };
                match (object, objects.get(&object).copied(), opcode) {
                    (1, _, 1) => {
                        registry = word(0);
                        global(&mut server, registry, 1, "wl_compositor", 4);
                        global(&mut server, registry, 2, "wl_shm", 1);
                        global(&mut server, registry, 3, "zwlr_layer_shell_v1", 4);
                    }
                    (1, _, 0) => {
                        send(&mut server, word(0), 0, &uint(0));
                        send(&mut server, 1, 1, &uint(word(0)));
                    }
                    (_, _, 0) if object == registry => {
                        let global_id = word(0);
                        let interface_size = word(4) as usize;
                        let new_id = word(12 + interface_size.div_ceil(4) * 4);
                        objects.insert(new_id, global_id);
                        if let Some(name) = outputs.get(&global_id) {
                            send(&mut server, new_id, 4, &string(name));
                            send(&mut server, new_id, 2, &[]);
                        } else if global_id == 2 {
                            send(&mut server, new_id, 0, &uint(0));
                        }
                    }
                    (_, Some(1), 0) => {
                        objects.insert(word(0), 100);
                    }
                    (_, Some(3), 0) => {
                        layers.push(word(0));
                        objects.insert(word(0), 101);
                    }
                    (_, Some(101), 7) => {
                        layers.retain(|layer| *layer != object);
                    }
                    _ => {}
                }
            }
        }
    });
    let backend =
        NiriBackend::connect_with(Connection::from_socket(client).unwrap(), target).unwrap();
    (
        Peer {
            commands,
            thread: Some(handle),
        },
        backend,
    )
}

fn pump_until(backend: &mut NiriBackend, predicate: impl Fn(&NiriBackend) -> bool) {
    let started = Instant::now();
    while !predicate(backend) {
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "Wayland lifecycle timed out"
        );
        backend.dispatch_pending().unwrap();
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn missing_target_waits_and_hotplug_creates_and_releases_only_matching_surface() {
    let (peer, mut backend) = peer(Some("DP-1"));
    assert!(backend.surface.is_none());
    assert!(
        backend
            .present(&super::tests::frame(2, 2), FillMode::Cover)
            .unwrap()
            .is_none()
    );
    peer.commands
        .send(Command::AddOutput(10, "HDMI-A-1"))
        .unwrap();
    pump_until(&mut backend, |b| {
        b.state.output_state.outputs().count() == 1
    });
    assert!(backend.surface.is_none());
    peer.commands.send(Command::AddOutput(11, "DP-1")).unwrap();
    pump_until(&mut backend, |b| b.surface.is_some());
    assert_eq!(backend.output_name(), "DP-1");
    peer.commands.send(Command::RemoveOutput(11)).unwrap();
    pump_until(&mut backend, |b| b.surface.is_none());
    peer.commands.send(Command::AddOutput(12, "DP-1")).unwrap();
    pump_until(&mut backend, |b| b.surface.is_some());
    assert_eq!(backend.output_name(), "DP-1");
}

#[test]
fn auto_selection_preserves_current_output_and_moves_after_removal() {
    let (peer, mut backend) = peer(None);
    peer.commands.send(Command::AddOutput(10, "DP-1")).unwrap();
    pump_until(&mut backend, |b| b.surface.is_some());
    peer.commands
        .send(Command::AddOutput(11, "HDMI-A-1"))
        .unwrap();
    pump_until(&mut backend, |b| {
        b.state.output_state.outputs().count() == 2
    });
    assert_eq!(backend.output_name(), "DP-1");
    peer.commands.send(Command::RemoveOutput(10)).unwrap();
    pump_until(&mut backend, |b| b.output_name() == "HDMI-A-1");
}

#[test]
fn closed_layer_is_released_and_recreated_after_backoff() {
    let (peer, mut backend) = peer(Some("DP-1"));
    peer.commands.send(Command::AddOutput(10, "DP-1")).unwrap();
    pump_until(&mut backend, |b| b.surface.is_some());
    // Flush creation before asking the peer to close the new layer.
    backend.event_queue.roundtrip(&mut backend.state).unwrap();
    peer.commands.send(Command::CloseLayers).unwrap();
    pump_until(&mut backend, |b| b.surface.is_none());
    assert!(backend.next_surface_attempt > Instant::now());
    backend.dispatch_pending().unwrap();
    assert!(backend.surface.is_none());
    backend.next_surface_attempt = Instant::now();
    backend.dispatch_pending().unwrap();
    assert!(backend.surface.is_some());
}

#[test]
fn socket_disconnect_is_detected_without_a_frame_callback() {
    let (peer, mut backend) = peer(Some("missing"));
    peer.commands.send(Command::Disconnect).unwrap();
    let started = Instant::now();
    while backend.dispatch_pending().is_ok() {
        assert!(started.elapsed() < Duration::from_secs(2));
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn resize_marks_repaint_and_stale_surface_events_are_ignored() {
    let (peer, mut backend) = peer(Some("DP-1"));
    peer.commands.send(Command::AddOutput(10, "DP-1")).unwrap();
    pump_until(&mut backend, |b| b.surface.is_some());
    let old_layer = backend.surface.as_ref().unwrap().layer.clone();
    peer.commands.send(Command::RemoveOutput(10)).unwrap();
    pump_until(&mut backend, |b| b.surface.is_none());
    peer.commands.send(Command::AddOutput(11, "DP-1")).unwrap();
    pump_until(&mut backend, |b| b.surface.is_some());
    let layer = backend.surface.as_ref().unwrap().layer.clone();
    let qh = backend.event_queue.handle();
    backend.state.frame_ready = false;
    backend
        .state
        .frame(&backend.connection, &qh, old_layer.wl_surface(), 0);
    backend.state.closed(&backend.connection, &qh, &old_layer);
    backend.state.configure_surface(&old_layer, (640, 480), 1);
    assert!(!backend.state.frame_ready);
    assert!(!backend.state.closed);
    assert_eq!(backend.state.configured_size, None);
    backend.state.configure_surface(&layer, (1920, 1080), 2);
    assert_eq!(backend.size(), (1920, 1080));
    backend.state.needs_redraw = false;
    backend.state.configure_surface(&layer, (2560, 1440), 3);
    assert_eq!(backend.size(), (2560, 1440));
    assert!(backend.state.needs_redraw);
}
