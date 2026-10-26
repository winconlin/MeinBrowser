//! Die Dokumenttypdeklaration: der interne Teil der DTD mit Entitäten und
//! Attributlisten (XML 1.0, Abschnitte 2.8, 3.2 bis 3.3 und 4.2 bis 4.7).
//!
//! Externe Entitäten liest der Parser nicht, auch nicht den externen Teil
//! der DTD. Für die öffentlichen Bezeichner, die der HTML Standard
//! nennt, kennt er die Entitäten aus dessen Tabelle so, als stünden sie im
//! externen Teil. Bedingte Abschnitte gibt es nur in externen Entitäten
//! (Abschnitt 3.4), also hier nie.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::rc::Rc;

use mb_base::DomString;
use mb_dom::Doctype;

use super::{EntityKey, Fatal, Parser, Result, collapse_spaces};

/// Die öffentlichen Bezeichner aus „Parsing XML documents“ im HTML
/// Standard, für die die Entitäten des HTML Standard gelten.
const HTML_PUBLIC_IDS: [&str; 11] = [
    "-//W3C//DTD XHTML 1.0 Transitional//EN",
    "-//W3C//DTD XHTML 1.1//EN",
    "-//W3C//DTD XHTML 1.0 Strict//EN",
    "-//W3C//DTD XHTML 1.0 Frameset//EN",
    "-//W3C//DTD XHTML Basic 1.0//EN",
    "-//W3C//DTD XHTML 1.1 plus MathML 2.0//EN",
    "-//W3C//DTD XHTML 1.1 plus MathML 2.0 plus SVG 1.1//EN",
    "-//W3C//DTD MathML 2.0//EN",
    "-//WAPFORUM//DTD XHTML Mobile 1.0//EN",
    "-//WAPFORUM//DTD XHTML Mobile 1.1//EN",
    "-//WAPFORUM//DTD XHTML Mobile 1.2//EN",
];

enum Value {
    /// Der Ersetzungstext: Zeichenverweise sind ersetzt, Verweise auf
    /// allgemeine Entitäten stehen noch da (Abschnitt 4.5).
    Internal(Rc<[char]>),
    External,
    /// Mit `NDATA`.
    Unparsed,
}

struct Entity {
    value: Value,
    /// Deklariert im Ersetzungstext einer Parameter-Entität; zählt dann
    /// nicht für „Entity Declared“.
    in_parameter_entity: bool,
}

/// Eine Entität, wie ein Verweis sie vorfindet.
pub(super) enum Reference {
    /// Vordefiniert (Abschnitt 4.6) oder aus der Tabelle des HTML Standard.
    Characters(&'static str),
    Internal(usize, Rc<[char]>),
    External,
    Unparsed,
    Undeclared,
}

struct AttributeDeclaration {
    name: String,
    /// Nicht CDATA: Leerraum wird zusammengefasst.
    tokenized: bool,
    /// Normalisiert; keiner bei `#REQUIRED` und `#IMPLIED`.
    default: Option<String>,
}

#[derive(Default)]
pub(super) struct Dtd {
    /// Ob es eine Dokumenttypdeklaration gibt.
    present: bool,
    external_subset: bool,
    /// Ob im internen Teil ein Verweis auf eine Parameter-Entität steht.
    parameter_references: bool,
    /// Nach einem Verweis auf eine Parameter-Entität, die der Parser nicht
    /// liest, verarbeitet er keine Deklarationen von Entitäten und
    /// Attributlisten mehr (Abschnitt 5.1).
    skip_declarations: bool,
    html_entities: bool,
    general: Vec<Entity>,
    general_names: BTreeMap<String, usize>,
    parameter: Vec<Entity>,
    parameter_names: BTreeMap<String, usize>,
    attributes: BTreeMap<String, Vec<AttributeDeclaration>>,
}

impl Dtd {
    /// Ob „Entity Declared“ eine Bedingung für Wohlgeformtheit ist: ohne
    /// DTD, mit nur einem internen Teil ohne Verweise auf
    /// Parameter-Entitäten, oder mit `standalone="yes"`.
    pub(super) fn strict(&self, standalone: bool) -> bool {
        !self.present || (!self.external_subset && !self.parameter_references) || standalone
    }

