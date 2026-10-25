//! Punycode nach RFC 3492: Bootstring mit den Parametern aus Abschnitt 5.
//!
//! Alle Rechnungen sind auf Überlauf geprüft (Abschnitt 6.4), ein Überlauf
//! ist ein Fehler. Zusätzlich gilt [`MAX_LENGTH`].

const BASE: u32 = 36;
const T_MIN: u32 = 1;
const T_MAX: u32 = 26;
const SKEW: u32 = 38;
const DAMP: u32 = 700;
const INITIAL_BIAS: u32 = 72;
const INITIAL_N: u32 = 0x80;
const DELIMITER: char = '-';

/// Die größte Länge einer Eingabe in Zeichen. RFC 3492 kennt keine Grenze,
/// aber die Laufzeit wächst quadratisch mit der Länge. Längere Eingaben
/// gelten hier als Fehler; DNS erlaubt ohnehin nur 63 Bytes je Label.
pub const MAX_LENGTH: usize = 1000;

/// Dekodiert Punycode (Abschnitt 6.2), ohne das Präfix `xn--`. `None`, wenn
/// die Eingabe kein gültiges Punycode ist.
pub fn decode(input: &str) -> Option<String> {
    if input.len() > MAX_LENGTH {
        return None;
    }
    // Alles vor dem letzten Trennzeichen sind Basiszeichen. Steht es ganz
    // vorn, gehört es zum Rest und macht ihn ungültig.
    let (basic, extended) = match input.rfind(DELIMITER) {
        Some(position) if position > 0 => (input.get(..position)?, input.get(position + 1..)?),
        _ => ("", input),
    };
    if !basic.is_ascii() {
        return None;
    }
    let mut output: Vec<char> = basic.chars().collect();
    let mut n = INITIAL_N;
    let mut i: u32 = 0;
    let mut bias = INITIAL_BIAS;
    let mut digits = extended.bytes().peekable();
    while digits.peek().is_some() {
        let old_i = i;
        let mut w: u32 = 1;
        let mut k = BASE;
        loop {
            let digit = digit_value(digits.next()?)?;
            i = i.checked_add(digit.checked_mul(w)?)?;
            let t = threshold(k, bias);
            if digit < t {
                break;
            }
            w = w.checked_mul(BASE - t)?;
            k = k.checked_add(BASE)?;
        }
        let length = u32::try_from(output.len()).ok()?.checked_add(1)?;
        bias = adapt(i.checked_sub(old_i)?, length, old_i == 0);
        n = n.checked_add(i / length)?;
        i %= length;
        // `i` ist kleiner als `length`, also höchstens die Länge der Ausgabe.
        output.insert(usize::try_from(i).ok()?, char::from_u32(n)?);
        i = i.checked_add(1)?;
    }
    Some(output.into_iter().collect())
}

/// Kodiert eine Zeichenkette als Punycode (Abschnitt 6.3), ohne das Präfix
/// `xn--`. `None` bei Überlauf oder einer zu langen Eingabe.
pub fn encode(input: &str) -> Option<String> {
    let code_points: Vec<u32> = input.chars().map(u32::from).collect();
    if code_points.len() > MAX_LENGTH {
        return None;
    }
    let mut output: String = input.chars().filter(char::is_ascii).collect();
    let basic = u32::try_from(output.len()).ok()?;
    if basic > 0 {
        output.push(DELIMITER);
    }
    let total = u32::try_from(code_points.len()).ok()?;
    let mut handled = basic;
    let mut n = INITIAL_N;
    let mut delta: u32 = 0;
    let mut bias = INITIAL_BIAS;
    while handled < total {
        let m = code_points.iter().copied().filter(|&c| c >= n).min()?;
        delta = delta.checked_add((m - n).checked_mul(handled.checked_add(1)?)?)?;
        n = m;
        for &c in &code_points {
            if c < n {
                delta = delta.checked_add(1)?;
            }
            if c == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = threshold(k, bias);
                    if q < t {
                        break;
                    }
                    output.push(digit_char(t + (q - t) % (BASE - t))?);
                    q = (q - t) / (BASE - t);
                    k = k.checked_add(BASE)?;
                }
                output.push(digit_char(q)?);
                bias = adapt(delta, handled.checked_add(1)?, handled == basic);
                delta = 0;
                handled = handled.checked_add(1)?;
            }
        }
        delta = delta.checked_add(1)?;
        n = n.checked_add(1)?;
    }
    Some(output)
}

