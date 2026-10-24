//! Anfragen, Antworten und Ereignisse des X11-Kernprotokolls.
//!
//! Grundlage ist die Spezifikation „X Window System Protocol, X Version 11,
//! Release 6.9/7.0“. Beim Verbindungsaufbau kündigt der Client die eigene
//! Bytereihenfolge an. Darum werden alle Zahlen hier in der Reihenfolge der
//! eigenen Maschine geschrieben und gelesen.

#![forbid(unsafe_code)]

use crate::Error;

/// Byte, mit dem der Client beim Verbindungsaufbau seine Bytereihenfolge
/// ankündigt: `l` für Little-Endian, `B` für Big-Endian.
const BYTE_ORDER: u8 = if cfg!(target_endian = "little") {
    b'l'
} else {
    b'B'
};

pub const ATOM_ATOM: u32 = 4;
pub const ATOM_STRING: u32 = 31;
pub const ATOM_WM_NAME: u32 = 39;
pub const ATOM_WM_CLASS: u32 = 67;

pub const WINDOW_CLASS_INPUT_OUTPUT: u16 = 1;
pub const IMAGE_FORMAT_Z_PIXMAP: u8 = 2;

pub const CW_BACK_PIXEL: u32 = 0x0002;
pub const CW_BIT_GRAVITY: u32 = 0x0010;
pub const CW_EVENT_MASK: u32 = 0x0800;
pub const BIT_GRAVITY_NORTH_WEST: u32 = 1;

pub const EVENT_MASK_KEY_PRESS: u32 = 0x0000_0001;
pub const EVENT_MASK_KEY_RELEASE: u32 = 0x0000_0002;
pub const EVENT_MASK_BUTTON_PRESS: u32 = 0x0000_0004;
pub const EVENT_MASK_BUTTON_RELEASE: u32 = 0x0000_0008;
pub const EVENT_MASK_ENTER_WINDOW: u32 = 0x0000_0010;
pub const EVENT_MASK_LEAVE_WINDOW: u32 = 0x0000_0020;
pub const EVENT_MASK_POINTER_MOTION: u32 = 0x0000_0040;
pub const EVENT_MASK_EXPOSURE: u32 = 0x0000_8000;
pub const EVENT_MASK_STRUCTURE_NOTIFY: u32 = 0x0002_0000;
pub const EVENT_MASK_FOCUS_CHANGE: u32 = 0x0020_0000;

pub const GC_GRAPHICS_EXPOSURES: u32 = 0x0001_0000;

/// Länge eines Ereignisses, eines Fehlers und des festen Teils einer Antwort.
pub const PACKET_SIZE: usize = 32;

/// Liest Werte aus einem Byte-Slice, ohne je einen ungültigen Index zu
/// benutzen.
pub struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, position: 0 }
    }

    pub fn bytes(&mut self, count: usize) -> Result<&'a [u8], Error> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| too_short("Länge"))?;
        let bytes = self
            .data
            .get(self.position..end)
            .ok_or_else(|| too_short("Daten"))?;
        self.position = end;
        Ok(bytes)
    }

    pub fn skip(&mut self, count: usize) -> Result<(), Error> {
        self.bytes(count).map(|_| ())
    }

    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_ne_bytes(self.array()?))
    }

    pub fn i16(&mut self) -> Result<i16, Error> {
        Ok(i16::from_ne_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_ne_bytes(self.array()?))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let bytes = self.bytes(N)?;
        let mut array = [0; N];
        array.copy_from_slice(bytes);
        Ok(array)
    }
}

fn too_short(what: &str) -> Error {
    Error::Protocol(format!("X11-Nachricht zu kurz ({what})"))
}

/// Füllt auf ein Vielfaches von vier Bytes auf.
pub fn pad(length: usize) -> usize {
    (4 - length % 4) % 4
}

/// Schreibt eine Anfrage: Opcode, ein Datenbyte, Länge in Vier-Byte-Einheiten,
/// dann der Rumpf, aufgefüllt auf ein Vielfaches von vier Bytes.
struct Request {
    bytes: Vec<u8>,
}