    /// Die allgemeine Entität `name`. Mit `strict` zählen nur Deklarationen
    /// direkt im internen Teil.
    pub(super) fn general_entity(&self, name: &str, strict: bool) -> Reference {
        if let Some(entity) = self
            .general_names
            .get(name)
            .and_then(|&index| Some((index, self.general.get(index)?)))
            .filter(|(_, entity)| !(strict && entity.in_parameter_entity))
        {
            return match &entity.1.value {
                Value::Internal(text) => Reference::Internal(entity.0, text.clone()),
                Value::External => Reference::External,
                Value::Unparsed => Reference::Unparsed,
            };
        }
        let predefined = match name {
            "lt" => Some("<"),
            "gt" => Some(">"),
            "amp" => Some("&"),
            "apos" => Some("'"),
            "quot" => Some("\""),
            _ => None,
        };
        if let Some(characters) = predefined {
            return Reference::Characters(characters);
        }
        // Die Tabelle steht für den externen Teil, zählt also nicht, wenn
        // „Entity Declared“ gilt. Ihre Namen enden alle mit `;`.
        if self.html_entities
            && !strict
            && let Some(characters) = mb_html::named_character_reference(&format!("{name};"))
        {
            return Reference::Characters(characters);
        }
        Reference::Undeclared
    }

    /// Normalisiert die angegebenen Attribute nach ihrem Typ und ergänzt
    /// die mit Vorgabewert (Abschnitte 3.3.2 und 3.3.3). Attribute ohne
    /// Deklaration gelten als CDATA.
    pub(super) fn complete_attributes(
        &self,
        element: &str,
        attributes: &mut Vec<(String, String)>,
    ) {
        let Some(declarations) = self.attributes.get(element) else {
            return;
        };
        for (name, value) in attributes.iter_mut() {
            if declarations
                .iter()
                .any(|declaration| declaration.name == *name && declaration.tokenized)
            {
                *value = collapse_spaces(value);
            }
        }
        for declaration in declarations {
            if let Some(default) = &declaration.default
                && !attributes.iter().any(|(name, _)| *name == declaration.name)
            {
                attributes.push((declaration.name.clone(), default.clone()));
            }
        }
    }

    /// Die erste Deklaration einer Entität gilt (Abschnitt 4.2).
    fn declare_entity(&mut self, parameter: bool, name: String, entity: Entity) {
        let (entities, names) = if parameter {
            (&mut self.parameter, &mut self.parameter_names)
        } else {
            (&mut self.general, &mut self.general_names)
        };
        if let Entry::Vacant(slot) = names.entry(name) {
            slot.insert(entities.len());
            entities.push(entity);
        }
    }
}

impl Parser {
    /// `doctypedecl`, ab `<!DOCTYPE`.
    pub(super) fn doctype(&mut self) -> Result<()> {
        self.advance(9);
        self.require_space()?;
        let name = self.qualified_name()?;
        self.dtd.present = true;
        let mut public_id = String::new();
        let mut system_id = String::new();
        if self.skip_space() && (self.starts_with("SYSTEM") || self.starts_with("PUBLIC")) {
            let (public, system) = self.external_id(false)?;
            public_id = public;
            system_id = system;
            self.dtd.external_subset = true;
            self.skip_space();
        }
        if self.eat("[") {
            self.internal_subset()?;
            self.skip_space();
        }
        self.expect(">", "Dokumenttypdeklaration nicht abgeschlossen")?;
        self.dtd.html_entities = HTML_PUBLIC_IDS.contains(&public_id.as_str());
        let node = self.document.create_doctype(Doctype {
            name: DomString::from(name.as_str()),
            public_id: DomString::from(public_id.as_str()),
            system_id: DomString::from(system_id.as_str()),
        });
        self.insert(node);
        Ok(())
    }

