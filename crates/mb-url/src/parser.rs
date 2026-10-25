//! Der *basic URL parser* (`#concept-basic-url-parser`) als Zustandsautomat,
//! Schritt für Schritt nach dem Standard. Die Kommentare `#…` nennen die
//! Anker der Abschnitte.

use mb_encoding::Encoding;

use crate::error::{ParseError, Report, ValidationError};
use crate::host::{self, Host, is_url_code_point};
use crate::percent::{
    C0_CONTROL, FRAGMENT, PATH, QUERY, SPECIAL_QUERY, USERINFO, percent_encode_after_encoding,
    push_utf8_percent_encoded,
};
use crate::{Path, Url, default_port, is_special_scheme};

/// Die Zustände des Parsers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    SchemeStart,
    Scheme,
    NoScheme,
    SpecialRelativeOrAuthority,
    PathOrAuthority,
    Relative,
    RelativeSlash,
    SpecialAuthoritySlashes,
    SpecialAuthorityIgnoreSlashes,
    Authority,
    Host,
    Hostname,
    Port,
    File,
    FileSlash,
    FileHost,
    PathStart,
    Path,
    OpaquePath,
    Query,
    Fragment,
}

/// Wie es nach einem Schritt weitergeht.
enum Step {
    /// Zum nächsten Zeichen, oder fertig am Ende der Eingabe.
    Next,
    /// Dasselbe Zeichen noch einmal. Das steht für „decrease pointer by 1“,
    /// denn danach erhöht die Schleife den Zeiger wieder.
    Again,
    /// „return“ mit *state override*: fertig, ohne Fehlschlag.
    Return,
}

/// Läuft den Parser über `input`. Ohne `state_override` ist `url` neu, und
/// die Eingabe wird am Rand gekürzt; mit `state_override` ändert der Parser
/// die übergebene URL (für die Setter der API). Ein Fehlschlag kann `url`
/// teilweise geändert lassen, so wie im Standard.
pub(crate) fn basic_url_parse(
    input: &str,
    base: Option<&Url>,
    encoding: Encoding,
    url: &mut Url,
    state_override: Option<State>,
    report: &mut Report,
) -> Result<(), ParseError> {
    let mut input = input;
    if state_override.is_none() {
        // C0-Steuerzeichen oder Leerzeichen: U+0000 bis U+0020.
        let trimmed = input.trim_matches(|c: char| c <= ' ');
        if trimmed.len() != input.len() {
            report.error(ValidationError::InvalidUrlUnit);
        }
        input = trimmed;
    }
    let is_tab_or_newline = |c: char| matches!(c, '\t' | '\n' | '\r');
    if input.contains(is_tab_or_newline) {
        report.error(ValidationError::InvalidUrlUnit);
    }
    let mut parser = Parser {
        input: input.chars().filter(|&c| !is_tab_or_newline(c)).collect(),
        pointer: 0,
        state: state_override.unwrap_or(State::SchemeStart),
        state_override,
        base,
        encoding: encoding.output_encoding(),
        url,
        buffer: String::new(),
        at_sign_seen: false,
        inside_brackets: false,
        password_token_seen: false,
        report,
    };
    parser.run()
}

struct Parser<'p, 'r> {
    input: Vec<char>,
    pointer: usize,
    state: State,
    state_override: Option<State>,
    base: Option<&'p Url>,
    encoding: Encoding,
    url: &'p mut Url,
    buffer: String,
    at_sign_seen: bool,
    inside_brackets: bool,
    password_token_seen: bool,
    report: &'p mut Report<'r>,
}

impl Parser<'_, '_> {
    fn run(&mut self) -> Result<(), ParseError> {
        loop {
            let c = self.input.get(self.pointer).copied();
            match self.step(c)? {
                Step::Next => {
                    if self.pointer >= self.input.len() {
                        return Ok(());
                    }
                    self.pointer += 1;
                }
                Step::Again => {}
                Step::Return => return Ok(()),
            }
        }
    }