impl Request {
    fn new(opcode: u8, data: u8) -> Request {
        let mut bytes = Vec::with_capacity(32);
        bytes.extend_from_slice(&[opcode, data, 0, 0]);
        Request { bytes }
    }

    fn u8(mut self, value: u8) -> Request {
        self.bytes.push(value);
        self
    }

    fn u16(mut self, value: u16) -> Request {
        self.bytes.extend_from_slice(&value.to_ne_bytes());
        self
    }

    fn i16(mut self, value: i16) -> Request {
        self.bytes.extend_from_slice(&value.to_ne_bytes());
        self
    }

    fn u32(mut self, value: u32) -> Request {
        self.bytes.extend_from_slice(&value.to_ne_bytes());
        self
    }

    fn bytes(mut self, value: &[u8]) -> Request {
        self.bytes.extend_from_slice(value);
        self
    }

    fn finish(mut self) -> Result<Vec<u8>, Error> {
        let padding = pad(self.bytes.len());
        self.bytes.resize(self.bytes.len() + padding, 0);
        let units = u16::try_from(self.bytes.len() / 4).map_err(|_| {
            Error::InvalidArgument(format!(
                "X11-Anfrage mit {} Bytes ist zu lang",
                self.bytes.len()
            ))
        })?;
        self.bytes[2..4].copy_from_slice(&units.to_ne_bytes());
        Ok(self.bytes)
    }
}

/// Die Anfrage zum Verbindungsaufbau.
pub fn setup_request(auth_name: &[u8], auth_data: &[u8]) -> Result<Vec<u8>, Error> {
    let name_len = u16::try_from(auth_name.len())
        .map_err(|_| Error::InvalidArgument("Name der X11-Anmeldung zu lang".into()))?;
    let data_len = u16::try_from(auth_data.len())
        .map_err(|_| Error::InvalidArgument("Daten der X11-Anmeldung zu lang".into()))?;
    let mut bytes = vec![BYTE_ORDER, 0];
    bytes.extend_from_slice(&11u16.to_ne_bytes());
    bytes.extend_from_slice(&0u16.to_ne_bytes());
    bytes.extend_from_slice(&name_len.to_ne_bytes());
    bytes.extend_from_slice(&data_len.to_ne_bytes());
    bytes.extend_from_slice(&[0, 0]);
    bytes.extend_from_slice(auth_name);
    bytes.resize(bytes.len() + pad(auth_name.len()), 0);
    bytes.extend_from_slice(auth_data);
    bytes.resize(bytes.len() + pad(auth_data.len()), 0);
    Ok(bytes)
}

/// Bytereihenfolge der Bilddaten des Servers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOrder {
    LsbFirst,
    MsbFirst,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Format {
    pub depth: u8,
    pub bits_per_pixel: u8,
    pub scanline_pad: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visual {
    pub id: u32,
    pub class: u8,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
}

pub const VISUAL_CLASS_TRUE_COLOR: u8 = 4;
pub const VISUAL_CLASS_DIRECT_COLOR: u8 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Depth {
    pub depth: u8,
    pub visuals: Vec<Visual>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    pub root: u32,
    pub white_pixel: u32,
    pub black_pixel: u32,
    pub width: u16,
    pub height: u16,
    pub root_visual: u32,
    pub root_depth: u8,
    pub depths: Vec<Depth>,
}

impl Screen {
    pub fn visual(&self, id: u32) -> Option<(u8, &Visual)> {
        self.depths.iter().find_map(|depth| {
            depth
                .visuals
                .iter()
                .find(|visual| visual.id == id)
                .map(|visual| (depth.depth, visual))
        })
    }
}

/// Was der Server beim Verbindungsaufbau über sich mitteilt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setup {
    pub resource_id_base: u32,
    pub resource_id_mask: u32,
    /// Größte Anfrage in Vier-Byte-Einheiten.
    pub max_request_length: u16,
    pub image_byte_order: ByteOrder,
    pub min_keycode: u8,
    pub max_keycode: u8,
    pub vendor: String,
    pub formats: Vec<Format>,
    pub screens: Vec<Screen>,
}

