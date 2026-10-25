//! Der Parser: Prolog, Elemente, Text und Verweise nach XML 1.0 (Fifth
//! Edition), die DTD in `parser/dtd.rs`.
//!
//! Der Text liegt als Folge von Zeichen in Rahmen ([`Frame`]): unten das
//! Dokument, darüber der Ersetzungstext jeder Entität, in der der Parser
//! gerade liest. Jedes Stück Markup liegt ganz in einem Rahmen; die
//! Lesefunktionen sehen nur den obersten und melden an seinem Ende kein
//! Zeichen mehr.

mod dtd;

use std::rc::Rc;

use mb_base::DomString;
use mb_dom::{Attribute, Document, NodeId};

use crate::XmlError;
use crate::chars::{is_char, is_name_char, is_name_start_char, is_ncname, is_qname, is_space};
use crate::namespaces::{Name, Scopes};

use dtd::{Dtd, Reference};

/// Ein fataler Fehler (XML 1.0, Abschnitt 1.2): Danach wird nicht weiter
/// gelesen.
pub(crate) struct Fatal(pub(crate) &'static str);

type Result<T> = std::result::Result<T, Fatal>;

/// Eine Entität als Nummer in ihrer Tabelle in der DTD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntityKey {
    General(usize),
    Parameter(usize),
}

struct Frame {
    text: Rc<[char]>,
    pos: usize,
    /// Die Entität, deren Ersetzungstext der Rahmen enthält; beim Dokument
    /// keine.
    entity: Option<EntityKey>,
    /// Die Zahl der offenen Elemente beim Betreten. Elemente müssen in der
    /// Entität enden, in der sie beginnen (Abschnitt 4.3.2).
    depth: usize,
}

/// Ein offenes Element.
struct Open {
    name: String,
    node: NodeId,
    /// Die Stelle in den Namensraumbindungen vor seinen Deklarationen.
    scope: usize,
}

/// Wie viele Zeichen Entitäten insgesamt einsetzen dürfen, bevor der
/// Parser aufgibt: so viel, wie ein Dokument vernünftigerweise braucht,
/// aber nicht die Milliarden einer Entitätenbombe.
fn expansion_budget(length: usize) -> usize {
    length.saturating_mul(100).saturating_add(8 << 20)
}

pub(crate) struct Parser {
    frames: Vec<Frame>,
    namespaces: bool,
    document: Document,
    dtd: Dtd,
    standalone: bool,
    open: Vec<Open>,
    scopes: Scopes,
    /// Text, der noch nicht im Baum steht. Text über Entitätsgrenzen
    /// hinweg wird so ein Knoten.
    text: String,
    budget: usize,
}

/// Liest das Dokument `text` in `document`. Den ersten fatalen Fehler gibt
/// es zurück; bis dahin steht der Baum im Dokument.
pub(crate) fn run(
    document: Document,
    text: &str,
    namespaces: bool,
) -> (Document, Option<XmlError>) {
    // Abschnitt 2.11: Zeilenenden werden zu U+000A.
    let mut chars: Vec<char> = Vec::with_capacity(text.len());
    let mut input = text.chars().peekable();
    while let Some(c) = input.next() {
        if c == '\r' {
            input.next_if_eq(&'\n');
            chars.push('\n');
        } else {
            chars.push(c);
        }
    }
    // Abschnitt 2.2: Das erste Zeichen, das kein `Char` ist, beendet das
    // Dokument mit einem Fehler. Bis dahin liest der Parser normal.
    let illegal = chars.iter().position(|&c| !is_char(c));
    if let Some(end) = illegal {
        chars.truncate(end);
    }
    let budget = expansion_budget(chars.len());
    let mut parser = Parser {
        frames: vec![Frame {
            text: chars.into(),
            pos: 0,
            entity: None,
            depth: 0,
        }],
        namespaces,
        document,
        dtd: Dtd::default(),
        standalone: false,
        open: Vec::new(),
        scopes: Scopes::default(),
        text: String::new(),
        budget,
    };
    let result = parser.document_entity();
    parser.flush_text();
    let at_end = parser
        .frames
        .first()
        .is_some_and(|frame| frame.pos >= frame.text.len());
    let message = match result {
        Ok(()) => illegal.map(|_| ILLEGAL_CHARACTER),
        Err(_) if illegal.is_some() && at_end => Some(ILLEGAL_CHARACTER),
        Err(Fatal(message)) => Some(message),
    };
    let error = message.map(|message| parser.error(message));
    (parser.document, error)
}