    /// Ein Durchlauf des Automaten mit dem Zeichen `c`; `None` ist das Ende
    /// der Eingabe (*EOF code point*).
    fn step(&mut self, c: Option<char>) -> Result<Step, ParseError> {
        match self.state {
            State::SchemeStart => self.scheme_start(c),
            State::Scheme => self.scheme(c),
            State::NoScheme => self.no_scheme(c),
            State::SpecialRelativeOrAuthority => Ok(self.special_relative_or_authority(c)),
            State::PathOrAuthority => Ok(self.path_or_authority(c)),
            State::Relative => Ok(self.relative(c)),
            State::RelativeSlash => Ok(self.relative_slash(c)),
            State::SpecialAuthoritySlashes => Ok(self.special_authority_slashes(c)),
            State::SpecialAuthorityIgnoreSlashes => Ok(self.special_authority_ignore_slashes(c)),
            State::Authority => self.authority(c),
            State::Host | State::Hostname => self.host(c),
            State::Port => self.port(c),
            State::File => Ok(self.file(c)),
            State::FileSlash => Ok(self.file_slash(c)),
            State::FileHost => self.file_host(c),
            State::PathStart => Ok(self.path_start(c)),
            State::Path => Ok(self.path(c)),
            State::OpaquePath => Ok(self.opaque_path(c)),
            State::Query => Ok(self.query(c)),
            State::Fragment => Ok(self.fragment(c)),
        }
    }

    fn fail(&mut self, error: ValidationError) -> Result<Step, ParseError> {
        self.report.error(error);
        Err(error.into())
    }

    /// Ob *remaining*, der Rest nach dem aktuellen Zeichen, mit `prefix`
    /// beginnt.
    fn remaining_starts_with(&self, prefix: &str) -> bool {
        let remaining = self.input.get(self.pointer + 1..).unwrap_or_default();
        let mut remaining = remaining.iter();
        prefix
            .chars()
            .all(|expected| remaining.next() == Some(&expected))
    }

    /// Ob die Eingabe ab dem aktuellen Zeichen mit einem
    /// Windows-Laufwerksbuchstaben beginnt.
    fn rest_starts_with_windows_drive_letter(&self) -> bool {
        starts_with_windows_drive_letter(self.input.get(self.pointer..).unwrap_or_default())
    }

    /// Meldet *invalid-URL-unit* für Zeichen, die keine URL-Einheit beginnen.
    fn check_url_unit(&mut self, c: char) {
        if !self.report.is_wanted() {
            return;
        }
        if !is_url_code_point(c) && c != '%' {
            self.report.error(ValidationError::InvalidUrlUnit);
        }
        if c == '%' {
            let mut remaining = self
                .input
                .get(self.pointer + 1..)
                .unwrap_or_default()
                .iter();
            let hex = |c: Option<&char>| c.is_some_and(char::is_ascii_hexdigit);
            if !(hex(remaining.next()) && hex(remaining.next())) {
                self.report.error(ValidationError::InvalidUrlUnit);
            }
        }
    }

    /// `#scheme-start-state`
    fn scheme_start(&mut self, c: Option<char>) -> Result<Step, ParseError> {
        if let Some(c) = c
            && c.is_ascii_alphabetic()
        {
            self.buffer.push(c.to_ascii_lowercase());
            self.state = State::Scheme;
            Ok(Step::Next)
        } else if self.state_override.is_none() {
            self.state = State::NoScheme;
            Ok(Step::Again)
        } else {
            Err(ParseError { cause: None })
        }
    }