/// Der feste, acht Byte lange Kopf der Antwort auf den Verbindungsaufbau:
/// Status und Länge des Rests in Bytes.
pub fn setup_reply_header(header: &[u8]) -> Result<(u8, u8, usize), Error> {
    let mut reader = Reader::new(header);
    let status = reader.u8()?;
    let reason_length = reader.u8()?;
    reader.skip(4)?;
    let units = reader.u16()?;
    Ok((status, reason_length, usize::from(units) * 4))
}

/// Wertet die Antwort auf den Verbindungsaufbau aus. `body` ist alles nach
/// dem acht Byte langen Kopf.
pub fn parse_setup(status: u8, reason_length: u8, body: &[u8]) -> Result<Setup, Error> {
    match status {
        1 => {}
        0 => {
            let reason = body.get(..usize::from(reason_length)).unwrap_or(body);
            return Err(Error::Protocol(format!(
                "X-Server lehnt die Verbindung ab: {}",
                String::from_utf8_lossy(reason).trim_end()
            )));
        }
        2 => {
            return Err(Error::Protocol(format!(
                "X-Server verlangt eine weitere Anmeldung: {}",
                String::from_utf8_lossy(body)
                    .trim_end_matches('\0')
                    .trim_end()
            )));
        }
        other => {
            return Err(Error::Protocol(format!(
                "unbekannter Status {other} beim Verbindungsaufbau"
            )));
        }
    }
    let mut reader = Reader::new(body);
    let _release = reader.u32()?;
    let resource_id_base = reader.u32()?;
    let resource_id_mask = reader.u32()?;
    let _motion_buffer_size = reader.u32()?;
    let vendor_length = usize::from(reader.u16()?);
    let max_request_length = reader.u16()?;
    let screen_count = reader.u8()?;
    let format_count = reader.u8()?;
    let image_byte_order = match reader.u8()? {
        0 => ByteOrder::LsbFirst,
        _ => ByteOrder::MsbFirst,
    };
    let _bitmap_bit_order = reader.u8()?;
    let _bitmap_scanline_unit = reader.u8()?;
    let _bitmap_scanline_pad = reader.u8()?;
    let min_keycode = reader.u8()?;
    let max_keycode = reader.u8()?;
    reader.skip(4)?;
    let vendor = String::from_utf8_lossy(reader.bytes(vendor_length)?).into_owned();
    reader.skip(pad(vendor_length))?;

    let mut formats = Vec::with_capacity(usize::from(format_count));
    for _ in 0..format_count {
        let depth = reader.u8()?;
        let bits_per_pixel = reader.u8()?;
        let scanline_pad = reader.u8()?;
        reader.skip(5)?;
        formats.push(Format {
            depth,
            bits_per_pixel,
            scanline_pad,
        });
    }

    let mut screens = Vec::with_capacity(usize::from(screen_count));
    for _ in 0..screen_count {
        let root = reader.u32()?;
        let _default_colormap = reader.u32()?;
        let white_pixel = reader.u32()?;
        let black_pixel = reader.u32()?;
        let _current_input_masks = reader.u32()?;
        let width = reader.u16()?;
        let height = reader.u16()?;
        let _width_mm = reader.u16()?;
        let _height_mm = reader.u16()?;
        let _min_installed_maps = reader.u16()?;
        let _max_installed_maps = reader.u16()?;
        let root_visual = reader.u32()?;
        let _backing_stores = reader.u8()?;
        let _save_unders = reader.u8()?;
        let root_depth = reader.u8()?;
        let depth_count = reader.u8()?;
        let mut depths = Vec::with_capacity(usize::from(depth_count));
        for _ in 0..depth_count {
            let depth = reader.u8()?;
            reader.skip(1)?;
            let visual_count = reader.u16()?;
            reader.skip(4)?;
            let mut visuals = Vec::with_capacity(usize::from(visual_count));
            for _ in 0..visual_count {
                let id = reader.u32()?;
                let class = reader.u8()?;
                let _bits_per_rgb = reader.u8()?;
                let _colormap_entries = reader.u16()?;
                let red_mask = reader.u32()?;
                let green_mask = reader.u32()?;
                let blue_mask = reader.u32()?;
                reader.skip(4)?;
                visuals.push(Visual {
                    id,
                    class,
                    red_mask,
                    green_mask,
                    blue_mask,
                });
            }
            depths.push(Depth { depth, visuals });
        }
        screens.push(Screen {
            root,
            white_pixel,
            black_pixel,
            width,
            height,
            root_visual,
            root_depth,
            depths,
        });
    }

    Ok(Setup {
        resource_id_base,
        resource_id_mask,
        max_request_length,
        image_byte_order,
        min_keycode,
        max_keycode,
        vendor,
        formats,
        screens,
    })
}

