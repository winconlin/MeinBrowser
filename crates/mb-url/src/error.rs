//! Validierungsfehler (URL Standard, Abschnitt 1.1 „Writing“) und der
//! Fehlschlag des Parsers.

/// Ein Validierungsfehler: Die Eingabe weicht von einer gültigen URL ab. Die
/// meisten sind nur Hinweise; bei denen, die die Tabelle des Standards mit
/// „Failure“ markiert, bricht der Parser ab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// Unicode ToASCII meldet mit den strengen Schaltern einen Fehler.
    /// Abbruch, wenn die Domain nicht nur aus ASCII besteht und ToASCII auch
    /// mit den lockeren Schaltern scheitert, oder wenn das Ergebnis leer ist
    /// oder ein verbotenes Zeichen enthält.
    DomainToAscii,
    /// Eine Domain enthält prozentkodierte Bytes.
    DomainPercentEncoded,
    /// Ein opaker Host enthält ein verbotenes Zeichen. Abbruch.
    HostInvalidCodePoint,
    /// Eine IPv4-Adresse endet mit einem Punkt.
    Ipv4EmptyPart,
    /// Eine IPv4-Adresse hat weniger als vier Teile.
    Ipv4TooFewParts,
    /// Eine IPv4-Adresse hat mehr als vier Teile. Abbruch.
    Ipv4TooManyParts,
    /// Ein Teil einer IPv4-Adresse ist keine Zahl. Abbruch.
    Ipv4NonNumericPart,
    /// Ein Teil einer IPv4-Adresse ist hexadezimal oder oktal geschrieben.
    Ipv4NonDecimalPart,
    /// Ein Teil einer IPv4-Adresse ist größer als 255. Abbruch, außer beim
    /// letzten Teil, solange er in den Rest der Adresse passt.
    Ipv4OutOfRangePart,
    /// Eine IPv4-Adresse entsteht erst durch die IDNA-Verarbeitung.
    Ipv4NonAsciiInput,
    /// Einer IPv6-Adresse fehlt die schließende Klammer. Abbruch.
    Ipv6Unclosed,
    /// Eine IPv6-Adresse beginnt mit einem einzelnen Doppelpunkt. Abbruch.
    Ipv6InvalidCompression,
    /// Eine IPv6-Adresse hat mehr als acht Teile. Abbruch.
    Ipv6TooManyPieces,
    /// Eine IPv6-Adresse ist an mehr als einer Stelle verkürzt. Abbruch.
    Ipv6MultipleCompression,
    /// Eine IPv6-Adresse enthält ein Zeichen, das weder Hexziffer noch
    /// Doppelpunkt ist, oder endet unerwartet. Abbruch.
    Ipv6InvalidCodePoint,
    /// Eine unverkürzte IPv6-Adresse hat weniger als acht Teile. Abbruch.
    Ipv6TooFewPieces,
    /// Ein Teil einer IPv6-Adresse beginnt mit einer Null.
    Ipv6PieceLeadingZero,
    /// Vor einer IPv4-Adresse in einer IPv6-Adresse stehen mehr als sechs
    /// Teile. Abbruch.
    Ipv4InIpv6TooManyPieces,
    /// Ein Teil einer IPv4-Adresse in einer IPv6-Adresse ist leer, enthält
    /// etwas anderes als Ziffern oder beginnt mit einer Null, oder es gibt zu
    /// viele Teile. Abbruch.
    Ipv4InIpv6InvalidCodePoint,
    /// Ein Teil einer IPv4-Adresse in einer IPv6-Adresse ist größer als 255.
    /// Abbruch.
    Ipv4InIpv6OutOfRangePart,
    /// Eine IPv4-Adresse in einer IPv6-Adresse hat zu wenige Teile. Abbruch.
    Ipv4InIpv6TooFewParts,
    /// Ein Zeichen ist keine URL-Einheit.
    InvalidUrlUnit,
    /// Auf ein spezielles Schema folgt nicht `//`.
    SpecialSchemeMissingFollowingSolidus,
    /// Das Schema fehlt, und es gibt keine Basis-URL, die sich als Basis
    /// eignet. Abbruch.
    MissingSchemeNonRelativeUrl,
    /// Eine URL mit speziellem Schema benutzt `\` statt `/`.
    InvalidReverseSolidus,
    /// Die Eingabe enthält Benutzername oder Passwort.
    InvalidCredentials,
    /// Einer URL mit speziellem Schema fehlt der Host. Abbruch.
    HostMissing,
    /// Der Port ist zu groß. Abbruch.
    PortOutOfRange,
    /// Der Port ist keine Zahl. Abbruch.
    PortInvalid,
    /// Eine relative URL beginnt mit einem Windows-Laufwerksbuchstaben, und
    /// die Basis hat das Schema `file`.
    FileInvalidWindowsDriveLetter,
    /// Der Host einer `file:`-URL ist ein Windows-Laufwerksbuchstabe.
    FileInvalidWindowsDriveLetterHost,
}