    /// `ExternalID`, mit `public_only` auch `PublicID` (bei Notationen).
    /// Liefert die Literale, ein fehlendes leer.
    fn external_id(&mut self, public_only: bool) -> Result<(String, String)> {
        if self.eat("SYSTEM") {
            self.require_space()?;
            return Ok((String::new(), self.system_literal()?));
        }
        self.expect("PUBLIC", "SYSTEM oder PUBLIC erwartet")?;
        self.require_space()?;
        let public = self.pubid_literal()?;
        if public_only {
            if self.skip_space() && matches!(self.peek(), Some('"' | '\'')) {
                return Ok((public, self.system_literal()?));
            }
            return Ok((public, String::new()));
        }
        self.require_space()?;
        Ok((public, self.system_literal()?))
    }

    /// `SystemLiteral`
    fn system_literal(&mut self) -> Result<String> {
        let quote = self.quote()?;
        let mut literal = String::new();
        loop {
            match self.next() {
                Some(c) if c == quote => return Ok(literal),
                Some(c) => literal.push(c),
                None => return Err(Fatal("Literal nicht abgeschlossen")),
            }
        }
    }

    /// `PubidLiteral`
    fn pubid_literal(&mut self) -> Result<String> {
        let quote = self.quote()?;
        let mut literal = String::new();
        loop {
            match self.next() {
                Some(c) if c == quote => return Ok(literal),
                Some(c) if crate::chars::is_pubid_char(c) => literal.push(c),
                Some(_) => return Err(Fatal("unzulässiges Zeichen im öffentlichen Bezeichner")),
                None => return Err(Fatal("Literal nicht abgeschlossen")),
            }
        }
    }

    /// `intSubset` bis zum `]`. Der Ersetzungstext einer Parameter-Entität
    /// zwischen den Deklarationen muss ganze Deklarationen enthalten („PE
    /// Between Declarations“).
    fn internal_subset(&mut self) -> Result<()> {
        loop {
            if self.peek().is_none() {
                if self.frames.len() > 1 {
                    self.frames.pop();
                    continue;
                }
                return Err(Fatal("DTD nicht abgeschlossen"));
            }
            if self.skip_space() {
                continue;
            }
            if self.frames.len() == 1 && self.eat("]") {
                return Ok(());
            }
            if self.starts_with("<!ELEMENT") {
                self.element_declaration()?;
            } else if self.starts_with("<!ATTLIST") {
                self.attribute_list_declaration()?;
            } else if self.starts_with("<!ENTITY") {
                self.entity_declaration()?;
            } else if self.starts_with("<!NOTATION") {
                self.notation_declaration()?;
            } else if self.starts_with("<!--") {
                self.comment()?;
            } else if self.starts_with("<?") {
                self.processing_instruction()?;
            } else if self.peek() == Some('%') {
                self.parameter_entity_reference()?;
            } else {
                return Err(Fatal("unerwartetes Zeichen in der DTD"));
            }
        }
    }

    /// `PEReference` zwischen Deklarationen, ab `%`.
    fn parameter_entity_reference(&mut self) -> Result<()> {
        self.advance(1);
        let name = self.nc_name()?;
        self.expect(";", "„;“ nach dem Namen der Entität fehlt")?;
        self.dtd.parameter_references = true;
        let strict = self.standalone && !self.in_parameter_entity();
        let found = self
            .dtd
            .parameter_names
            .get(&name)
            .and_then(|&index| Some((index, self.dtd.parameter.get(index)?)))
            .filter(|(_, entity)| !(strict && entity.in_parameter_entity));
        match found {
            Some((
                index,
                Entity {
                    value: Value::Internal(text),
                    ..
                },
            )) => {
                let text = text.clone();
                self.enter(EntityKey::Parameter(index), text)
            }
            None if strict => Err(Fatal("Parameter-Entität nicht deklariert")),
            _ => {
                if !self.standalone {
                    self.dtd.skip_declarations = true;
                }
                Ok(())
            }
        }
    }