/// Werte für `CreateWindow`, die dieser Client setzt.
pub struct WindowValues {
    pub background_pixel: u32,
    pub bit_gravity: u32,
    pub event_mask: u32,
}

#[allow(clippy::too_many_arguments)]
pub fn create_window(
    window: u32,
    parent: u32,
    depth: u8,
    visual: u32,
    width: u16,
    height: u16,
    values: &WindowValues,
) -> Result<Vec<u8>, Error> {
    // Die Werte folgen in der Reihenfolge ihrer Bits in der Maske.
    Request::new(1, depth)
        .u32(window)
        .u32(parent)
        .i16(0)
        .i16(0)
        .u16(width)
        .u16(height)
        .u16(0)
        .u16(WINDOW_CLASS_INPUT_OUTPUT)
        .u32(visual)
        .u32(CW_BACK_PIXEL | CW_BIT_GRAVITY | CW_EVENT_MASK)
        .u32(values.background_pixel)
        .u32(values.bit_gravity)
        .u32(values.event_mask)
        .finish()
}

pub fn destroy_window(window: u32) -> Result<Vec<u8>, Error> {
    Request::new(4, 0).u32(window).finish()
}

pub fn map_window(window: u32) -> Result<Vec<u8>, Error> {
    Request::new(8, 0).u32(window).finish()
}

pub fn intern_atom(name: &str) -> Result<Vec<u8>, Error> {
    let length =
        u16::try_from(name.len()).map_err(|_| Error::InvalidArgument("Atomname zu lang".into()))?;
    Request::new(16, 0)
        .u16(length)
        .u16(0)
        .bytes(name.as_bytes())
        .finish()
}

/// `ChangeProperty` im Modus „Replace“. `format` ist 8, 16 oder 32 Bit je
/// Element, `data` enthält die Elemente bereits in eigener Bytereihenfolge.
pub fn change_property(
    window: u32,
    property: u32,
    kind: u32,
    format: u8,
    data: &[u8],
) -> Result<Vec<u8>, Error> {
    let element_size = usize::from(format / 8).max(1);
    let elements = u32::try_from(data.len() / element_size)
        .map_err(|_| Error::InvalidArgument("Eigenschaft zu lang".into()))?;
    Request::new(18, 0)
        .u32(window)
        .u32(property)
        .u32(kind)
        .u8(format)
        .bytes(&[0, 0, 0])
        .u32(elements)
        .bytes(data)
        .finish()
}

pub fn get_input_focus() -> Result<Vec<u8>, Error> {
    Request::new(43, 0).finish()
}

pub fn create_gc(gc: u32, drawable: u32) -> Result<Vec<u8>, Error> {
    Request::new(55, 0)
        .u32(gc)
        .u32(drawable)
        .u32(GC_GRAPHICS_EXPOSURES)
        .u32(0)
        .finish()
}

pub fn free_gc(gc: u32) -> Result<Vec<u8>, Error> {
    Request::new(60, 0).u32(gc).finish()
}

