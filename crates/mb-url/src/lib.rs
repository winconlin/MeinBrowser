//! URL-Parser nach dem WHATWG URL Standard.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.4. Umgesetzt nach dem Text des
//! Standards im Commit `845bafca22fb0c5a42c4bf384a9bdea525a2f10d` von
//! `whatwg/url`, von Hand und Schritt für Schritt; die Kommentare `#…` nennen
//! die Anker der Abschnitte. Geprüft wird mit den URL-Testdaten der WPT.
//!
//! [`Url::parse`] ist der *basic URL parser*. Den *URL parser* mit dem
//! Eintrag für `blob:`-URLs gibt es erst mit dem Blob-Speicher der File API.
//! Die Getter und Setter der Klasse `URL` sind Methoden von [`Url`]; nur
//! `host` und `port` heißen als Getter [`Url::host_string`] und
//! [`Url::port_string`], weil [`Url::host`] und [`Url::port`] die Bestandteile
//! selbst liefern.

#![forbid(unsafe_code)]

mod error;
pub mod form_urlencoded;
mod host;
mod parser;
pub mod percent;

use std::sync::atomic::{AtomicU64, Ordering};

pub use error::{ParseError, ValidationError};
pub use host::{Host, domain_to_unicode};
use mb_encoding::Encoding;

use crate::error::Report;
use crate::parser::{State, basic_url_parse, is_normalized_windows_drive_letter};
use crate::percent::{USERINFO, utf8_percent_encode};

/// Eine URL (`#concept-url`), der *URL record* des Standards.
#[derive(Clone, Debug)]
pub struct Url {
    scheme: String,
    username: String,
    password: String,
    host: Option<Host>,
    port: Option<u16>,
    path: Path,
    query: Option<String>,
    fragment: Option<String>,
}

/// Der Pfad einer URL (`#url-path`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Path {
    /// Ein opaker Pfad, etwa bei `mailto:` oder `data:`.
    Opaque(String),
    /// Eine Liste von Segmenten, die kein `/` enthalten.
    Segments(Vec<String>),
}

impl Path {
    fn push(&mut self, segment: String) {
        if let Path::Segments(segments) = self {
            segments.push(segment);
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Path::Segments(segments) if segments.is_empty())
    }
}

/// Der *URL path serializer* (`#url-path-serializer`).
impl std::fmt::Display for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Path::Opaque(path) => f.write_str(path),
            Path::Segments(segments) => segments
                .iter()
                .try_for_each(|segment| write!(f, "/{segment}")),
        }
    }
}

/// `#special-scheme`
fn is_special_scheme(scheme: &str) -> bool {
    matches!(scheme, "ftp" | "file" | "http" | "https" | "ws" | "wss")
}

/// `#default-port`
fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "ftp" => Some(21),
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        _ => None,
    }
}

impl Url {
    /// Eine neue, leere URL, wie sie der Parser anlegt.
    fn new() -> Url {
        Url {
            scheme: String::new(),
            username: String::new(),
            password: String::new(),
            host: None,
            port: None,
            path: Path::Segments(Vec::new()),
            query: None,
            fragment: None,
        }
    }

    /// Der *basic URL parser* mit UTF-8 und `base` als Basis-URL.
    pub fn parse(input: &str, base: Option<&Url>) -> Result<Url, ParseError> {
        Url::parse_with_encoding(input, base, Encoding::Utf8)
    }

    /// Der *basic URL parser* mit einer Kodierung für die Query, wie HTML
    /// sie für Dokumente in anderen Kodierungen verlangt.
    pub fn parse_with_encoding(
        input: &str,
        base: Option<&Url>,
        encoding: Encoding,
    ) -> Result<Url, ParseError> {
        let mut url = Url::new();
        basic_url_parse(
            input,
            base,
            encoding,
            &mut url,
            None,
            &mut Report::new(None),
        )?;
        Ok(url)
    }

    /// Wie [`Url::parse_with_encoding`], dazu alle Validierungsfehler in
    /// `errors`, auch der, der zum Abbruch führt.
    pub fn parse_with_errors(
        input: &str,
        base: Option<&Url>,
        encoding: Encoding,
        errors: &mut Vec<ValidationError>,
    ) -> Result<Url, ParseError> {
        let mut url = Url::new();
        let mut report = Report::new(Some(errors));
        basic_url_parse(input, base, encoding, &mut url, None, &mut report)?;
        Ok(url)
    }

    /// Der *API URL parser* (`#api-url-parser`), wie `new URL(url, base)`:
    /// Die Basis ist selbst eine Zeichenkette und muss sich parsen lassen.
    pub fn api_parse(url: &str, base: Option<&str>) -> Result<Url, ParseError> {
        let parsed_base = base.map(|base| Url::parse(base, None)).transpose()?;
        Url::parse(url, parsed_base.as_ref())
    }