/// Der Schwellwert `t` zu `k` (Abschnitt 6.2 und 6.3).
fn threshold(k: u32, bias: u32) -> u32 {
    if k <= bias {
        T_MIN
    } else if k >= bias.saturating_add(T_MAX) {
        T_MAX
    } else {
        k - bias
    }
}

/// Die Anpassung der Verschiebung (Abschnitt 6.1).
fn adapt(delta: u32, length: u32, first: bool) -> u32 {
    let mut delta = if first { delta / DAMP } else { delta / 2 };
    delta = delta.saturating_add(delta / length.max(1));
    let mut k = 0;
    while delta > ((BASE - T_MIN) * T_MAX) / 2 {
        delta /= BASE - T_MIN;
        k += BASE;
    }
    k + ((BASE - T_MIN + 1) * delta) / (delta + SKEW)
}

/// Der Wert einer Ziffer: `a` bis `z` (auch groß) sind 0 bis 25, `0` bis `9`
/// sind 26 bis 35.
fn digit_value(byte: u8) -> Option<u32> {
    let value = match byte {
        b'a'..=b'z' => byte - b'a',
        b'A'..=b'Z' => byte - b'A',
        b'0'..=b'9' => byte - b'0' + 26,
        _ => return None,
    };
    Some(u32::from(value))
}

/// Die Ziffer zu einem Wert, mit Kleinbuchstaben.
fn digit_char(value: u32) -> Option<char> {
    let value = u8::try_from(value).ok()?;
    match value {
        0..=25 => Some(char::from(b'a' + value)),
        26..=35 => Some(char::from(b'0' + value - 26)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Beispiele, geprüft mit dem Punycode-Codec von Python. Die beiden
    /// letzten sind die Beispiele (A) und (B) aus RFC 3492, Abschnitt 7.1.
    const EXAMPLES: [(&str, &str); 9] = [
        ("bücher", "bcher-kva"),
        ("faß", "fa-hia"),
        ("münchen", "mnchen-3ya"),
        ("ü", "tda"),
        ("aübü", "ab-xkab"),
        ("😀", "e28h"),
        ("abc", "abc-"),
        (
            "\u{644}\u{64A}\u{647}\u{645}\u{627}\u{628}\u{62A}\u{643}\u{644}\u{645}\u{648}\u{634}\u{639}\u{631}\u{628}\u{64A}\u{61F}",
            "egbpdaj6bu4bxfgehfvwxn",
        ),
        (
            "\u{4ED6}\u{4EEC}\u{4E3A}\u{4EC0}\u{4E48}\u{4E0D}\u{8BF4}\u{4E2D}\u{6587}",
            "ihqwcrb4cv8a8dqg056pqjye",
        ),
    ];

    #[test]
    fn examples_in_both_directions() {
        for (text, encoded) in EXAMPLES {
            assert_eq!(encode(text).as_deref(), Some(encoded), "{text}");
            assert_eq!(decode(encoded).as_deref(), Some(text), "{encoded}");
        }
        assert_eq!(encode("").as_deref(), Some(""));
        assert_eq!(decode("").as_deref(), Some(""));
        // Großbuchstaben sind dieselben Ziffern, Basiszeichen bleiben, wie sie sind.
        assert_eq!(decode("BCHER-KVA").as_deref(), Some("BüCHER"));
    }

    #[test]
    fn invalid_input() {
        for input in [
            // Trennzeichen am Anfang: Es wird nur verbraucht, wenn davor
            // Basiszeichen stehen (Abschnitt 6.2).
            "-abc",
            "-",
            // Ungültige Ziffern, Nicht-ASCII.
            "a-b!",
            "ä-abc",
            "abc-ä",
            // Endet mitten in einer Zahl.
            "bcher-kv",
            // Überlauf und Werte über U+10FFFF.
            "99999999999999",
            "zzzzzzzzzzzzzzzzzzzzz",
        ] {
            assert_eq!(decode(input), None, "{input}");
        }
        // Ein Surrogat ist kein Zeichen. So kodiert Python U+D800.
        assert_eq!(decode("ib9b"), None);
    }

    #[test]
    fn length_limit() {
        let long = "ü".repeat(MAX_LENGTH);
        let encoded = encode(&long).expect("an der Grenze");
        assert!(encoded.len() > MAX_LENGTH);
        assert_eq!(encode(&(long + "ü")), None);
        assert_eq!(decode(&"a".repeat(MAX_LENGTH + 1)), None);
        assert_eq!(
            decode(&"a".repeat(MAX_LENGTH)).map(|t| t.chars().count()),
            Some(MAX_LENGTH)
        );
    }
}