const ILLEGAL_CHARACTER: &str = "unzulässiges Zeichen";

impl Parser {
    // Lesen im obersten Rahmen

    fn peek(&self) -> Option<char> {
        let frame = self.frames.last()?;
        frame.text.get(frame.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        let frame = self.frames.last()?;
        frame.text.get(frame.pos.checked_add(offset)?).copied()
    }

    fn starts_with(&self, text: &str) -> bool {
        text.chars()
            .enumerate()
            .all(|(offset, c)| self.peek_at(offset) == Some(c))
    }

    fn advance(&mut self, count: usize) {
        if let Some(frame) = self.frames.last_mut() {
            frame.pos = frame.pos.saturating_add(count).min(frame.text.len());
        }
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.advance(1);
        Some(c)
    }

    fn eat(&mut self, text: &str) -> bool {
        let found = self.starts_with(text);
        if found {
            self.advance(text.chars().count());
        }
        found
    }

    fn expect(&mut self, text: &str, message: &'static str) -> Result<()> {
        if self.eat(text) {
            Ok(())
        } else {
            Err(Fatal(message))
        }
    }

    /// Überspringt `S?` und sagt, ob Leerraum da war.
    fn skip_space(&mut self) -> bool {
        let mut found = false;
        while self.peek().is_some_and(is_space) {
            self.advance(1);
            found = true;
        }
        found
    }

    fn require_space(&mut self) -> Result<()> {
        if self.skip_space() {
            Ok(())
        } else {
            Err(Fatal("Leerraum fehlt"))
        }
    }

    /// `Name`
    fn name(&mut self) -> Result<String> {
        let first = self
            .peek()
            .filter(|&c| is_name_start_char(c))
            .ok_or(Fatal("Name erwartet"))?;
        let mut name = String::from(first);
        self.advance(1);
        while let Some(c) = self.peek().filter(|&c| is_name_char(c)) {
            name.push(c);
            self.advance(1);
        }
        Ok(name)
    }

    /// Ein Name von Element oder Attribut: mit Namensräumen ein `QName`.
    fn qualified_name(&mut self) -> Result<String> {
        let name = self.name()?;
        if self.namespaces && !is_qname(&name) {
            return Err(Fatal("Name ist kein QName"));
        }
        Ok(name)
    }

    /// Ein Name von Entität, Notation oder Verarbeitungsanweisung: mit
    /// Namensräumen ohne Doppelpunkt (Namespaces, Abschnitt 7).
    fn nc_name(&mut self) -> Result<String> {
        let name = self.name()?;
        if self.namespaces && !is_ncname(&name) {
            return Err(Fatal("Name mit Doppelpunkt"));
        }
        Ok(name)
    }

    /// Zeile und Spalte der Stelle im Dokument, an der der Parser steht.
    fn error(&self, message: &'static str) -> XmlError {
        let (line, column) = match self.frames.first() {
            Some(frame) => {
                let before = frame.text.get(..frame.pos).unwrap_or_default();
                let line = before.iter().filter(|&&c| c == '\n').count();
                let column = before.iter().rev().take_while(|&&c| c != '\n').count();
                (line.saturating_add(1), column.saturating_add(1))
            }
            None => (1, 1),
        };
        XmlError {
            line,
            column,
            message,
        }
    }

    /// Betritt den Ersetzungstext einer Entität („No Recursion“).
    fn enter(&mut self, key: EntityKey, text: Rc<[char]>) -> Result<()> {
        if self.frames.iter().any(|frame| frame.entity == Some(key)) {
            return Err(Fatal("Entität verweist auf sich selbst"));
        }
        self.budget = self
            .budget
            .checked_sub(text.len())
            .ok_or(Fatal("Entitäten setzen zu viel Text ein"))?;
        self.frames.push(Frame {
            text,
            pos: 0,
            entity: Some(key),
            depth: self.open.len(),
        });
        Ok(())
    }

    /// Ob der Parser im Ersetzungstext einer Parameter-Entität liest.
    fn in_parameter_entity(&self) -> bool {
        self.frames
            .iter()
            .any(|frame| matches!(frame.entity, Some(EntityKey::Parameter(_))))
    }

    // Baum

    /// Der Knoten, an den neue Knoten kommen. Bei einem `template` ist das
    /// sein Inhalt (HTML Standard, „Parsing XML documents“).
    fn insertion_parent(&self) -> NodeId {
        match self.open.last() {
            Some(open) => self
                .document
                .template_contents(open.node)
                .unwrap_or(open.node),
            None => self.document.root(),
        }
    }

    fn insert(&mut self, node: NodeId) {
        let parent = self.insertion_parent();
        let inserted = self.document.append(parent, node);
        debug_assert!(inserted.is_ok(), "Knoten passt an seine Stelle");
    }

    fn flush_text(&mut self) {
        if self.text.is_empty() {
            return;
        }
        let data = DomString::from(self.text.as_str());
        self.text.clear();
        let node = self.document.create_text(data);
        self.insert(node);
    }

    // Dokument

    /// `document ::= prolog element Misc*`
    fn document_entity(&mut self) -> Result<()> {
        if self.starts_with("<?xml") && self.peek_at(5).is_some_and(is_space) {
            self.xml_declaration()?;
        }
        self.misc()?;
        if self.starts_with("<!DOCTYPE") {
            self.doctype()?;
            self.misc()?;
        }
        if self.peek() != Some('<') || !self.peek_at(1).is_some_and(is_name_start_char) {
            return Err(Fatal("Wurzelelement erwartet"));
        }
        self.content()?;
        self.misc()?;
        if self.peek().is_some() {
            return Err(Fatal("nach dem Wurzelelement darf nur noch Misc stehen"));
        }
        Ok(())
    }

    /// `XMLDecl`, mit `<?xml` und Leerraum am Anfang.
    fn xml_declaration(&mut self) -> Result<()> {
        self.advance(5);
        self.skip_space();
        self.expect("version", "XML-Deklaration ohne version")?;
        let version = self.pseudo_attribute()?;
        // Abschnitt 2.8: Jede Version 1.x wird wie 1.0 gelesen.
        let known = version
            .strip_prefix("1.")
            .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()));
        if !known {
            return Err(Fatal("unbekannte XML-Version"));
        }
        let mut space = self.skip_space();
        if space && self.eat("encoding") {
            // Die Kodierung selbst hat `encoding.rs` schon geprüft.
            let name = self.pseudo_attribute()?;
            if !crate::encoding::is_encoding_name(name.as_bytes()) {
                return Err(Fatal("ungültiger Name der Zeichenkodierung"));
            }
            space = self.skip_space();
        }
        if space && self.eat("standalone") {
            match self.pseudo_attribute()?.as_str() {
                "yes" => self.standalone = true,
                "no" => {}
                _ => return Err(Fatal("standalone ist weder yes noch no")),
            }
            self.skip_space();
        }
        self.expect("?>", "XML-Deklaration nicht abgeschlossen")
    }

    /// `Eq` und ein Wert in Anführungszeichen, in der XML-Deklaration.
    fn pseudo_attribute(&mut self) -> Result<String> {
        self.skip_space();
        self.expect("=", "„=“ erwartet")?;
        self.skip_space();
        let quote = self.quote()?;
        let mut value = String::new();
        loop {
            match self.next() {
                Some(c) if c == quote => return Ok(value),
                Some(c) => value.push(c),
                None => return Err(Fatal("Wert nicht abgeschlossen")),
            }
        }
    }

    fn quote(&mut self) -> Result<char> {
        self.next()
            .filter(|&c| c == '"' || c == '\'')
            .ok_or(Fatal("Anführungszeichen erwartet"))
    }

    /// `Misc*`: Kommentare, Verarbeitungsanweisungen und Leerraum vor und
    /// nach dem Wurzelelement. Leerraum kommt nicht in den Baum.
    fn misc(&mut self) -> Result<()> {
        loop {
            self.skip_space();
            if self.starts_with("<!--") {
                let data = self.comment()?;
                let node = self.document.create_comment(DomString::from(data.as_str()));
                self.insert(node);
            } else if self.starts_with("<?") {
                let (target, data) = self.processing_instruction()?;
                let node = self.document.create_processing_instruction(
                    DomString::from(target.as_str()),
                    DomString::from(data.as_str()),
                );
                self.insert(node);
            } else {
                return Ok(());
            }
        }
    }

    /// `Comment`, ab `<!--`.
    fn comment(&mut self) -> Result<String> {
        self.advance(4);
        let mut data = String::new();
        loop {
            match self.next() {
                Some('-') if self.peek() == Some('-') => {
                    self.advance(1);
                    return if self.eat(">") {
                        Ok(data)
                    } else {
                        Err(Fatal("„--“ im Kommentar"))
                    };
                }
                Some(c) => data.push(c),
                None => return Err(Fatal("Kommentar nicht abgeschlossen")),
            }
        }
    }

    /// `PI`, ab `<?`. Der Leerraum nach dem Ziel gehört nicht zu den Daten.
    fn processing_instruction(&mut self) -> Result<(String, String)> {
        self.advance(2);
        let target = self.nc_name()?;
        if target.eq_ignore_ascii_case("xml") {
            return Err(Fatal("Verarbeitungsanweisung mit dem Ziel xml"));
        }
        let mut data = String::new();
        if self.eat("?>") {
            return Ok((target, data));
        }
        self.require_space()?;
        loop {
            if self.eat("?>") {
                return Ok((target, data));
            }
            match self.next() {
                Some(c) => data.push(c),
                None => return Err(Fatal("Verarbeitungsanweisung nicht abgeschlossen")),
            }
        }
    }

    // Inhalt

    /// Das Wurzelelement mit seinem Inhalt (`element` und `content`).
    fn content(&mut self) -> Result<()> {
        loop {
            let Some(c) = self.peek() else {
                let depth = self.frames.last().map_or(0, |frame| frame.depth);
                if self.frames.len() > 1 {
                    if self.open.len() != depth {
                        return Err(Fatal("Element endet nicht in seiner Entität"));
                    }
                    self.frames.pop();
                    continue;
                }
                return Err(Fatal("Dokument endet in einem Element"));
            };
            match c {
                '<' if self.starts_with("</") => {
                    self.end_tag()?;
                    if self.open.is_empty() {
                        return Ok(());
                    }
                }
                '<' if self.starts_with("<!--") => {
                    let data = self.comment()?;
                    self.flush_text();
                    let node = self.document.create_comment(DomString::from(data.as_str()));
                    self.insert(node);
                }
                '<' if self.starts_with("<![CDATA[") => self.cdata_section()?,
                '<' if self.starts_with("<?") => {
                    let (target, data) = self.processing_instruction()?;
                    self.flush_text();
                    let node = self.document.create_processing_instruction(
                        DomString::from(target.as_str()),
                        DomString::from(data.as_str()),
                    );
                    self.insert(node);
                }
                '<' => {
                    self.start_tag()?;
                    if self.open.is_empty() {
                        return Ok(());
                    }
                }
                '&' => self.reference_in_content()?,
                _ => self.character_data()?,
            }
        }
    }

    /// `CharData`
    fn character_data(&mut self) -> Result<()> {
        while let Some(c) = self.peek() {
            if c == '<' || c == '&' {
                break;
            }
            if c == ']' && self.starts_with("]]>") {
                return Err(Fatal("„]]>“ im Text"));
            }
            self.text.push(c);
            self.advance(1);
        }
        Ok(())
    }

    /// `CDSect`, ab `<![CDATA[`.
    fn cdata_section(&mut self) -> Result<()> {
        self.advance(9);
        let mut data = String::new();
        while !self.eat("]]>") {
            data.push(
                self.next()
                    .ok_or(Fatal("CDATA-Abschnitt nicht abgeschlossen"))?,
            );
        }
        self.flush_text();
        let node = self
            .document
            .create_cdata_section(DomString::from(data.as_str()));
        self.insert(node);
        Ok(())
    }

    /// `CharRef`, ab `&#`.
    fn character_reference(&mut self) -> Result<char> {
        self.advance(2);
        let radix = if self.eat("x") { 16 } else { 10 };
        let mut value: u32 = 0;
        let mut digits = 0_usize;
        while let Some(digit) = self.peek().and_then(|c| c.to_digit(radix)) {
            value = value.saturating_mul(radix).saturating_add(digit);
            digits = digits.saturating_add(1);
            self.advance(1);
        }
        if digits == 0 || !self.eat(";") {
            return Err(Fatal("ungültiger Zeichenverweis"));
        }
        // „Legal Character“
        char::from_u32(value)
            .filter(|&c| is_char(c))
            .ok_or(Fatal("Zeichenverweis auf ein unzulässiges Zeichen"))
    }

    /// `EntityRef` ohne das `&`, aufgelöst nach „Entity Declared“.
    fn entity_reference(&mut self) -> Result<Reference> {
        let name = self.nc_name()?;
        self.expect(";", "„;“ nach dem Namen der Entität fehlt")?;
        // Verweise in Parameter-Entitäten sind von der Bedingung ausgenommen.
        let strict = self.dtd.strict(self.standalone) && !self.in_parameter_entity();
        match self.dtd.general_entity(&name, strict) {
            Reference::Undeclared if strict => Err(Fatal("Entität nicht deklariert")),
            reference => Ok(reference),
        }
    }

    /// `Reference` im Inhalt. Externe Entitäten liest der Parser nicht, er
    /// lässt sie aus (Abschnitt 4.4.3); ebenso nicht deklarierte, wenn das
    /// erlaubt ist.
    fn reference_in_content(&mut self) -> Result<()> {
        if self.peek_at(1) == Some('#') {
            let c = self.character_reference()?;
            self.text.push(c);
            return Ok(());
        }
        self.advance(1);
        match self.entity_reference()? {
            Reference::Characters(characters) => self.text.push_str(characters),
            Reference::Internal(index, text) => self.enter(EntityKey::General(index), text)?,
            Reference::Unparsed => return Err(Fatal("Verweis auf eine nicht analysierte Entität")),
            Reference::External | Reference::Undeclared => {}
        }
        Ok(())
    }

    /// `STag` oder `EmptyElemTag`, ab `<`.
    fn start_tag(&mut self) -> Result<()> {
        self.advance(1);
        let name = self.name()?;
        let mut attributes: Vec<(String, String)> = Vec::new();
        let empty = loop {
            let space = self.skip_space();
            if self.eat(">") {
                break false;
            }
            if self.eat("/>") {
                break true;
            }
            if !space {
                return Err(Fatal("Leerraum vor dem Attribut fehlt"));
            }
            let attribute = self.name()?;
            self.skip_space();
            self.expect("=", "„=“ nach dem Namen des Attributs fehlt")?;
            self.skip_space();
            let value = self.attribute_value(true)?;
            // „Unique Att Spec“
            if attributes.iter().any(|(other, _)| *other == attribute) {
                return Err(Fatal("Attribut doppelt angegeben"));
            }
            attributes.push((attribute, value));
        };
        self.dtd.complete_attributes(&name, &mut attributes);
        let scope = self.scopes.mark();
        let (element, names) = if self.namespaces {
            self.scopes
                .start_element(&name, &attributes)
                .map_err(Fatal)?
        } else {
            let names = attributes
                .iter()
                .map(|(name, _)| Name::plain(name))
                .collect();
            (Name::plain(&name), names)
        };
        let mut dom_attributes = Vec::with_capacity(attributes.len());
        for (attribute, (_, value)) in names.into_iter().zip(attributes) {
            dom_attributes.push(Attribute {
                namespace: attribute
                    .namespace
                    .map(|url| self.document.intern_namespace(&url)),
                prefix: attribute
                    .prefix
                    .map(|prefix| DomString::from(prefix.as_str())),
                local_name: DomString::from(attribute.local.as_str()),
                value: DomString::from(value.as_str()),
            });
        }
        let namespace = element
            .namespace
            .map(|url| self.document.intern_namespace(&url));
        let node = self.document.create_element_ns(
            namespace,
            element
                .prefix
                .map(|prefix| DomString::from(prefix.as_str())),
            DomString::from(element.local.as_str()),
            dom_attributes,
        );
        self.flush_text();
        self.insert(node);
        if empty {
            self.scopes.truncate(scope);
        } else {
            self.open.push(Open { name, node, scope });
        }
        Ok(())
    }

    /// `ETag`, ab `</`.
    fn end_tag(&mut self) -> Result<()> {
        self.advance(2);
        let name = self.name()?;
        self.skip_space();
        self.expect(">", "Endtag nicht abgeschlossen")?;
        let depth = self.frames.last().map_or(0, |frame| frame.depth);
        if self.open.len() <= depth {
            return Err(Fatal("Endtag ohne Starttag in derselben Entität"));
        }
        // „Element Type Match“
        if self.open.last().is_none_or(|open| open.name != name) {
            return Err(Fatal("Endtag passt nicht zum Starttag"));
        }
        self.flush_text();
        if let Some(open) = self.open.pop() {
            self.scopes.truncate(open.scope);
        }
        Ok(())
    }

    /// `AttValue`, normalisiert nach Abschnitt 3.3.3 bis auf das
    /// Zusammenfassen von Leerraum, das vom Typ abhängt. Ohne `resolve`
    /// prüft es Verweise nur auf ihre Form; so liest der Parser
    /// Deklarationen, die er nicht verarbeiten darf.
    fn attribute_value(&mut self, resolve: bool) -> Result<String> {
        let quote = self.quote()?;
        let base = self.frames.len();
        let mut value = String::new();
        loop {
            let Some(c) = self.peek() else {
                if self.frames.len() > base {
                    self.frames.pop();
                    continue;
                }
                return Err(Fatal("Attributwert nicht abgeschlossen"));
            };
            match c {
                _ if c == quote && self.frames.len() == base => {
                    self.advance(1);
                    return Ok(value);
                }
                // „No < in Attribute Values“
                '<' => return Err(Fatal("„<“ in einem Attributwert")),
                '&' if self.peek_at(1) == Some('#') => value.push(self.character_reference()?),
                '&' => {
                    self.advance(1);
                    if !resolve {
                        self.nc_name()?;
                        self.expect(";", "„;“ nach dem Namen der Entität fehlt")?;
                        continue;
                    }
                    match self.entity_reference()? {
                        Reference::Characters(characters) => value.extend(
                            characters
                                .chars()
                                .map(|c| if is_space(c) { ' ' } else { c }),
                        ),
                        Reference::Internal(index, text) => {
                            self.enter(EntityKey::General(index), text)?;
                        }
                        // „No External Entity References“
                        Reference::External | Reference::Unparsed => {
                            return Err(Fatal(
                                "Verweis auf eine externe Entität in einem Attributwert",
                            ));
                        }
                        Reference::Undeclared => {}
                    }
                }
                _ => {
                    value.push(if is_space(c) { ' ' } else { c });
                    self.advance(1);
                }
            }
        }
    }
}

/// Der Rest von Abschnitt 3.3.3 für Attribute, die nicht CDATA sind: kein
/// Leerzeichen am Rand und keines doppelt.
fn collapse_spaces(value: &str) -> String {
    value
        .split(' ')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