impl ValidationError {
    /// Der Name aus dem Standard, etwa `IPv4-empty-part`.
    pub const fn name(self) -> &'static str {
        match self {
            ValidationError::DomainToAscii => "domain-to-ASCII",
            ValidationError::DomainPercentEncoded => "domain-percent-encoded",
            ValidationError::HostInvalidCodePoint => "host-invalid-code-point",
            ValidationError::Ipv4EmptyPart => "IPv4-empty-part",
            ValidationError::Ipv4TooFewParts => "IPv4-too-few-parts",
            ValidationError::Ipv4TooManyParts => "IPv4-too-many-parts",
            ValidationError::Ipv4NonNumericPart => "IPv4-non-numeric-part",
            ValidationError::Ipv4NonDecimalPart => "IPv4-non-decimal-part",
            ValidationError::Ipv4OutOfRangePart => "IPv4-out-of-range-part",
            ValidationError::Ipv4NonAsciiInput => "IPv4-non-ASCII-input",
            ValidationError::Ipv6Unclosed => "IPv6-unclosed",
            ValidationError::Ipv6InvalidCompression => "IPv6-invalid-compression",
            ValidationError::Ipv6TooManyPieces => "IPv6-too-many-pieces",
            ValidationError::Ipv6MultipleCompression => "IPv6-multiple-compression",
            ValidationError::Ipv6InvalidCodePoint => "IPv6-invalid-code-point",
            ValidationError::Ipv6TooFewPieces => "IPv6-too-few-pieces",
            ValidationError::Ipv6PieceLeadingZero => "IPv6-piece-leading-zero",
            ValidationError::Ipv4InIpv6TooManyPieces => "IPv4-in-IPv6-too-many-pieces",
            ValidationError::Ipv4InIpv6InvalidCodePoint => "IPv4-in-IPv6-invalid-code-point",
            ValidationError::Ipv4InIpv6OutOfRangePart => "IPv4-in-IPv6-out-of-range-part",
            ValidationError::Ipv4InIpv6TooFewParts => "IPv4-in-IPv6-too-few-parts",
            ValidationError::InvalidUrlUnit => "invalid-URL-unit",
            ValidationError::SpecialSchemeMissingFollowingSolidus => {
                "special-scheme-missing-following-solidus"
            }
            ValidationError::MissingSchemeNonRelativeUrl => "missing-scheme-non-relative-URL",
            ValidationError::InvalidReverseSolidus => "invalid-reverse-solidus",
            ValidationError::InvalidCredentials => "invalid-credentials",
            ValidationError::HostMissing => "host-missing",
            ValidationError::PortOutOfRange => "port-out-of-range",
            ValidationError::PortInvalid => "port-invalid",
            ValidationError::FileInvalidWindowsDriveLetter => "file-invalid-Windows-drive-letter",
            ValidationError::FileInvalidWindowsDriveLetterHost => {
                "file-invalid-Windows-drive-letter-host"
            }
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Der Fehlschlag des Parsers („failure“), mit dem Validierungsfehler, der
/// ihn ausgelöst hat. Nur die Setter der API scheitern auch ohne einen
/// solchen; dort gibt es keine Ursache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub(crate) cause: Option<ValidationError>,
}

impl ParseError {
    pub fn cause(self) -> Option<ValidationError> {
        self.cause
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.cause {
            Some(cause) => write!(f, "keine gültige URL ({cause})"),
            None => f.write_str("keine gültige URL"),
        }
    }
}

impl std::error::Error for ParseError {}

impl From<ValidationError> for ParseError {
    fn from(cause: ValidationError) -> ParseError {
        ParseError { cause: Some(cause) }
    }
}

/// Sammelt Validierungsfehler, wenn jemand sie haben will. Ohne Abnehmer
/// spart der Parser die Prüfungen, die nur Fehler melden.
pub(crate) struct Report<'a> {
    errors: Option<&'a mut Vec<ValidationError>>,
}

impl<'a> Report<'a> {
    pub(crate) fn new(errors: Option<&'a mut Vec<ValidationError>>) -> Report<'a> {
        Report { errors }
    }

    pub(crate) fn is_wanted(&self) -> bool {
        self.errors.is_some()
    }

    pub(crate) fn error(&mut self, error: ValidationError) {
        if let Some(errors) = &mut self.errors {
            errors.push(error);
        }
    }

    /// Meldet `error` und gibt ihn als Abbruch zurück.
    pub(crate) fn fail<T>(&mut self, error: ValidationError) -> Result<T, ValidationError> {
        self.error(error);
        Err(error)
    }
}
