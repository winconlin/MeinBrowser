//! Eigener X11-Client: Fenster, Eingabe und Bildausgabe über das
//! Kernprotokoll, ohne `libX11` und `libxcb`.
//!
//! Die Verbindung läuft über einen Unix-Socket oder TCP aus der
//! Standardbibliothek. Bilder gehen mit `PutImage` an den Server, in Streifen,
//! die in eine Anfrage passen. Unter Wayland läuft das Fenster über XWayland;
//! ein eigener Wayland-Client folgt später.

#![forbid(unsafe_code)]

mod display;
mod keysym;
mod proto;

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::event::{Event, KeyEvent, PointerButton, ScrollDelta};
use crate::{Capture, Error, WindowOptions};

use display::{Address, AuthAddress};
use keysym::Keymap;
use proto::{ByteOrder, RawEvent, Setup, XError};

/// Verbindung zum X-Server über Unix-Socket oder TCP.
enum Stream {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Stream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Stream::Unix(stream) => stream.set_read_timeout(timeout),
            Stream::Tcp(stream) => stream.set_read_timeout(timeout),
        }
    }

    fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        match self {
            Stream::Unix(stream) => stream.set_nonblocking(nonblocking),
            Stream::Tcp(stream) => stream.set_nonblocking(nonblocking),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Unix(stream) => stream.read(buffer),
            Stream::Tcp(stream) => stream.read(buffer),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Unix(stream) => stream.write(buffer),
            Stream::Tcp(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Unix(stream) => stream.flush(),
            Stream::Tcp(stream) => stream.flush(),
        }
    }
}

/// Öffnet die Verbindung und nennt die Adressen, unter denen der Client in
/// der Xauthority-Datei stehen kann.
fn connect(address: &Address) -> Result<(Stream, Vec<AuthAddress>), Error> {
    let hostname = crate::unix::hostname().unwrap_or_default();
    match address {
        Address::Unix(path) => {
            let stream = connect_unix(path)?;
            Ok((Stream::Unix(stream), vec![AuthAddress::local(&hostname)]))
        }
        Address::Tcp { host, port } => {
            let stream = TcpStream::connect((host.as_str(), *port))?;
            stream.set_nodelay(true)?;
            let peer = stream.peer_addr()?.ip();
            let mut addresses = vec![AuthAddress::ip(peer)];
            if peer.is_loopback() {
                addresses.push(AuthAddress::local(&hostname));
            }
            Ok((Stream::Tcp(stream), addresses))
        }
    }
}

fn connect_unix(path: &PathBuf) -> Result<UnixStream, Error> {
    // Linux-Server lauschen zusätzlich auf einem abstrakten Socket gleichen
    // Namens. Den probiert auch libxcb zuerst.
    #[cfg(target_os = "linux")]
    {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::net::SocketAddr;
        if let Ok(address) = SocketAddr::from_abstract_name(path.as_os_str().as_bytes())
            && let Ok(stream) = UnixStream::connect_addr(&address)
        {
            return Ok(stream);
        }
    }
    UnixStream::connect(path).map_err(|err| {
        Error::Io(io::Error::new(
            err.kind(),
            format!("X-Server unter {} nicht erreichbar: {err}", path.display()),
        ))
    })
}

/// Name und Daten der Anmeldung aus `$XAUTHORITY` oder `~/.Xauthority`.
/// Ohne passenden Eintrag verbindet sich der Client ohne Anmeldung.
fn authorization(number: u16, addresses: &[AuthAddress]) -> (Vec<u8>, Vec<u8>) {
    let path = std::env::var_os("XAUTHORITY")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".Xauthority")));
    let Some(bytes) = path.and_then(|path| std::fs::read(path).ok()) else {
        return (Vec::new(), Vec::new());
    };
    let entries = display::parse_xauthority(&bytes);
    match display::find_cookie(&entries, addresses, number) {
        Some(entry) => (entry.name.clone(), entry.data.clone()),
        None => (Vec::new(), Vec::new()),
    }
}

/// Eine offene Verbindung mit Puffern für Anfragen und Antworten.
struct Connection {
    stream: Stream,
    setup: Setup,
    output: Vec<u8>,
    input: Vec<u8>,
    /// Sequenznummer der zuletzt gesendeten Anfrage.
    sequence: u16,
    next_resource: u32,
    resource_increment: u32,
    /// Ereignisse und Fehler, die beim Warten auf eine Antwort ankamen.
    queued: VecDeque<Vec<u8>>,
}