/// Länge des Kopfs einer `PutImage`-Anfrage in Bytes.
pub const PUT_IMAGE_HEADER: usize = 24;

#[allow(clippy::too_many_arguments)]
pub fn put_image(
    drawable: u32,
    gc: u32,
    width: u16,
    height: u16,
    x: i16,
    y: i16,
    depth: u8,
    data: &[u8],
) -> Result<Vec<u8>, Error> {
    Request::new(72, IMAGE_FORMAT_Z_PIXMAP)
        .u32(drawable)
        .u32(gc)
        .u16(width)
        .u16(height)
        .i16(x)
        .i16(y)
        .u8(0)
        .u8(depth)
        .u16(0)
        .bytes(data)
        .finish()
}

pub fn get_image(drawable: u32, x: i16, y: i16, width: u16, height: u16) -> Result<Vec<u8>, Error> {
    Request::new(73, IMAGE_FORMAT_Z_PIXMAP)
        .u32(drawable)
        .i16(x)
        .i16(y)
        .u16(width)
        .u16(height)
        .u32(u32::MAX)
        .finish()
}

pub fn get_keyboard_mapping(first: u8, count: u8) -> Result<Vec<u8>, Error> {
    Request::new(101, 0).u8(first).u8(count).u16(0).finish()
}

pub fn get_modifier_mapping() -> Result<Vec<u8>, Error> {
    Request::new(119, 0).finish()
}

/// Wie lang ein Paket insgesamt ist, sobald seine ersten acht Bytes bekannt
/// sind: Antworten (Typ 1) und generische Ereignisse (Typ 35) haben einen
/// Rumpf, alles andere ist 32 Bytes lang.
pub fn packet_length(header: &[u8]) -> Option<usize> {
    let kind = *header.first()? & 0x7f;
    if kind == 1 || kind == 35 {
        let units = u32::from_ne_bytes(header.get(4..8)?.try_into().ok()?);
        let extra = usize::try_from(units).ok()?.checked_mul(4)?;
        PACKET_SIZE.checked_add(extra)
    } else {
        Some(PACKET_SIZE)
    }
}

/// Ein Fehler, den der Server meldet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XError {
    pub code: u8,
    pub sequence: u16,
    pub bad_value: u32,
    pub minor_opcode: u16,
    pub major_opcode: u8,
}

impl XError {
    pub fn parse(packet: &[u8]) -> Result<XError, Error> {
        let mut reader = Reader::new(packet);
        reader.skip(1)?;
        let code = reader.u8()?;
        let sequence = reader.u16()?;
        let bad_value = reader.u32()?;
        let minor_opcode = reader.u16()?;
        let major_opcode = reader.u8()?;
        Ok(XError {
            code,
            sequence,
            bad_value,
            minor_opcode,
            major_opcode,
        })
    }

    pub fn into_error(self) -> Error {
        let name = match self.code {
            1 => "Request",
            2 => "Value",
            3 => "Window",
            4 => "Pixmap",
            5 => "Atom",
            6 => "Cursor",
            7 => "Font",
            8 => "Match",
            9 => "Drawable",
            10 => "Access",
            11 => "Alloc",
            12 => "Colormap",
            13 => "GContext",
            14 => "IDChoice",
            15 => "Name",
            16 => "Length",
            17 => "Implementation",
            _ => "unbekannt",
        };
        Error::Protocol(format!(
            "X-Server meldet Fehler {} ({name}) zu Anfrage {}.{} (Wert {:#x})",
            self.code, self.major_opcode, self.minor_opcode, self.bad_value
        ))
    }
}

/// Die Sequenznummer einer Antwort, eines Fehlers oder Ereignisses.
pub fn packet_sequence(packet: &[u8]) -> Option<u16> {
    Some(u16::from_ne_bytes(packet.get(2..4)?.try_into().ok()?))
}