    /// `elementdecl`, ab `<!ELEMENT`. Der Parser prüft nur die Form.
    fn element_declaration(&mut self) -> Result<()> {
        self.advance(9);
        self.require_space()?;
        self.qualified_name()?;
        self.require_space()?;
        if !self.eat("EMPTY") && !self.eat("ANY") {
            self.expect("(", "Inhaltsmodell erwartet")?;
            self.content_model()?;
        }
        self.skip_space();
        self.expect(">", "Elementdeklaration nicht abgeschlossen")
    }

    /// `Mixed` oder `children` nach der ersten Klammer, mit einem Stapel
    /// statt Rekursion. Je offene Gruppe merkt er sich das Trennzeichen;
    /// `,` und `|` dürfen sich in einer Gruppe nicht mischen.
    fn content_model(&mut self) -> Result<()> {
        self.skip_space();
        if self.eat("#PCDATA") {
            self.skip_space();
            if self.eat(")") {
                self.eat("*");
                return Ok(());
            }
            loop {
                self.skip_space();
                if self.eat(")*") {
                    return Ok(());
                }
                self.expect("|", "„|“ im gemischten Inhalt erwartet")?;
                self.skip_space();
                self.qualified_name()?;
            }
        }
        let mut groups: Vec<Option<char>> = vec![None];
        loop {
            self.skip_space();
            if self.eat("(") {
                groups.push(None);
                continue;
            }
            self.qualified_name()?;
            self.occurrence();
            loop {
                self.skip_space();
                if self.eat(")") {
                    groups.pop();
                    self.occurrence();
                    if groups.is_empty() {
                        return Ok(());
                    }
                    continue;
                }
                let separator = self
                    .peek()
                    .filter(|&c| c == ',' || c == '|')
                    .ok_or(Fatal("„,“, „|“ oder „)“ erwartet"))?;
                let group = groups
                    .last_mut()
                    .ok_or(Fatal("„,“, „|“ oder „)“ erwartet"))?;
                if group.is_some_and(|other| other != separator) {
                    return Err(Fatal("„,“ und „|“ in derselben Gruppe"));
                }
                *group = Some(separator);
                self.advance(1);
                break;
            }
        }
    }

    fn occurrence(&mut self) {
        if matches!(self.peek(), Some('?' | '*' | '+')) {
            self.advance(1);
        }
    }

    /// `AttlistDecl`, ab `<!ATTLIST`.
    fn attribute_list_declaration(&mut self) -> Result<()> {
        self.advance(9);
        self.require_space()?;
        let element = self.qualified_name()?;
        let record = !self.dtd.skip_declarations;
        loop {
            let space = self.skip_space();
            if self.eat(">") {
                return Ok(());
            }
            if !space {
                return Err(Fatal("Leerraum fehlt"));
            }
            let name = self.qualified_name()?;
            self.require_space()?;
            let tokenized = self.attribute_type()?;
            self.require_space()?;
            let default = if self.eat("#REQUIRED") || self.eat("#IMPLIED") {
                None
            } else {
                if self.eat("#FIXED") {
                    self.require_space()?;
                }
                // Die Entitäten im Vorgabewert müssen vorher deklariert
                // sein („Entity Declared“), also gilt der Wert von jetzt.
                let value = self.attribute_value(record)?;
                Some(if tokenized {
                    collapse_spaces(&value)
                } else {
                    value
                })
            };
            if record {
                let declarations = self.dtd.attributes.entry(element.clone()).or_default();
                // Die erste Deklaration eines Attributs gilt (Abschnitt 3.3).
                if !declarations
                    .iter()
                    .any(|declaration| declaration.name == name)
                {
                    declarations.push(AttributeDeclaration {
                        name,
                        tokenized,
                        default,
                    });
                }
            }
        }
    }