impl Connection {
    fn open() -> Result<(Connection, u16), Error> {
        let Some(name) = std::env::var_os("DISPLAY") else {
            return Err(Error::Unsupported(
                "kein X-Server gefunden, DISPLAY ist nicht gesetzt".into(),
            ));
        };
        let name = name
            .to_str()
            .ok_or_else(|| Error::InvalidArgument("DISPLAY ist kein gültiges UTF-8".into()))?;
        let display = display::parse_display(name)?;
        let (mut stream, addresses) = connect(&display.address)?;
        let (auth_name, auth_data) = authorization(display.number, &addresses);
        stream.write_all(&proto::setup_request(&auth_name, &auth_data)?)?;
        let mut header = [0u8; 8];
        stream.read_exact(&mut header)?;
        let (status, reason_length, length) = proto::setup_reply_header(&header)?;
        let mut body = vec![0; length];
        stream.read_exact(&mut body)?;
        let setup = proto::parse_setup(status, reason_length, &body)?;
        let mask = setup.resource_id_mask;
        if mask == 0 {
            return Err(Error::Protocol(
                "X-Server vergibt keine Ressourcennummern".into(),
            ));
        }
        // Das niedrigste gesetzte Bit der Maske ist die Schrittweite.
        let increment = mask.isolate_lowest_one();
        Ok((
            Connection {
                stream,
                setup,
                output: Vec::new(),
                input: Vec::new(),
                sequence: 0,
                next_resource: increment,
                resource_increment: increment,
                queued: VecDeque::new(),
            },
            display.screen,
        ))
    }

    fn generate_id(&mut self) -> Result<u32, Error> {
        let part = self.next_resource;
        if part == 0 || part & !self.setup.resource_id_mask != 0 {
            return Err(Error::Protocol(
                "keine freien X11-Ressourcennummern mehr".into(),
            ));
        }
        self.next_resource = part.checked_add(self.resource_increment).unwrap_or(0);
        Ok(self.setup.resource_id_base | part)
    }

    /// Reiht eine Anfrage ein und gibt ihre Sequenznummer zurück.
    fn send(&mut self, request: Vec<u8>) -> u16 {
        self.output.extend_from_slice(&request);
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }

    fn flush(&mut self) -> Result<(), Error> {
        if !self.output.is_empty() {
            self.stream.write_all(&self.output)?;
            self.output.clear();
        }
        Ok(())
    }

    /// Nimmt ein vollständiges Paket aus dem Eingangspuffer.
    fn take_packet(&mut self) -> Option<Vec<u8>> {
        if self.input.is_empty() {
            return None;
        }
        let length = proto::packet_length(&self.input)?;
        if self.input.len() < length {
            return None;
        }
        Some(self.input.drain(..length).collect())
    }

    /// Liest weitere Daten. `Ok(false)` heißt: Frist abgelaufen. Nach Ablauf
    /// der Frist wird noch gelesen, was schon vorliegt, damit auch Abfragen
    /// mit der Frist null Ereignisse bekommen.
    fn fill(&mut self, deadline: Option<Instant>) -> Result<bool, Error> {
        let timeout = match deadline {
            None => None,
            Some(deadline) => {
                let now = Instant::now();
                if now >= deadline {
                    return self.fill_available();
                }
                // Eine Frist von null lehnt die Standardbibliothek ab.
                Some((deadline - now).max(Duration::from_millis(1)))
            }
        };
        self.stream.set_read_timeout(timeout)?;
        self.read_some()
    }

    /// Liest, was gerade vorliegt, ohne zu warten. `Ok(false)` heißt: nichts da.
    fn fill_available(&mut self) -> Result<bool, Error> {
        self.stream.set_nonblocking(true)?;
        let result = self.read_some();
        self.stream.set_nonblocking(false)?;
        result
    }