    /// `#scheme-state`
    fn scheme(&mut self, c: Option<char>) -> Result<Step, ParseError> {
        if let Some(c) = c
            && (c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            self.buffer.push(c.to_ascii_lowercase());
            return Ok(Step::Next);
        }
        if c != Some(':') {
            if self.state_override.is_some() {
                return Err(ParseError { cause: None });
            }
            // Von vorn, ohne Schema.
            self.buffer.clear();
            self.state = State::NoScheme;
            self.pointer = 0;
            return Ok(Step::Again);
        }
        if self.state_override.is_some() {
            let url_is_special = self.url.is_special();
            let buffer_is_special = is_special_scheme(&self.buffer);
            if url_is_special != buffer_is_special {
                return Ok(Step::Return);
            }
            if (self.url.includes_credentials() || self.url.port.is_some()) && self.buffer == "file"
            {
                return Ok(Step::Return);
            }
            if self.url.scheme == "file" && self.url.host == Some(Host::Empty) {
                return Ok(Step::Return);
            }
        }
        self.url.scheme = std::mem::take(&mut self.buffer);
        if self.state_override.is_some() {
            if self.url.port == default_port(&self.url.scheme) {
                self.url.port = None;
            }
            return Ok(Step::Return);
        }
        if self.url.scheme == "file" {
            if !self.remaining_starts_with("//") {
                self.report
                    .error(ValidationError::SpecialSchemeMissingFollowingSolidus);
            }
            self.state = State::File;
        } else if self.url.is_special()
            && self.base.is_some_and(|base| base.scheme == self.url.scheme)
        {
            // Die Basis ist dann auch speziell und hat keinen opaken Pfad.
            self.state = State::SpecialRelativeOrAuthority;
        } else if self.url.is_special() {
            self.state = State::SpecialAuthoritySlashes;
        } else if self.remaining_starts_with("/") {
            self.state = State::PathOrAuthority;
            self.pointer += 1;
        } else {
            self.url.path = Path::Opaque(String::new());
            self.state = State::OpaquePath;
        }
        Ok(Step::Next)
    }

    /// `#no-scheme-state`
    fn no_scheme(&mut self, c: Option<char>) -> Result<Step, ParseError> {
        let Some(base) = self.base else {
            return self.fail(ValidationError::MissingSchemeNonRelativeUrl);
        };
        if base.has_opaque_path() {
            if c != Some('#') {
                return self.fail(ValidationError::MissingSchemeNonRelativeUrl);
            }
            self.url.scheme = base.scheme.clone();
            self.url.path = base.path.clone();
            self.url.query = base.query.clone();
            self.url.fragment = Some(String::new());
            self.state = State::Fragment;
            Ok(Step::Next)
        } else if base.scheme != "file" {
            self.state = State::Relative;
            Ok(Step::Again)
        } else {
            self.state = State::File;
            Ok(Step::Again)
        }
    }

    /// `#special-relative-or-authority-state`
    fn special_relative_or_authority(&mut self, c: Option<char>) -> Step {
        if c == Some('/') && self.remaining_starts_with("/") {
            self.state = State::SpecialAuthorityIgnoreSlashes;
            self.pointer += 1;
            Step::Next
        } else {
            self.report
                .error(ValidationError::SpecialSchemeMissingFollowingSolidus);
            self.state = State::Relative;
            Step::Again
        }
    }

    /// `#path-or-authority-state`
    fn path_or_authority(&mut self, c: Option<char>) -> Step {
        if c == Some('/') {
            self.state = State::Authority;
            Step::Next
        } else {
            self.state = State::Path;
            Step::Again
        }
    }

    /// Übernimmt Benutzername, Passwort, Host und Port der Basis.
    fn copy_authority_from_base(&mut self) {
        if let Some(base) = self.base {
            self.url.username = base.username.clone();
            self.url.password = base.password.clone();
            self.url.host = base.host.clone();
            self.url.port = base.port;
        }
    }

    /// `#relative-state`. Die Basis existiert hier und hat nicht das Schema
    /// `file`.
    fn relative(&mut self, c: Option<char>) -> Step {
        if let Some(base) = self.base {
            self.url.scheme = base.scheme.clone();
        }
        if c == Some('/') {
            self.state = State::RelativeSlash;
        } else if self.url.is_special() && c == Some('\\') {
            self.report.error(ValidationError::InvalidReverseSolidus);
            self.state = State::RelativeSlash;
        } else {
            self.copy_authority_from_base();
            if let Some(base) = self.base {
                self.url.path = base.path.clone();
                self.url.query = base.query.clone();
            }
            if c == Some('?') {
                self.url.query = Some(String::new());
                self.state = State::Query;
            } else if c == Some('#') {
                self.url.fragment = Some(String::new());
                self.state = State::Fragment;
            } else if c.is_some() {
                self.url.query = None;
                self.url.shorten_path();
                self.state = State::Path;
                return Step::Again;
            }
        }
        Step::Next
    }