    /// `AttType`; sagt, ob der Typ nicht CDATA ist.
    fn attribute_type(&mut self) -> Result<bool> {
        // Die längeren Schlüsselwörter zuerst, sonst passte `ID` auf `IDREF`.
        for keyword in [
            "CDATA", "IDREFS", "IDREF", "ID", "ENTITIES", "ENTITY", "NMTOKENS", "NMTOKEN",
        ] {
            if self.eat(keyword) {
                return Ok(keyword != "CDATA");
            }
        }
        let notation = self.eat("NOTATION");
        if notation {
            self.require_space()?;
        }
        self.expect("(", "Typ des Attributs erwartet")?;
        loop {
            self.skip_space();
            if notation {
                self.nc_name()?;
            } else if !self.nmtoken() {
                return Err(Fatal("Nmtoken erwartet"));
            }
            self.skip_space();
            if self.eat(")") {
                return Ok(true);
            }
            self.expect("|", "„|“ oder „)“ erwartet")?;
        }
    }

    /// `Nmtoken`; sagt, ob eines da war.
    fn nmtoken(&mut self) -> bool {
        let mut found = false;
        while self.peek().is_some_and(crate::chars::is_name_char) {
            self.advance(1);
            found = true;
        }
        found
    }

    /// `EntityDecl`, ab `<!ENTITY`.
    fn entity_declaration(&mut self) -> Result<()> {
        self.advance(8);
        self.require_space()?;
        let parameter = self.eat("%");
        if parameter {
            self.require_space()?;
        }
        let name = self.nc_name()?;
        self.require_space()?;
        let value = if matches!(self.peek(), Some('"' | '\'')) {
            Value::Internal(self.entity_value()?)
        } else {
            self.external_id(false)?;
            if self.skip_space() && self.eat("NDATA") {
                if parameter {
                    return Err(Fatal("NDATA bei einer Parameter-Entität"));
                }
                self.require_space()?;
                self.nc_name()?;
                Value::Unparsed
            } else {
                Value::External
            }
        };
        self.skip_space();
        self.expect(">", "Entitätsdeklaration nicht abgeschlossen")?;
        if !self.dtd.skip_declarations {
            let entity = Entity {
                value,
                in_parameter_entity: self.in_parameter_entity(),
            };
            self.dtd.declare_entity(parameter, name, entity);
        }
        Ok(())
    }

    /// `EntityValue` als Ersetzungstext. Verweise auf Parameter-Entitäten
    /// dürfen im internen Teil nicht in Deklarationen stehen („PEs in
    /// Internal Subset“); Verweise auf allgemeine Entitäten bleiben stehen.
    fn entity_value(&mut self) -> Result<Rc<[char]>> {
        let quote = self.quote()?;
        let mut text = Vec::new();
        loop {
            match self.peek() {
                Some(c) if c == quote => {
                    self.advance(1);
                    return Ok(text.into());
                }
                Some('%') => {
                    return Err(Fatal(
                        "Parameter-Entität in einer Deklaration im internen Teil der DTD",
                    ));
                }
                Some('&') if self.peek_at(1) == Some('#') => text.push(self.character_reference()?),
                Some('&') => {
                    self.advance(1);
                    let name = self.nc_name()?;
                    self.expect(";", "„;“ nach dem Namen der Entität fehlt")?;
                    text.push('&');
                    text.extend(name.chars());
                    text.push(';');
                }
                Some(c) => {
                    text.push(c);
                    self.advance(1);
                }
                None => return Err(Fatal("Wert der Entität nicht abgeschlossen")),
            }
        }
    }

    /// `NotationDecl`, ab `<!NOTATION`.
    fn notation_declaration(&mut self) -> Result<()> {
        self.advance(10);
        self.require_space()?;
        self.nc_name()?;
        self.require_space()?;
        self.external_id(true)?;
        self.skip_space();
        self.expect(">", "Notationsdeklaration nicht abgeschlossen")
    }
}