    /// Ein Lesevorgang. `Ok(false)` heißt: Frist abgelaufen oder nichts da.
    fn read_some(&mut self) -> Result<bool, Error> {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            match self.stream.read(&mut buffer) {
                Ok(0) => {
                    return Err(Error::Protocol(
                        "X-Server hat die Verbindung beendet".into(),
                    ));
                }
                Ok(count) => {
                    self.input
                        .extend_from_slice(buffer.get(..count).unwrap_or(&[]));
                    return Ok(true);
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(false);
                }
                Err(err) => return Err(err.into()),
            }
        }
    }

    /// Wartet auf die Antwort zu einer Anfrage.
    fn reply(&mut self, sequence: u16) -> Result<Vec<u8>, Error> {
        self.flush()?;
        loop {
            let Some(packet) = self.take_packet() else {
                self.fill(None)?;
                continue;
            };
            match packet.first().map(|kind| kind & 0x7f) {
                Some(0) if proto::packet_sequence(&packet) == Some(sequence) => {
                    return Err(XError::parse(&packet)?.into_error());
                }
                Some(1) if proto::packet_sequence(&packet) == Some(sequence) => {
                    return Ok(packet);
                }
                // Antwort auf eine frühere Anfrage, auf die niemand mehr wartet.
                Some(1) => {}
                _ => self.queued.push_back(packet),
            }
        }
    }

    /// Das nächste Ereignis oder ein Fehler, oder `None`, wenn die Frist
    /// abgelaufen ist.
    fn next_packet(&mut self, deadline: Option<Instant>) -> Result<Option<Vec<u8>>, Error> {
        if let Some(packet) = self.queued.pop_front() {
            return Ok(Some(packet));
        }
        self.flush()?;
        loop {
            if let Some(packet) = self.take_packet() {
                if packet.first().map(|kind| kind & 0x7f) == Some(1) {
                    continue;
                }
                return Ok(Some(packet));
            }
            if !self.fill(deadline)? {
                return Ok(None);
            }
        }
    }

    /// Das nächste Paket, falls schon eines da ist, ohne es zu entnehmen.
    fn peek_packet(&mut self) -> Result<Option<&[u8]>, Error> {
        if self.queued.is_empty() && self.take_packet_into_queue().is_none() {
            self.fill_available()?;
            self.take_packet_into_queue();
        }
        Ok(self.queued.front().map(Vec::as_slice))
    }

    fn take_packet_into_queue(&mut self) -> Option<()> {
        loop {
            let packet = self.take_packet()?;
            if packet.first().map(|kind| kind & 0x7f) != Some(1) {
                self.queued.push_back(packet);
                return Some(());
            }
        }
    }
}

/// Wie die Kanäle eines Pixels beim Server angeordnet sind.
#[derive(Clone, Copy, Debug)]
struct PixelFormat {
    depth: u8,
    red_shift: u32,
    green_shift: u32,
    blue_shift: u32,
    byte_order: ByteOrder,
}

impl PixelFormat {
    fn new(setup: &Setup, screen: &proto::Screen) -> Result<PixelFormat, Error> {
        let (depth, visual) = screen
            .visual(screen.root_visual)
            .ok_or_else(|| Error::Protocol("Visual des Bildschirms fehlt".into()))?;
        if visual.class != proto::VISUAL_CLASS_TRUE_COLOR
            && visual.class != proto::VISUAL_CLASS_DIRECT_COLOR
        {
            return Err(Error::Unsupported(format!(
                "X11-Visual der Klasse {} (nur TrueColor und DirectColor)",
                visual.class
            )));
        }
        let bits_per_pixel = setup
            .formats
            .iter()
            .find(|format| format.depth == depth)
            .map(|format| format.bits_per_pixel);
        if bits_per_pixel != Some(32) {
            return Err(Error::Unsupported(format!(
                "X11-Bildformat mit Tiefe {depth} und {bits_per_pixel:?} Bit je Pixel (nur 32)"
            )));
        }
        let shift = |mask: u32| {
            let shift = mask.trailing_zeros();
            if mask.checked_shr(shift) == Some(0xff) {
                Ok(shift)
            } else {
                Err(Error::Unsupported(format!("X11-Farbmaske {mask:#x}")))
            }
        };
        Ok(PixelFormat {
            depth,
            red_shift: shift(visual.red_mask)?,
            green_shift: shift(visual.green_mask)?,
            blue_shift: shift(visual.blue_mask)?,
            byte_order: setup.image_byte_order,
        })
    }