    /// `#relative-slash-state`
    fn relative_slash(&mut self, c: Option<char>) -> Step {
        if self.url.is_special() && matches!(c, Some('/' | '\\')) {
            if c == Some('\\') {
                self.report.error(ValidationError::InvalidReverseSolidus);
            }
            self.state = State::SpecialAuthorityIgnoreSlashes;
            Step::Next
        } else if c == Some('/') {
            self.state = State::Authority;
            Step::Next
        } else {
            self.copy_authority_from_base();
            self.state = State::Path;
            Step::Again
        }
    }

    /// `#special-authority-slashes-state`
    fn special_authority_slashes(&mut self, c: Option<char>) -> Step {
        if c == Some('/') && self.remaining_starts_with("/") {
            self.state = State::SpecialAuthorityIgnoreSlashes;
            self.pointer += 1;
            Step::Next
        } else {
            self.report
                .error(ValidationError::SpecialSchemeMissingFollowingSolidus);
            self.state = State::SpecialAuthorityIgnoreSlashes;
            Step::Again
        }
    }

    /// `#special-authority-ignore-slashes-state`
    fn special_authority_ignore_slashes(&mut self, c: Option<char>) -> Step {
        if !matches!(c, Some('/' | '\\')) {
            self.state = State::Authority;
            Step::Again
        } else {
            self.report
                .error(ValidationError::SpecialSchemeMissingFollowingSolidus);
            Step::Next
        }
    }

    /// Ob `c` den Host oder den Port beendet: Ende, `/`, `?`, `#` und bei
    /// speziellen URLs `\`.
    fn ends_authority(&self, c: Option<char>) -> bool {
        matches!(c, None | Some('/' | '?' | '#')) || (self.url.is_special() && c == Some('\\'))
    }

    /// `#authority-state`
    fn authority(&mut self, c: Option<char>) -> Result<Step, ParseError> {
        if c == Some('@') {
            self.report.error(ValidationError::InvalidCredentials);
            if self.at_sign_seen {
                self.buffer.insert_str(0, "%40");
            }
            self.at_sign_seen = true;
            for code_point in self.buffer.chars() {
                if code_point == ':' && !self.password_token_seen {
                    self.password_token_seen = true;
                    continue;
                }
                let target = if self.password_token_seen {
                    &mut self.url.password
                } else {
                    &mut self.url.username
                };
                push_utf8_percent_encoded(code_point, USERINFO, target);
            }
            self.buffer.clear();
            Ok(Step::Next)
        } else if self.ends_authority(c) {
            if self.at_sign_seen && self.buffer.is_empty() {
                return self.fail(ValidationError::HostMissing);
            }
            // Zurück an den Anfang des Puffers: „decrease pointer by buffer's
            // code point length + 1“, danach erhöht die Schleife um 1.
            self.pointer = self.pointer.saturating_sub(self.buffer.chars().count());
            self.buffer.clear();
            self.state = State::Host;
            Ok(Step::Again)
        } else {
            if let Some(c) = c {
                self.buffer.push(c);
            }
            Ok(Step::Next)
        }
    }