/// Ein Ereignis, so wie es vom Server kommt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawEvent {
    Key {
        pressed: bool,
        keycode: u8,
        time: u32,
        window: u32,
        state: u16,
    },
    Button {
        pressed: bool,
        button: u8,
        window: u32,
        x: i16,
        y: i16,
        state: u16,
    },
    Motion {
        window: u32,
        x: i16,
        y: i16,
    },
    Crossing {
        entered: bool,
        window: u32,
        x: i16,
        y: i16,
        mode: u8,
    },
    Focus {
        focused: bool,
        window: u32,
        detail: u8,
        mode: u8,
    },
    Expose {
        window: u32,
        count: u16,
    },
    Configure {
        window: u32,
        width: u16,
        height: u16,
    },
    ClientMessage {
        window: u32,
        format: u8,
        message_type: u32,
        data: [u8; 20],
    },
    MappingNotify {
        request: u8,
    },
    /// Ein Ereignis, das dieser Client nicht auswertet.
    Other(u8),
}

impl RawEvent {
    /// Dekodiert ein 32 Byte langes Ereignis.
    pub fn parse(packet: &[u8]) -> Result<RawEvent, Error> {
        let mut reader = Reader::new(packet);
        let code = reader.u8()? & 0x7f;
        let detail = reader.u8()?;
        let _sequence = reader.u16()?;
        Ok(match code {
            2..=6 => {
                let time = reader.u32()?;
                let _root = reader.u32()?;
                let window = reader.u32()?;
                let _child = reader.u32()?;
                let _root_x = reader.i16()?;
                let _root_y = reader.i16()?;
                let x = reader.i16()?;
                let y = reader.i16()?;
                let state = reader.u16()?;
                match code {
                    2 | 3 => RawEvent::Key {
                        pressed: code == 2,
                        keycode: detail,
                        time,
                        window,
                        state,
                    },
                    4 | 5 => RawEvent::Button {
                        pressed: code == 4,
                        button: detail,
                        window,
                        x,
                        y,
                        state,
                    },
                    _ => RawEvent::Motion { window, x, y },
                }
            }
            7 | 8 => {
                let _time = reader.u32()?;
                let _root = reader.u32()?;
                let window = reader.u32()?;
                let _child = reader.u32()?;
                let _root_x = reader.i16()?;
                let _root_y = reader.i16()?;
                let x = reader.i16()?;
                let y = reader.i16()?;
                let _state = reader.u16()?;
                let mode = reader.u8()?;
                RawEvent::Crossing {
                    entered: code == 7,
                    window,
                    x,
                    y,
                    mode,
                }
            }
            9 | 10 => {
                let window = reader.u32()?;
                let mode = reader.u8()?;
                RawEvent::Focus {
                    focused: code == 9,
                    window,
                    detail,
                    mode,
                }
            }
            12 => {
                let window = reader.u32()?;
                reader.skip(8)?;
                let count = reader.u16()?;
                RawEvent::Expose { window, count }
            }
            22 => {
                let _event = reader.u32()?;
                let window = reader.u32()?;
                let _above_sibling = reader.u32()?;
                let _x = reader.i16()?;
                let _y = reader.i16()?;
                let width = reader.u16()?;
                let height = reader.u16()?;
                RawEvent::Configure {
                    window,
                    width,
                    height,
                }
            }
            33 => {
                let window = reader.u32()?;
                let message_type = reader.u32()?;
                let mut data = [0; 20];
                data.copy_from_slice(reader.bytes(20)?);
                RawEvent::ClientMessage {
                    window,
                    format: detail,
                    message_type,
                    data,
                }
            }
            34 => RawEvent::MappingNotify {
                request: reader.u8()?,
            },
            other => RawEvent::Other(other),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16s(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_ne_bytes()).collect()
    }

    fn u32s(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_ne_bytes()).collect()
    }

    #[test]
    fn requests_have_correct_length_and_padding() {
        let request = intern_atom("WM_PROTOCOLS").unwrap();
        // 8 Byte Kopf, 12 Byte Name: 20 Bytes, also fünf Einheiten.
        assert_eq!(request.len(), 20);
        assert_eq!(request[0], 16);
        assert_eq!(u16::from_ne_bytes([request[2], request[3]]), 5);
        assert_eq!(&request[8..20], b"WM_PROTOCOLS");

        let request = intern_atom("ATOM_X").unwrap();
        assert_eq!(request.len(), 16);
        assert_eq!(&request[14..16], &[0, 0]);

        assert_eq!(map_window(7).unwrap().len(), 8);
        assert_eq!(get_input_focus().unwrap(), {
            let mut expected = vec![43, 0];
            expected.extend(u16s(&[1]));
            expected
        });
    }

    #[test]
    fn create_window_lists_values_in_mask_order() {
        let request = create_window(
            0x0040_0001,
            0x2a,
            24,
            0x21,
            640,
            400,
            &WindowValues {
                background_pixel: 0,
                bit_gravity: BIT_GRAVITY_NORTH_WEST,
                event_mask: EVENT_MASK_EXPOSURE,
            },
        )
        .unwrap();
        assert_eq!(request.len(), 44);
        assert_eq!(request[1], 24);
        assert_eq!(u16::from_ne_bytes([request[2], request[3]]), 11);
        let mut reader = Reader::new(&request[28..]);
        assert_eq!(
            reader.u32().unwrap(),
            CW_BACK_PIXEL | CW_BIT_GRAVITY | CW_EVENT_MASK
        );
        assert_eq!(reader.u32().unwrap(), 0);
        assert_eq!(reader.u32().unwrap(), BIT_GRAVITY_NORTH_WEST);
        assert_eq!(reader.u32().unwrap(), EVENT_MASK_EXPOSURE);
    }

    #[test]
    fn too_long_requests_are_rejected() {
        let data = vec![0; 4 * 65_536];
        assert!(put_image(1, 2, 256, 256, 0, 0, 24, &data).is_err());
    }

    #[test]
    fn setup_request_pads_name_and_data() {
        let request = setup_request(b"MIT-MAGIC-COOKIE-1", &[1; 16]).unwrap();
        assert_eq!(request.len(), 12 + 20 + 16);
        assert_eq!(request[0], BYTE_ORDER);
        assert_eq!(&request[12..30], b"MIT-MAGIC-COOKIE-1");
        assert_eq!(&request[30..32], &[0, 0]);
    }

    /// Eine Antwort, wie Xvfb sie mit einem Bildschirm sendet (gekürzt).
    fn sample_setup_body() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend(u32s(&[12_101_011, 0x0020_0000, 0x001f_ffff, 256]));
        body.extend(u16s(&[20, 65_535]));
        body.extend([1, 2, 0, 0, 32, 32, 8, 255, 0, 0, 0, 0]);
        body.extend(b"The X.Org Foundation\0\0\0\0".iter().take(20));
        body.extend([1, 1, 32, 0, 0, 0, 0, 0]);
        body.extend([24, 32, 32, 0, 0, 0, 0, 0]);
        // Bildschirm
        body.extend(u32s(&[0x3a5, 0x21, 0x00ff_ffff, 0, 0]));
        body.extend(u16s(&[1024, 768, 270, 203, 1, 1]));
        body.extend(u32s(&[0x21]));
        body.extend([0, 0, 24, 1]);
        // Tiefe 24 mit einem Visual
        body.extend([24, 0]);
        body.extend(u16s(&[1]));
        body.extend([0, 0, 0, 0]);
        body.extend(u32s(&[0x21]));
        body.extend([4, 8]);
        body.extend(u16s(&[256]));
        body.extend(u32s(&[0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0]));
        body
    }