    fn encode(&self, pixel: u32, out: &mut Vec<u8>) {
        let r = (pixel >> 16) & 0xff;
        let g = (pixel >> 8) & 0xff;
        let b = pixel & 0xff;
        let value = (r << self.red_shift) | (g << self.green_shift) | (b << self.blue_shift);
        match self.byte_order {
            ByteOrder::LsbFirst => out.extend_from_slice(&value.to_le_bytes()),
            ByteOrder::MsbFirst => out.extend_from_slice(&value.to_be_bytes()),
        }
    }

    fn decode(&self, bytes: [u8; 4]) -> u32 {
        let value = match self.byte_order {
            ByteOrder::LsbFirst => u32::from_le_bytes(bytes),
            ByteOrder::MsbFirst => u32::from_be_bytes(bytes),
        };
        let r = (value >> self.red_shift) & 0xff;
        let g = (value >> self.green_shift) & 0xff;
        let b = (value >> self.blue_shift) & 0xff;
        (r << 16) | (g << 8) | b
    }
}

pub struct Window {
    connection: Connection,
    window: u32,
    gc: u32,
    format: PixelFormat,
    wm_protocols: u32,
    wm_delete_window: u32,
    keymap: Keymap,
    width: u32,
    height: u32,
    focused: Option<bool>,
    /// Tastencode, dessen nächstes Drücken eine automatische Wiederholung ist.
    repeating: Option<u8>,
    pending: VecDeque<Event>,
}