    /// `#host-state` und `#hostname-state`
    fn host(&mut self, c: Option<char>) -> Result<Step, ParseError> {
        if self.state_override.is_some() && self.url.scheme == "file" {
            self.state = State::FileHost;
            Ok(Step::Again)
        } else if c == Some(':') && !self.inside_brackets {
            if self.buffer.is_empty() {
                return self.fail(ValidationError::HostMissing);
            }
            if self.state_override == Some(State::Hostname) {
                return Err(ParseError { cause: None });
            }
            let host = host::parse(&self.buffer, !self.url.is_special(), self.report)?;
            self.url.host = Some(host);
            self.buffer.clear();
            self.state = State::Port;
            Ok(Step::Next)
        } else if self.ends_authority(c) {
            if self.url.is_special() && self.buffer.is_empty() {
                return self.fail(ValidationError::HostMissing);
            }
            if self.state_override.is_some()
                && self.buffer.is_empty()
                && (self.url.includes_credentials() || self.url.port.is_some())
            {
                return Err(ParseError { cause: None });
            }
            let host = host::parse(&self.buffer, !self.url.is_special(), self.report)?;
            self.url.host = Some(host);
            self.buffer.clear();
            self.state = State::PathStart;
            if self.state_override.is_some() {
                return Ok(Step::Return);
            }
            Ok(Step::Again)
        } else {
            if c == Some('[') {
                self.inside_brackets = true;
            }
            if c == Some(']') {
                self.inside_brackets = false;
            }
            if let Some(c) = c {
                self.buffer.push(c);
            }
            Ok(Step::Next)
        }
    }

    /// `#port-state`
    fn port(&mut self, c: Option<char>) -> Result<Step, ParseError> {
        if let Some(c) = c
            && c.is_ascii_digit()
        {
            self.buffer.push(c);
            return Ok(Step::Next);
        }
        if !(self.ends_authority(c) || self.state_override.is_some()) {
            return self.fail(ValidationError::PortInvalid);
        }
        if !self.buffer.is_empty() {
            // Der Standard rechnet mit beliebig großen Zahlen; alles über
            // 65535 ist ohnehin zu groß.
            let value = self
                .buffer
                .chars()
                .filter_map(|c| c.to_digit(10))
                .fold(0u32, |value, digit| {
                    value.saturating_mul(10).saturating_add(digit)
                });
            let Ok(port) = u16::try_from(value) else {
                return self.fail(ValidationError::PortOutOfRange);
            };
            self.url.port = if default_port(&self.url.scheme) == Some(port) {
                None
            } else {
                Some(port)
            };
            self.buffer.clear();
            if self.state_override.is_some() {
                return Ok(Step::Return);
            }
        }
        if self.state_override.is_some() {
            return Err(ParseError { cause: None });
        }
        self.state = State::PathStart;
        Ok(Step::Again)
    }

    /// `#file-state`
    fn file(&mut self, c: Option<char>) -> Step {
        self.url.scheme = String::from("file");
        self.url.host = Some(Host::Empty);
        if matches!(c, Some('/' | '\\')) {
            if c == Some('\\') {
                self.report.error(ValidationError::InvalidReverseSolidus);
            }
            self.state = State::FileSlash;
            return Step::Next;
        }
        let Some(base) = self.base.filter(|base| base.scheme == "file") else {
            self.state = State::Path;
            return Step::Again;
        };
        self.url.host = base.host.clone();
        self.url.path = base.path.clone();
        self.url.query = base.query.clone();
        if c == Some('?') {
            self.url.query = Some(String::new());
            self.state = State::Query;
        } else if c == Some('#') {
            self.url.fragment = Some(String::new());
            self.state = State::Fragment;
        } else if c.is_some() {
            self.url.query = None;
            if !self.rest_starts_with_windows_drive_letter() {
                self.url.shorten_path();
            } else {
                self.report
                    .error(ValidationError::FileInvalidWindowsDriveLetter);
                self.url.path = Path::Segments(Vec::new());
            }
            self.state = State::Path;
            return Step::Again;
        }
        Step::Next
    }

    /// `#file-slash-state`
    fn file_slash(&mut self, c: Option<char>) -> Step {
        if matches!(c, Some('/' | '\\')) {
            if c == Some('\\') {
                self.report.error(ValidationError::InvalidReverseSolidus);
            }
            self.state = State::FileHost;
            return Step::Next;
        }
        if let Some(base) = self.base.filter(|base| base.scheme == "file") {
            self.url.host = base.host.clone();
            if !self.rest_starts_with_windows_drive_letter()
                && let Path::Segments(segments) = &base.path
                && let Some(first) = segments.first()
                && is_normalized_windows_drive_letter(first)
            {
                self.url.path.push(first.clone());
            }
        }
        self.state = State::Path;
        Step::Again
    }