    /// Der Parser mit *state override*, für die Setter. Ein Fehlschlag lässt
    /// die URL so, wie der Standard sie bis dahin geändert hat.
    fn parse_with_state_override(&mut self, input: &str, state: State) {
        let _ = basic_url_parse(
            input,
            None,
            Encoding::Utf8,
            self,
            Some(state),
            &mut Report::new(None),
        );
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// Der Benutzername, prozentkodiert. Zugleich der Getter `username`.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Das Passwort, prozentkodiert. Zugleich der Getter `password`.
    pub fn password(&self) -> &str {
        &self.password
    }

    pub fn host(&self) -> Option<&Host> {
        self.host.as_ref()
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }

    /// `#is-special`
    pub fn is_special(&self) -> bool {
        is_special_scheme(&self.scheme)
    }

    /// `#include-credentials`
    pub fn includes_credentials(&self) -> bool {
        !self.username.is_empty() || !self.password.is_empty()
    }

    /// `#url-opaque-path`
    pub fn has_opaque_path(&self) -> bool {
        matches!(self.path, Path::Opaque(_))
    }

    /// `#cannot-have-a-username-password-port`
    pub fn cannot_have_username_password_port(&self) -> bool {
        matches!(self.host, None | Some(Host::Empty)) || self.scheme == "file"
    }

    /// *Shorten a url's path* (`#shorten-a-urls-path`).
    fn shorten_path(&mut self) {
        let is_file = self.scheme == "file";
        if let Path::Segments(segments) = &mut self.path {
            if is_file
                && let [only] = segments.as_slice()
                && is_normalized_windows_drive_letter(only)
            {
                return;
            }
            segments.pop();
        }
    }

    /// Der *URL serializer* (`#concept-url-serializer`).
    pub fn serialize(&self, exclude_fragment: bool) -> String {
        let mut output = format!("{}:", self.scheme);
        if let Some(host) = &self.host {
            output.push_str("//");
            if self.includes_credentials() {
                output.push_str(&self.username);
                if !self.password.is_empty() {
                    output.push(':');
                    output.push_str(&self.password);
                }
                output.push('@');
            }
            output.push_str(&host.to_string());
            if let Some(port) = self.port {
                output.push(':');
                output.push_str(&port.to_string());
            }
        }
        if self.host.is_none()
            && let Path::Segments(segments) = &self.path
            && segments.len() > 1
            && segments.first().is_some_and(String::is_empty)
        {
            output.push_str("/.");
        }
        output.push_str(&self.path.to_string());
        if let Some(query) = &self.query {
            output.push('?');
            output.push_str(query);
        }
        if !exclude_fragment && let Some(fragment) = &self.fragment {
            output.push('#');
            output.push_str(fragment);
        }
        output
    }

    /// Ob zwei URLs gleich sind (`#concept-url-equals`): Ihre
    /// Serialisierungen sind gleich, wahlweise ohne Fragment.
    pub fn equals(&self, other: &Url, exclude_fragments: bool) -> bool {
        self.serialize(exclude_fragments) == other.serialize(exclude_fragments)
    }

    /// Der Ursprung (`#concept-url-origin`). Für `file:` ist er opak, wie es
    /// der Standard im Zweifel empfiehlt.
    pub fn origin(&self) -> Origin {
        match self.scheme.as_str() {
            "blob" => match Url::parse(&self.path.to_string(), None) {
                Ok(path_url) if matches!(path_url.scheme(), "http" | "https" | "file") => {
                    path_url.origin()
                }
                _ => Origin::Opaque(OpaqueOrigin::unique()),
            },
            "ftp" | "http" | "https" | "ws" | "wss" => Origin::Tuple {
                scheme: self.scheme.clone(),
                // Diese Schemata haben immer einen Host.
                host: self.host.clone().unwrap_or(Host::Empty),
                port: self.port,
            },
            _ => Origin::Opaque(OpaqueOrigin::unique()),
        }
    }

    /// Der Getter `href`: die Serialisierung.
    pub fn href(&self) -> String {
        self.serialize(false)
    }

    /// Der Setter `href`. Scheitert das Parsen, bleibt die URL, wie sie ist
    /// (die API wirft dann einen `TypeError`).
    pub fn set_href(&mut self, value: &str) -> Result<(), ParseError> {
        *self = Url::parse(value, None)?;
        Ok(())
    }

    /// Der Getter `protocol`: das Schema mit `:`.
    pub fn protocol(&self) -> String {
        format!("{}:", self.scheme)
    }

    /// Der Setter `protocol`.
    pub fn set_protocol(&mut self, value: &str) {
        self.parse_with_state_override(&format!("{value}:"), State::SchemeStart);
    }

    /// Der Setter `username`.
    pub fn set_username(&mut self, value: &str) {
        if self.cannot_have_username_password_port() {
            return;
        }
        self.username = utf8_percent_encode(value, USERINFO);
    }

    /// Der Setter `password`.
    pub fn set_password(&mut self, value: &str) {
        if self.cannot_have_username_password_port() {
            return;
        }
        self.password = utf8_percent_encode(value, USERINFO);
    }

    /// Der Getter `host`: Host und Port.
    pub fn host_string(&self) -> String {
        match (&self.host, self.port) {
            (None, _) => String::new(),
            (Some(host), None) => host.to_string(),
            (Some(host), Some(port)) => format!("{host}:{port}"),
        }
    }

    /// Der Setter `host`. Ohne Port im Wert bleibt der Port, wie er ist.
    pub fn set_host(&mut self, value: &str) {
        if self.has_opaque_path() {
            return;
        }
        self.parse_with_state_override(value, State::Host);
    }

    /// Der Getter `hostname`.
    pub fn hostname(&self) -> String {
        self.host.as_ref().map(Host::to_string).unwrap_or_default()
    }

    /// Der Setter `hostname`.
    pub fn set_hostname(&mut self, value: &str) {
        if self.has_opaque_path() {
            return;
        }
        self.parse_with_state_override(value, State::Hostname);
    }

    /// Der Getter `port`.
    pub fn port_string(&self) -> String {
        self.port.map(|port| port.to_string()).unwrap_or_default()
    }

    /// Der Setter `port`.
    pub fn set_port(&mut self, value: &str) {
        if self.cannot_have_username_password_port() {
            return;
        }
        if value.is_empty() {
            self.port = None;
        } else {
            self.parse_with_state_override(value, State::Port);
        }
    }

    /// Der Getter `pathname`.
    pub fn pathname(&self) -> String {
        self.path.to_string()
    }

    /// Der Setter `pathname`.
    pub fn set_pathname(&mut self, value: &str) {
        if self.has_opaque_path() {
            return;
        }
        self.path = Path::Segments(Vec::new());
        self.parse_with_state_override(value, State::PathStart);
    }

    /// Der Getter `search`: die Query mit `?`, leer, wenn sie fehlt oder leer
    /// ist.
    pub fn search(&self) -> String {
        match self.query.as_deref() {
            None | Some("") => String::new(),
            Some(query) => format!("?{query}"),
        }
    }

    /// Der Setter `search`. Die Liste von `searchParams` aus der neuen Query
    /// zu bilden, ist Sache der API darüber.
    pub fn set_search(&mut self, value: &str) {
        if value.is_empty() {
            self.query = None;
            return;
        }
        let input = value.strip_prefix('?').unwrap_or(value);
        self.query = Some(String::new());
        self.parse_with_state_override(input, State::Query);
    }

    /// Der Getter `hash`: das Fragment mit `#`, leer, wenn es fehlt oder leer
    /// ist.
    pub fn hash(&self) -> String {
        match self.fragment.as_deref() {
            None | Some("") => String::new(),
            Some(fragment) => format!("#{fragment}"),
        }
    }

    /// Der Setter `hash`.
    pub fn set_hash(&mut self, value: &str) {
        if value.is_empty() {
            self.fragment = None;
            return;
        }
        let input = value.strip_prefix('#').unwrap_or(value);
        self.fragment = Some(String::new());
        self.parse_with_state_override(input, State::Fragment);
    }
}

/// Gleichheit nach dem Standard: gleiche Serialisierung.
impl PartialEq for Url {
    fn eq(&self, other: &Url) -> bool {
        self.equals(other, false)
    }
}

impl Eq for Url {}

impl std::fmt::Display for Url {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.href())
    }
}

/// Ein Ursprung (*origin* aus HTML): opak oder ein Tupel aus Schema, Host
/// und Port. Die Domain des Tupels ist bei URLs immer null und fehlt hier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Opaque(OpaqueOrigin),
    Tuple {
        scheme: String,
        host: Host,
        port: Option<u16>,
    },
}

/// Ein opaker Ursprung. Jeder neue ist nur mit sich selbst gleich.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OpaqueOrigin(u64);

impl OpaqueOrigin {
    pub fn unique() -> OpaqueOrigin {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        OpaqueOrigin(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// Die Serialisierung eines Ursprungs aus HTML: `null` für opake Ursprünge,
/// sonst `scheme://host` mit `:port`, falls es einen gibt.
impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::Opaque(_) => f.write_str("null"),
            Origin::Tuple {
                scheme,
                host,
                port: None,
            } => write!(f, "{scheme}://{host}"),
            Origin::Tuple {
                scheme,
                host,
                port: Some(port),
            } => write!(f, "{scheme}://{host}:{port}"),
        }
    }
}

#[cfg(test)]
mod tests;