impl Window {
    pub fn open(options: &WindowOptions<'_>) -> Result<Window, Error> {
        let (mut connection, screen_number) = Connection::open()?;
        let screen = connection
            .setup
            .screens
            .get(usize::from(screen_number))
            .cloned()
            .ok_or_else(|| {
                Error::InvalidArgument(format!("X11-Bildschirm {screen_number} gibt es nicht"))
            })?;
        let format = PixelFormat::new(&connection.setup, &screen)?;
        let too_large = || {
            Error::InvalidArgument(format!(
                "Fenstergröße {}×{} ist für X11 zu groß",
                options.width, options.height
            ))
        };
        let width = u16::try_from(options.width).map_err(|_| too_large())?;
        let height = u16::try_from(options.height).map_err(|_| too_large())?;

        let atom_names = [
            "WM_PROTOCOLS",
            "WM_DELETE_WINDOW",
            "_NET_WM_NAME",
            "UTF8_STRING",
        ];
        let mut atom_requests = Vec::with_capacity(atom_names.len());
        for name in atom_names {
            atom_requests.push(connection.send(proto::intern_atom(name)?));
        }
        let min_keycode = connection.setup.min_keycode;
        let keycode_count = u8::try_from(
            u16::from(connection.setup.max_keycode)
                .saturating_sub(u16::from(min_keycode))
                .saturating_add(1),
        )
        .unwrap_or(u8::MAX);
        let keyboard_request =
            connection.send(proto::get_keyboard_mapping(min_keycode, keycode_count)?);
        let modifier_request = connection.send(proto::get_modifier_mapping()?);

        let mut atoms = Vec::with_capacity(atom_requests.len());
        for sequence in atom_requests {
            let reply = connection.reply(sequence)?;
            atoms.push(proto::Reader::new(reply.get(8..).unwrap_or(&[])).u32()?);
        }
        let [wm_protocols, wm_delete_window, net_wm_name, utf8_string] = atoms[..] else {
            return Err(Error::Protocol("Atome fehlen".into()));
        };
        let mut keymap = parse_keyboard_mapping(min_keycode, &connection.reply(keyboard_request)?)?;
        apply_modifier_mapping(&mut keymap, &connection.reply(modifier_request)?)?;

        let window = connection.generate_id()?;
        let gc = connection.generate_id()?;
        connection.send(proto::create_window(
            window,
            screen.root,
            format.depth,
            screen.root_visual,
            width,
            height,
            &proto::WindowValues {
                background_pixel: screen.black_pixel,
                bit_gravity: proto::BIT_GRAVITY_NORTH_WEST,
                event_mask: proto::EVENT_MASK_KEY_PRESS
                    | proto::EVENT_MASK_KEY_RELEASE
                    | proto::EVENT_MASK_BUTTON_PRESS
                    | proto::EVENT_MASK_BUTTON_RELEASE
                    | proto::EVENT_MASK_ENTER_WINDOW
                    | proto::EVENT_MASK_LEAVE_WINDOW
                    | proto::EVENT_MASK_POINTER_MOTION
                    | proto::EVENT_MASK_EXPOSURE
                    | proto::EVENT_MASK_STRUCTURE_NOTIFY
                    | proto::EVENT_MASK_FOCUS_CHANGE,
            },
        )?);
        let latin1: Vec<u8> = options
            .title
            .chars()
            .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
            .collect();
        connection.send(proto::change_property(
            window,
            proto::ATOM_WM_NAME,
            proto::ATOM_STRING,
            8,
            &latin1,
        )?);
        connection.send(proto::change_property(
            window,
            net_wm_name,
            utf8_string,
            8,
            options.title.as_bytes(),
        )?);
        connection.send(proto::change_property(
            window,
            proto::ATOM_WM_CLASS,
            proto::ATOM_STRING,
            8,
            b"meinbrowser\0MeinBrowser\0",
        )?);
        connection.send(proto::change_property(
            window,
            wm_protocols,
            proto::ATOM_ATOM,
            32,
            &wm_delete_window.to_ne_bytes(),
        )?);
        connection.send(proto::create_gc(gc, window)?);
        connection.send(proto::map_window(window)?);
        // Ein Rundgang stellt sicher, dass alle Anfragen angenommen wurden.
        let sync = connection.send(proto::get_input_focus()?);
        connection.reply(sync)?;

        let mut pending = VecDeque::new();
        pending.push_back(Event::Resized {
            width: options.width,
            height: options.height,
        });
        Ok(Window {
            connection,
            window,
            gc,
            format,
            wm_protocols,
            wm_delete_window,
            keymap,
            width: options.width,
            height: options.height,
            focused: None,
            repeating: None,
            pending,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn present(&mut self, width: u32, height: u32, pixels: &[u32]) -> Result<(), Error> {
        let too_large =
            || Error::InvalidArgument(format!("Bild {width}×{height} ist für X11 zu groß"));
        let width16 = u16::try_from(width).map_err(|_| too_large())?;
        u16::try_from(height).map_err(|_| too_large())?;
        let row_bytes = usize::from(width16) * 4;
        let max_bytes = usize::from(self.connection.setup.max_request_length) * 4;
        let rows_per_request = max_bytes.saturating_sub(proto::PUT_IMAGE_HEADER) / row_bytes;
        if rows_per_request == 0 {
            return Err(too_large());
        }
        let mut data = Vec::with_capacity(rows_per_request.saturating_mul(row_bytes));
        for (index, rows) in pixels
            .chunks(rows_per_request * usize::from(width16))
            .enumerate()
        {
            data.clear();
            for &pixel in rows {
                self.format.encode(pixel, &mut data);
            }
            let row_count = rows.len() / usize::from(width16);
            let y = index * rows_per_request;
            let request = proto::put_image(
                self.window,
                self.gc,
                width16,
                u16::try_from(row_count).map_err(|_| too_large())?,
                0,
                // Zeilen oberhalb von 32767 liegen ohnehin außerhalb jedes Fensters.
                i16::try_from(y).unwrap_or(i16::MAX),
                self.format.depth,
                &data,
            )?;
            self.connection.send(request);
        }
        self.connection.flush()
    }

    pub fn next_event(&mut self, timeout: Option<Duration>) -> Result<Option<Event>, Error> {
        let deadline = timeout.and_then(|timeout| Instant::now().checked_add(timeout));
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }
            let Some(packet) = self.connection.next_packet(deadline)? else {
                return Ok(None);
            };
            self.translate(&packet)?;
        }
    }

    pub fn capture(&mut self) -> Result<Capture, Error> {
        let width = u16::try_from(self.width).unwrap_or(u16::MAX);
        let height = u16::try_from(self.height).unwrap_or(u16::MAX);
        let sequence = self
            .connection
            .send(proto::get_image(self.window, 0, 0, width, height)?);
        let reply = self.connection.reply(sequence)?;
        let data = reply.get(proto::PACKET_SIZE..).unwrap_or(&[]);
        let count = usize::from(width) * usize::from(height);
        if data.len() < count * 4 {
            return Err(Error::Protocol(format!(
                "GetImage lieferte {} statt {} Bytes",
                data.len(),
                count * 4
            )));
        }
        let pixels = data
            .as_chunks::<4>()
            .0
            .iter()
            .take(count)
            .map(|&bytes| self.format.decode(bytes))
            .collect();
        Ok(Capture {
            width: u32::from(width),
            height: u32::from(height),
            pixels,
        })
    }

    fn translate(&mut self, packet: &[u8]) -> Result<(), Error> {
        if packet.first().map(|kind| kind & 0x7f) == Some(0) {
            return Err(XError::parse(packet)?.into_error());
        }
        match RawEvent::parse(packet)? {
            RawEvent::Key {
                pressed,
                keycode,
                time,
                window,
                state,
            } if window == self.window => {
                if !pressed && self.next_is_repeat_of(keycode, time)? {
                    self.repeating = Some(keycode);
                    return Ok(());
                }
                let repeat = pressed && self.repeating == Some(keycode);
                if pressed {
                    self.repeating = None;
                }
                let keysym = self.keymap.lookup(keycode, state);
                let modifiers = self.keymap.modifiers(state);
                self.pending.push_back(Event::Key(KeyEvent {
                    key: keysym::key_from_keysym(keysym),
                    pressed,
                    repeat,
                    modifiers,
                }));
                if pressed
                    && !modifiers.control
                    && !modifiers.alt
                    && let Some(c) = keysym::char_from_keysym(keysym)
                {
                    self.pending.push_back(Event::Text(c.to_string()));
                }
            }
            RawEvent::Button {
                pressed,
                button,
                window,
                x,
                y,
                ..
            } if window == self.window => {
                let (x, y) = (f64::from(x), f64::from(y));
                let scroll = match button {
                    4 => Some((0.0, -1.0)),
                    5 => Some((0.0, 1.0)),
                    6 => Some((-1.0, 0.0)),
                    7 => Some((1.0, 0.0)),
                    _ => None,
                };
                if let Some((dx, dy)) = scroll {
                    // Das Rad meldet Drücken und Loslassen; gezählt wird einmal.
                    if pressed {
                        self.pending
                            .push_back(Event::Scroll(ScrollDelta::Steps { x: dx, y: dy }));
                    }
                } else {
                    let button = match button {
                        1 => PointerButton::Left,
                        2 => PointerButton::Middle,
                        3 => PointerButton::Right,
                        8 => PointerButton::Back,
                        9 => PointerButton::Forward,
                        other => PointerButton::Other(u16::from(other)),
                    };
                    self.pending.push_back(Event::PointerButton {
                        button,
                        pressed,
                        x,
                        y,
                    });
                }
            }
            RawEvent::Motion { window, x, y } if window == self.window => {
                self.pending.push_back(Event::PointerMoved {
                    x: f64::from(x),
                    y: f64::from(y),
                });
            }
            RawEvent::Crossing {
                entered,
                window,
                x,
                y,
                mode,
            } if window == self.window => {
                if entered {
                    self.pending.push_back(Event::PointerMoved {
                        x: f64::from(x),
                        y: f64::from(y),
                    });
                } else if mode == 0 {
                    self.pending.push_back(Event::PointerLeft);
                }
            }
            RawEvent::Focus {
                focused,
                window,
                detail,
                mode,
            } if window == self.window => {
                // Nur echte Fokuswechsel, keine Grabs und keine Zeigerdetails.
                if mode != 1 && mode != 2 && detail <= 4 && self.focused != Some(focused) {
                    self.focused = Some(focused);
                    self.pending.push_back(Event::Focused(focused));
                }
            }
            RawEvent::Expose { window, count } if window == self.window => {
                if count == 0 {
                    self.pending.push_back(Event::RedrawRequested);
                }
            }
            RawEvent::Configure {
                window,
                width,
                height,
            } if window == self.window => {
                let (width, height) = (u32::from(width), u32::from(height));
                if (width, height) != (self.width, self.height) {
                    self.width = width;
                    self.height = height;
                    self.pending.push_back(Event::Resized { width, height });
                }
            }
            RawEvent::ClientMessage {
                window,
                format,
                message_type,
                data,
            } if window == self.window => {
                let first = proto::Reader::new(&data).u32()?;
                if format == 32
                    && message_type == self.wm_protocols
                    && first == self.wm_delete_window
                {
                    self.pending.push_back(Event::CloseRequested);
                }
            }
            RawEvent::MappingNotify { request } if request <= 1 => {
                self.reload_keymap()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Automatische Wiederholung sendet Loslassen und Drücken mit gleicher Zeit.
    fn next_is_repeat_of(&mut self, keycode: u8, time: u32) -> Result<bool, Error> {
        let Some(next) = self.connection.peek_packet()? else {
            return Ok(false);
        };
        Ok(matches!(
            RawEvent::parse(next),
            Ok(RawEvent::Key {
                pressed: true,
                keycode: next_keycode,
                time: next_time,
                ..
            }) if next_keycode == keycode && next_time == time
        ))
    }

    fn reload_keymap(&mut self) -> Result<(), Error> {
        let min_keycode = self.connection.setup.min_keycode;
        let count = u8::try_from(
            u16::from(self.connection.setup.max_keycode)
                .saturating_sub(u16::from(min_keycode))
                .saturating_add(1),
        )
        .unwrap_or(u8::MAX);
        let keyboard = self
            .connection
            .send(proto::get_keyboard_mapping(min_keycode, count)?);
        let modifiers = self.connection.send(proto::get_modifier_mapping()?);
        let mut keymap = parse_keyboard_mapping(min_keycode, &self.connection.reply(keyboard)?)?;
        apply_modifier_mapping(&mut keymap, &self.connection.reply(modifiers)?)?;
        self.keymap = keymap;
        Ok(())
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let requests = [proto::free_gc(self.gc), proto::destroy_window(self.window)];
        for request in requests.into_iter().flatten() {
            self.connection.send(request);
        }
        // Scheitert das, ist die Verbindung ohnehin weg.
        let _ = self.connection.flush();
    }
}

fn parse_keyboard_mapping(min_keycode: u8, reply: &[u8]) -> Result<Keymap, Error> {
    let per_keycode = usize::from(*reply.get(1).ok_or_else(|| short("GetKeyboardMapping"))?);
    let keysyms = reply
        .get(proto::PACKET_SIZE..)
        .unwrap_or(&[])
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&bytes| u32::from_ne_bytes(bytes))
        .collect();
    Ok(Keymap::new(min_keycode, per_keycode, keysyms))
}

fn apply_modifier_mapping(keymap: &mut Keymap, reply: &[u8]) -> Result<(), Error> {
    let per_modifier = usize::from(*reply.get(1).ok_or_else(|| short("GetModifierMapping"))?);
    keymap.set_modifier_mapping(per_modifier, reply.get(proto::PACKET_SIZE..).unwrap_or(&[]));
    Ok(())
}

fn short(request: &str) -> Error {
    Error::Protocol(format!("Antwort auf {request} zu kurz"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(stream: UnixStream) -> Connection {
        Connection {
            stream: Stream::Unix(stream),
            setup: Setup {
                resource_id_base: 0,
                resource_id_mask: 0xff,
                max_request_length: u16::MAX,
                image_byte_order: ByteOrder::LsbFirst,
                min_keycode: 8,
                max_keycode: 255,
                vendor: String::new(),
                formats: Vec::new(),
                screens: Vec::new(),
            },
            output: Vec::new(),
            input: Vec::new(),
            sequence: 0,
            next_resource: 1,
            resource_increment: 1,
            queued: VecDeque::new(),
        }
    }

    #[test]
    fn expired_deadline_still_reads_waiting_events() {
        let (mut server, client) = UnixStream::pair().expect("Socket-Paar");
        let mut connection = connection(client);
        let mut event = [0u8; proto::PACKET_SIZE];
        event[0] = 12; // Expose
        server.write_all(&event).expect("schreiben");
        let packet = connection.next_packet(Some(Instant::now())).expect("lesen");
        assert_eq!(packet.as_deref(), Some(&event[..]));
        // Danach liegt nichts mehr vor, und die Abfrage wartet nicht.
        let packet = connection.next_packet(Some(Instant::now())).expect("lesen");
        assert_eq!(packet, None);
    }
}