    #[test]
    fn parses_setup_reply() {
        let setup = parse_setup(1, 0, &sample_setup_body()).unwrap();
        assert_eq!(setup.resource_id_base, 0x0020_0000);
        assert_eq!(setup.resource_id_mask, 0x001f_ffff);
        assert_eq!(setup.max_request_length, 65_535);
        assert_eq!(setup.image_byte_order, ByteOrder::LsbFirst);
        assert_eq!((setup.min_keycode, setup.max_keycode), (8, 255));
        assert_eq!(setup.vendor, "The X.Org Foundation");
        assert_eq!(setup.formats.len(), 2);
        assert_eq!(setup.formats[1].bits_per_pixel, 32);
        let screen = &setup.screens[0];
        assert_eq!(
            (screen.width, screen.height, screen.root_depth),
            (1024, 768, 24)
        );
        let (depth, visual) = screen.visual(0x21).unwrap();
        assert_eq!(depth, 24);
        assert_eq!(visual.class, VISUAL_CLASS_TRUE_COLOR);
        assert_eq!(visual.red_mask, 0x00ff_0000);
    }

    #[test]
    fn truncated_setup_is_an_error_not_a_panic() {
        let body = sample_setup_body();
        for length in 0..body.len() {
            assert!(
                parse_setup(1, 0, &body[..length]).is_err(),
                "Länge {length}"
            );
        }
    }