    /// `#file-host-state`
    fn file_host(&mut self, c: Option<char>) -> Result<Step, ParseError> {
        if !matches!(c, None | Some('/' | '\\' | '?' | '#')) {
            if let Some(c) = c {
                self.buffer.push(c);
            }
            return Ok(Step::Next);
        }
        if self.state_override.is_none() && is_windows_drive_letter(&self.buffer) {
            // Der Puffer bleibt und wird im Pfad verwendet.
            self.report
                .error(ValidationError::FileInvalidWindowsDriveLetterHost);
            self.state = State::Path;
        } else if self.buffer.is_empty() {
            self.url.host = Some(Host::Empty);
            if self.state_override.is_some() {
                return Ok(Step::Return);
            }
            self.state = State::PathStart;
        } else {
            let mut host = host::parse(&self.buffer, !self.url.is_special(), self.report)?;
            if host == Host::Domain(String::from("localhost")) {
                host = Host::Empty;
            }
            self.url.host = Some(host);
            if self.state_override.is_some() {
                return Ok(Step::Return);
            }
            self.buffer.clear();
            self.state = State::PathStart;
        }
        Ok(Step::Again)
    }

    /// `#path-start-state`
    fn path_start(&mut self, c: Option<char>) -> Step {
        if self.url.is_special() {
            if c == Some('\\') {
                self.report.error(ValidationError::InvalidReverseSolidus);
            }
            self.state = State::Path;
            if !matches!(c, Some('/' | '\\')) {
                return Step::Again;
            }
        } else if self.state_override.is_none() && c == Some('?') {
            self.url.query = Some(String::new());
            self.state = State::Query;
        } else if self.state_override.is_none() && c == Some('#') {
            self.url.fragment = Some(String::new());
            self.state = State::Fragment;
        } else if c.is_some() {
            self.state = State::Path;
            if c != Some('/') {
                return Step::Again;
            }
        } else if self.state_override.is_some() && self.url.host.is_none() {
            self.url.path.push(String::new());
        }
        Step::Next
    }

    /// `#path-state`
    fn path(&mut self, c: Option<char>) -> Step {
        let special = self.url.is_special();
        let slash = c == Some('/') || (special && c == Some('\\'));
        let ends_segment =
            c.is_none() || slash || (self.state_override.is_none() && matches!(c, Some('?' | '#')));
        if !ends_segment {
            if let Some(c) = c {
                self.check_url_unit(c);
                push_utf8_percent_encoded(c, PATH, &mut self.buffer);
            }
            return Step::Next;
        }
        if special && c == Some('\\') {
            self.report.error(ValidationError::InvalidReverseSolidus);
        }
        if is_double_dot_segment(&self.buffer) {
            self.url.shorten_path();
            if !slash {
                self.url.path.push(String::new());
            }
        } else if is_single_dot_segment(&self.buffer) {
            if !slash {
                self.url.path.push(String::new());
            }
        } else {
            let mut segment = std::mem::take(&mut self.buffer);
            if self.url.scheme == "file"
                && self.url.path.is_empty()
                && is_windows_drive_letter(&segment)
            {
                // Die Quirk mit Windows-Laufwerksbuchstaben: `c|` wird zu `c:`.
                segment = segment.chars().take(1).chain([':']).collect();
            }
            self.url.path.push(segment);
        }
        self.buffer.clear();
        if c == Some('?') {
            self.url.query = Some(String::new());
            self.state = State::Query;
        }
        if c == Some('#') {
            self.url.fragment = Some(String::new());
            self.state = State::Fragment;
        }
        Step::Next
    }

