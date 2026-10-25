//! Prüfsummen: CRC-32 (ISO 3309, wie in PNG, gzip und ZIP) und Adler-32
//! (RFC 1950).

/// Die Tabelle für CRC-32 mit dem gespiegelten Polynom 0xEDB88320.
const CRC_TABLE: [u32; 256] = {
    let mut table = [0_u32; 256];
    let mut index = 0;
    while index < 256 {
        // `index` ist kleiner als 256, passt also immer.
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                0xEDB8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
};

/// CRC-32 über mehrere Stücke.
#[derive(Clone, Copy, Debug)]
pub struct Crc32 {
    state: u32,
}

impl Default for Crc32 {
    fn default() -> Crc32 {
        Crc32::new()
    }
}

impl Crc32 {
    pub fn new() -> Crc32 {
        Crc32 { state: !0 }
    }

    pub fn update(&mut self, data: &[u8]) {
        for &byte in data {
            let [low, ..] = self.state.to_le_bytes();
            let index = usize::from(low ^ byte);
            self.state = CRC_TABLE.get(index).copied().unwrap_or(0) ^ (self.state >> 8);
        }
    }

    pub fn value(&self) -> u32 {
        !self.state
    }
}

/// CRC-32 eines Stücks.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(data);
    crc.value()
}

/// Adler-32 nach RFC 1950, Abschnitt 8.
pub fn adler32(data: &[u8]) -> u32 {
    const MODULUS: u32 = 65521;
    // So viele Bytes, dass `b` in 32 Bit nicht überläuft (wie in zlib).
    const BLOCK: usize = 5552;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for block in data.chunks(BLOCK) {
        for &byte in block {
            a += u32::from(byte);
            b += a;
        }
        a %= MODULUS;
        b %= MODULUS;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        // Prüfwerte aus dem Katalog der CRC-Verfahren („check“ für
        // CRC-32/ISO-HDLC) und aus RFC 1950 (leere Eingabe ergibt 1).
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        let mut crc = Crc32::new();
        crc.update(b"1234");
        crc.update(b"56789");
        assert_eq!(crc.value(), 0xCBF4_3926);
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        // Lang genug für mehrere Blöcke.
        let long = vec![0xFF_u8; 100_000];
        let mut a: u64 = 1;
        let mut b: u64 = 0;
        for &byte in &long {
            a = (a + u64::from(byte)) % 65521;
            b = (b + a) % 65521;
        }
        assert_eq!(u64::from(adler32(&long)), (b << 16) | a);
    }
}