    #[test]
    fn refused_setup_reports_reason() {
        let err = parse_setup(0, 22, b"No protocol specified\n\0\0").unwrap_err();
        assert!(err.to_string().contains("No protocol specified"), "{err}");
    }

    #[test]
    fn packet_lengths() {
        let mut reply = [0u8; 32];
        reply[0] = 1;
        reply[4..8].copy_from_slice(&3u32.to_ne_bytes());
        assert_eq!(packet_length(&reply), Some(44));
        reply[0] = 12;
        assert_eq!(packet_length(&reply), Some(32));
        assert_eq!(packet_length(&reply[..1]), Some(32));
        reply[0] = 1;
        assert_eq!(packet_length(&reply[..6]), None);
    }

    #[test]
    fn parses_events() {
        let mut packet = [0u8; 32];
        packet[0] = 2 | 0x80;
        packet[1] = 38;
        packet[4..8].copy_from_slice(&1234u32.to_ne_bytes());
        packet[12..16].copy_from_slice(&0x0040_0001u32.to_ne_bytes());
        packet[28..30].copy_from_slice(&1u16.to_ne_bytes());
        assert_eq!(
            RawEvent::parse(&packet).unwrap(),
            RawEvent::Key {
                pressed: true,
                keycode: 38,
                time: 1234,
                window: 0x0040_0001,
                state: 1,
            }
        );

        let mut packet = [0u8; 32];
        packet[0] = 22;
        packet[8..12].copy_from_slice(&5u32.to_ne_bytes());
        packet[20..22].copy_from_slice(&800u16.to_ne_bytes());
        packet[22..24].copy_from_slice(&600u16.to_ne_bytes());
        assert_eq!(
            RawEvent::parse(&packet).unwrap(),
            RawEvent::Configure {
                window: 5,
                width: 800,
                height: 600,
            }
        );

        let mut packet = [0u8; 32];
        packet[0] = 33;
        packet[1] = 32;
        packet[4..8].copy_from_slice(&5u32.to_ne_bytes());
        packet[8..12].copy_from_slice(&300u32.to_ne_bytes());
        packet[12..16].copy_from_slice(&301u32.to_ne_bytes());
        let RawEvent::ClientMessage {
            window,
            format,
            message_type,
            data,
        } = RawEvent::parse(&packet).unwrap()
        else {
            panic!("keine ClientMessage");
        };
        assert_eq!((window, format, message_type), (5, 32, 300));
        assert_eq!(&data[..4], &301u32.to_ne_bytes());

        assert!(RawEvent::parse(&packet[..10]).is_err());
    }

    #[test]
    fn parses_errors() {
        let mut packet = [0u8; 32];
        packet[1] = 8;
        packet[2..4].copy_from_slice(&7u16.to_ne_bytes());
        packet[10] = 72;
        let error = XError::parse(&packet).unwrap();
        assert_eq!(error.code, 8);
        assert_eq!(error.sequence, 7);
        assert_eq!(error.major_opcode, 72);
        assert!(error.into_error().to_string().contains("Match"));
    }
}