    /// `#cannot-be-a-base-url-path-state` (*opaque path state*)
    fn opaque_path(&mut self, c: Option<char>) -> Step {
        match c {
            Some('?') => {
                self.url.query = Some(String::new());
                self.state = State::Query;
            }
            Some('#') => {
                self.url.fragment = Some(String::new());
                self.state = State::Fragment;
            }
            Some(' ') => {
                self.report.error(ValidationError::InvalidUrlUnit);
                let encoded = self.remaining_starts_with("?") || self.remaining_starts_with("#");
                if let Path::Opaque(path) = &mut self.url.path {
                    path.push_str(if encoded { "%20" } else { " " });
                }
            }
            Some(c) => {
                self.check_url_unit(c);
                if let Path::Opaque(path) = &mut self.url.path {
                    push_utf8_percent_encoded(c, C0_CONTROL, path);
                }
            }
            None => {}
        }
        Step::Next
    }

    /// `#query-state`
    fn query(&mut self, c: Option<char>) -> Step {
        if self.encoding != Encoding::Utf8
            && (!self.url.is_special() || matches!(self.url.scheme.as_str(), "ws" | "wss"))
        {
            self.encoding = Encoding::Utf8;
        }
        if (self.state_override.is_none() && c == Some('#')) || c.is_none() {
            let set = if self.url.is_special() {
                SPECIAL_QUERY
            } else {
                QUERY
            };
            let encoded = percent_encode_after_encoding(self.encoding, &self.buffer, set, false);
            self.url.query.get_or_insert_default().push_str(&encoded);
            self.buffer.clear();
            if c == Some('#') {
                self.url.fragment = Some(String::new());
                self.state = State::Fragment;
            }
        } else if let Some(c) = c {
            self.check_url_unit(c);
            self.buffer.push(c);
        }
        Step::Next
    }

    /// `#fragment-state`
    fn fragment(&mut self, c: Option<char>) -> Step {
        if let Some(c) = c {
            self.check_url_unit(c);
            push_utf8_percent_encoded(c, FRAGMENT, self.url.fragment.get_or_insert_default());
        }
        Step::Next
    }
}

/// `#windows-drive-letter`: zwei Zeichen, ein ASCII-Buchstabe und `:` oder `|`.
fn is_windows_drive_letter(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(letter), Some(':' | '|'), None) if letter.is_ascii_alphabetic()
    )
}

/// `#normalized-windows-drive-letter`
pub(crate) fn is_normalized_windows_drive_letter(text: &str) -> bool {
    is_windows_drive_letter(text) && text.ends_with(':')
}

/// `#start-with-a-windows-drive-letter`
fn starts_with_windows_drive_letter(text: &[char]) -> bool {
    match text {
        [letter, ':' | '|'] => letter.is_ascii_alphabetic(),
        [letter, ':' | '|', third, ..] => {
            letter.is_ascii_alphabetic() && matches!(third, '/' | '\\' | '?' | '#')
        }
        _ => false,
    }
}

/// `#single-dot-path-segment`
fn is_single_dot_segment(segment: &str) -> bool {
    segment == "." || segment.eq_ignore_ascii_case("%2e")
}

/// `#double-dot-path-segment`
fn is_double_dot_segment(segment: &str) -> bool {
    segment == ".."
        || [".%2e", "%2e.", "%2e%2e"]
            .iter()
            .any(|form| segment.eq_ignore_ascii_case(form))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_drive_letters() {
        // Die Beispiele aus dem Standard.
        let starts =
            |text: &str| starts_with_windows_drive_letter(&text.chars().collect::<Vec<_>>());
        assert!(starts("c:"));
        assert!(starts("c:/"));
        assert!(!starts("c:a"));
        assert!(starts("C|#"));
        assert!(!starts("1:"));
        assert!(is_windows_drive_letter("c|") && !is_normalized_windows_drive_letter("c|"));
        assert!(is_normalized_windows_drive_letter("Z:"));
        assert!(!is_windows_drive_letter("c:/"));
    }

    #[test]
    fn dot_segments() {
        assert!(is_single_dot_segment(".") && is_single_dot_segment("%2E"));
        assert!(!is_single_dot_segment(".."));
        for segment in ["..", ".%2E", "%2e.", "%2E%2e"] {
            assert!(is_double_dot_segment(segment), "{segment}");
        }
        assert!(!is_double_dot_segment("..."));
    }
}
