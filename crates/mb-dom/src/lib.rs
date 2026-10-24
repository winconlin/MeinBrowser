//! Dokumentspeicher (Spaltenspeicher) und Änderungsjournal.
//!
//! Siehe `docs/architektur.md`, Abschnitte 2.1, 2.2 und 4.5. Ein Knoten ist
//! eine Nummer mit Generation ([`NodeId`]); die Baumverweise liegen in
//! eigenen Spalten, die übrigen Eigenschaften je Knotenart in einer
//! weiteren. Jede Änderung am Baum landet im Journal ([`Change`]).
//!
//! Bisher kann der Speicher, was der HTML-Parser braucht. Knoten werden noch
//! nicht freigegeben (also bleibt die Generation 0), und der Inhalt eines
//! `template` liegt im selben Speicher statt in einem eigenen Dokument (*the
//! template contents owner document*).

#![forbid(unsafe_code)]

use mb_base::DomString;

/// Ein Knoten: Nummer und Generation. Eine veraltete Nummer (falsche
/// Generation) gilt wie eine unbekannte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId {
    index: u32,
    generation: u32,
}

/// Steht in den Verweisspalten für „kein Knoten“.
const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    DocumentType,
    DocumentFragment,
    Element,
    Text,
    Comment,
    ProcessingInstruction,
}

/// Die Namensräume, die der HTML-Parser kennt (Infra, „Namespaces“).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Namespace {
    Html,
    MathMl,
    Svg,
    XLink,
    Xml,
    Xmlns,
}

impl Namespace {
    pub const fn url(self) -> &'static str {
        match self {
            Namespace::Html => "http://www.w3.org/1999/xhtml",
            Namespace::MathMl => "http://www.w3.org/1998/Math/MathML",
            Namespace::Svg => "http://www.w3.org/2000/svg",
            Namespace::XLink => "http://www.w3.org/1999/xlink",
            Namespace::Xml => "http://www.w3.org/XML/1998/namespace",
            Namespace::Xmlns => "http://www.w3.org/2000/xmlns/",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub namespace: Option<Namespace>,
    pub prefix: Option<DomString>,
    pub local_name: DomString,
    pub value: DomString,
}

impl Attribute {
    /// Ein Attribut ohne Namensraum.
    pub fn new(local_name: DomString, value: DomString) -> Attribute {
        Attribute {
            namespace: None,
            prefix: None,
            local_name,
            value,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    pub namespace: Option<Namespace>,
    pub prefix: Option<DomString>,
    pub local_name: DomString,
    pub attributes: Vec<Attribute>,
    /// Der Inhalt eines HTML-`template` (*template contents*).
    template_contents: Option<NodeId>,
}

impl Element {
    /// Ob das Element im HTML-Namensraum liegt und so heißt.
    pub fn is_html(&self, local_name: &str) -> bool {
        self.namespace == Some(Namespace::Html) && self.local_name == *local_name
    }

    pub fn is(&self, namespace: Namespace, local_name: &str) -> bool {
        self.namespace == Some(namespace) && self.local_name == *local_name
    }

    /// Ein Attribut ohne Namensraum nach seinem Namen.
    pub fn attribute(&self, local_name: &str) -> Option<&DomString> {
        self.attributes
            .iter()
            .find(|attribute| attribute.namespace.is_none() && attribute.local_name == *local_name)
            .map(|attribute| &attribute.value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doctype {
    pub name: DomString,
    pub public_id: DomString,
    pub system_id: DomString,
}

#[derive(Clone, Debug)]
enum Payload {
    Document,
    Doctype(Doctype),
    /// `host` ist bei einem Template-Inhalt das `template` (DOM: *host*).
    Fragment {
        host: Option<NodeId>,
    },
    Element(Element),
    Text(DomString),
    Comment(DomString),
    ProcessingInstruction {
        target: DomString,
        data: DomString,
    },
}

/// Der Modus eines Dokuments (DOM: *mode*).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QuirksMode {
    #[default]
    NoQuirks,
    Quirks,
    LimitedQuirks,
}

/// Ein Eintrag im Änderungsjournal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Der Knoten wurde in einen Elternknoten eingefügt.
    Inserted(NodeId),
    /// Der Knoten wurde aus `parent` entfernt.
    Removed { node: NodeId, parent: NodeId },
    /// Die Attribute des Elements haben sich geändert.
    Attributes(NodeId),
    /// Die Daten eines Text-, Kommentar- oder Verarbeitungsknotens haben
    /// sich geändert. Aufeinanderfolgende Änderungen desselben Knotens
    /// ergeben einen Eintrag.
    Data(NodeId),
}

/// Der Baum passt nicht: ein Knoten würde sein eigener Vorfahr, der
/// Bezugsknoten ist kein Kind des Elternknotens, oder ein Verweis ist
/// unbekannt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HierarchyError;

impl std::fmt::Display for HierarchyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ungültige Einfügung in den Dokumentbaum")
    }
}

impl std::error::Error for HierarchyError {}

/// Ein Dokument mit allen seinen Knoten.
pub struct Document {
    generation: Vec<u32>,
    parent: Vec<u32>,
    first_child: Vec<u32>,
    last_child: Vec<u32>,
    previous_sibling: Vec<u32>,
    next_sibling: Vec<u32>,
    payload: Vec<Payload>,
    mode: QuirksMode,
    journal: Vec<Change>,
}

impl Default for Document {
    fn default() -> Document {
        Document::new()
    }
}

impl Document {
    /// Ein leeres Dokument; sein Dokumentknoten ist [`Document::root`].
    pub fn new() -> Document {
        let mut document = Document {
            generation: Vec::new(),
            parent: Vec::new(),
            first_child: Vec::new(),
            last_child: Vec::new(),
            previous_sibling: Vec::new(),
            next_sibling: Vec::new(),
            payload: Vec::new(),
            mode: QuirksMode::NoQuirks,
            journal: Vec::new(),
        };
        document.push(Payload::Document);
        document
    }

    /// Der Dokumentknoten.
    pub fn root(&self) -> NodeId {
        NodeId {
            index: 0,
            generation: self.generation.first().copied().unwrap_or(0),
        }
    }

    pub fn quirks_mode(&self) -> QuirksMode {
        self.mode
    }

    pub fn set_quirks_mode(&mut self, mode: QuirksMode) {
        self.mode = mode;
    }

    /// Die Zahl der angelegten Knoten.
    pub fn len(&self) -> usize {
        self.payload.len()
    }

    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }

    fn push(&mut self, payload: Payload) -> NodeId {
        // Vier Milliarden Knoten brauchen weit mehr Speicher, als ein Rechner
        // hat; vorher endet die Speicheranforderung.
        let index = u32::try_from(self.payload.len())
            .ok()
            .filter(|&index| index != NONE)
            .expect("Dokumentspeicher voll");
        self.generation.push(0);
        self.parent.push(NONE);
        self.first_child.push(NONE);
        self.last_child.push(NONE);
        self.previous_sibling.push(NONE);
        self.next_sibling.push(NONE);
        self.payload.push(payload);
        NodeId {
            index,
            generation: 0,
        }
    }

    /// Die Zeile eines gültigen Verweises.
    fn slot(&self, node: NodeId) -> Option<usize> {
        let slot = usize::try_from(node.index).ok()?;
        (self.generation.get(slot) == Some(&node.generation)).then_some(slot)
    }

    fn id(&self, index: u32) -> Option<NodeId> {
        if index == NONE {
            return None;
        }
        let generation = *self.generation.get(usize::try_from(index).ok()?)?;
        Some(NodeId { index, generation })
    }

    fn link(column: &[u32], slot: usize) -> u32 {
        column.get(slot).copied().unwrap_or(NONE)
    }

    fn set_link(column: &mut [u32], node: NodeId, value: Option<NodeId>) {
        if let Some(cell) = usize::try_from(node.index)
            .ok()
            .and_then(|slot| column.get_mut(slot))
        {
            *cell = value.map_or(NONE, |value| value.index);
        }
    }

    // Knoten anlegen

    /// Ein Element. Ein HTML-`template` bekommt gleich seinen Inhalt.
    pub fn create_element(
        &mut self,
        namespace: Option<Namespace>,
        local_name: DomString,
        attributes: Vec<Attribute>,
    ) -> NodeId {
        let is_template = namespace == Some(Namespace::Html) && local_name == "template";
        let element = self.push(Payload::Element(Element {
            namespace,
            prefix: None,
            local_name,
            attributes,
            template_contents: None,
        }));
        if is_template {
            let contents = self.push(Payload::Fragment {
                host: Some(element),
            });
            if let Some(Payload::Element(data)) = self
                .slot(element)
                .and_then(|slot| self.payload.get_mut(slot))
            {
                data.template_contents = Some(contents);
            }
        }
        element
    }

    pub fn create_text(&mut self, data: DomString) -> NodeId {
        self.push(Payload::Text(data))
    }

    pub fn create_comment(&mut self, data: DomString) -> NodeId {
        self.push(Payload::Comment(data))
    }

    pub fn create_processing_instruction(&mut self, target: DomString, data: DomString) -> NodeId {
        self.push(Payload::ProcessingInstruction { target, data })
    }

    pub fn create_doctype(&mut self, doctype: Doctype) -> NodeId {
        self.push(Payload::Doctype(doctype))
    }

    pub fn create_fragment(&mut self) -> NodeId {
        self.push(Payload::Fragment { host: None })
    }

    // Lesen

    fn payload(&self, node: NodeId) -> Option<&Payload> {
        self.payload.get(self.slot(node)?)
    }

    pub fn kind(&self, node: NodeId) -> Option<NodeKind> {
        Some(match self.payload(node)? {
            Payload::Document => NodeKind::Document,
            Payload::Doctype(_) => NodeKind::DocumentType,
            Payload::Fragment { .. } => NodeKind::DocumentFragment,
            Payload::Element(_) => NodeKind::Element,
            Payload::Text(_) => NodeKind::Text,
            Payload::Comment(_) => NodeKind::Comment,
            Payload::ProcessingInstruction { .. } => NodeKind::ProcessingInstruction,
        })
    }

    pub fn element(&self, node: NodeId) -> Option<&Element> {
        match self.payload(node)? {
            Payload::Element(element) => Some(element),
            _ => None,
        }
    }

    pub fn doctype(&self, node: NodeId) -> Option<&Doctype> {
        match self.payload(node)? {
            Payload::Doctype(doctype) => Some(doctype),
            _ => None,
        }
    }

    /// Die Daten eines Text- oder Kommentarknotens oder einer
    /// Verarbeitungsanweisung.
    pub fn data(&self, node: NodeId) -> Option<&DomString> {
        match self.payload(node)? {
            Payload::Text(data) | Payload::Comment(data) => Some(data),
            Payload::ProcessingInstruction { data, .. } => Some(data),
            _ => None,
        }
    }

    /// Das Ziel einer Verarbeitungsanweisung.
    pub fn target(&self, node: NodeId) -> Option<&DomString> {
        match self.payload(node)? {
            Payload::ProcessingInstruction { target, .. } => Some(target),
            _ => None,
        }
    }

    pub fn template_contents(&self, node: NodeId) -> Option<NodeId> {
        self.element(node)?.template_contents
    }

    /// Der *host* eines Fragments, beim Template-Inhalt das `template`.
    pub fn host(&self, node: NodeId) -> Option<NodeId> {
        match self.payload(node)? {
            Payload::Fragment { host } => *host,
            _ => None,
        }
    }

    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.id(Self::link(&self.parent, self.slot(node)?))
    }

    pub fn first_child(&self, node: NodeId) -> Option<NodeId> {
        self.id(Self::link(&self.first_child, self.slot(node)?))
    }

    pub fn last_child(&self, node: NodeId) -> Option<NodeId> {
        self.id(Self::link(&self.last_child, self.slot(node)?))
    }

    pub fn previous_sibling(&self, node: NodeId) -> Option<NodeId> {
        self.id(Self::link(&self.previous_sibling, self.slot(node)?))
    }

    pub fn next_sibling(&self, node: NodeId) -> Option<NodeId> {
        self.id(Self::link(&self.next_sibling, self.slot(node)?))
    }

    pub fn children(&self, node: NodeId) -> Children<'_> {
        Children {
            document: self,
            next: self.first_child(node),
        }
    }

    /// Ob `ancestor` ein inklusiver Vorfahr von `node` ist (*inclusive
    /// ancestor*).
    pub fn is_inclusive_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(node) = current {
            if node == ancestor {
                return true;
            }
            current = self.parent(node);
        }
        false
    }

    /// *Host-including inclusive ancestor*: wie [`Document::is_inclusive_ancestor`],
    /// geht aber von einem Fragment mit *host* zu diesem weiter.
    pub fn is_host_including_inclusive_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(node) = current {
            if node == ancestor {
                return true;
            }
            current = self.parent(node).or_else(|| self.host(node));
        }
        false
    }

    // Ändern

    /// Hängt den Codepunkt an die Daten eines Textknotens.
    pub fn append_data(&mut self, node: NodeId, code_point: mb_base::CodePoint) {
        let Some(slot) = self.slot(node) else {
            return;
        };
        if let Some(Payload::Text(data)) = self.payload.get_mut(slot) {
            data.push_code_point(code_point);
            if self.journal.last() != Some(&Change::Data(node)) {
                self.journal.push(Change::Data(node));
            }
        }
    }

    /// Hängt ein Attribut an ein Element an.
    pub fn append_attribute(&mut self, node: NodeId, attribute: Attribute) {
        let Some(slot) = self.slot(node) else {
            return;
        };
        if let Some(Payload::Element(element)) = self.payload.get_mut(slot) {
            element.attributes.push(attribute);
            self.journal.push(Change::Attributes(node));
        }
    }

    /// Fügt `node` vor `reference` in `parent` ein, ohne `reference` am
    /// Ende. Hat `node` schon einen Elternknoten, wird es dort zuerst
    /// entfernt.
    pub fn insert_before(
        &mut self,
        parent: NodeId,
        node: NodeId,
        reference: Option<NodeId>,
    ) -> Result<(), HierarchyError> {
        if self.slot(parent).is_none()
            || self.slot(node).is_none()
            || self.is_host_including_inclusive_ancestor(node, parent)
            || reference.is_some_and(|reference| self.parent(reference) != Some(parent))
        {
            return Err(HierarchyError);
        }
        // Wird `node` vor sich selbst eingefügt, gilt sein Nachfolger als
        // Bezug (DOM: *insert*, Schritt „If child is node“).
        let reference = if reference == Some(node) {
            self.next_sibling(node)
        } else {
            reference
        };
        self.remove(node);
        let previous = match reference {
            Some(reference) => self.previous_sibling(reference),
            None => self.last_child(parent),
        };
        Self::set_link(&mut self.parent, node, Some(parent));
        Self::set_link(&mut self.previous_sibling, node, previous);
        Self::set_link(&mut self.next_sibling, node, reference);
        match previous {
            Some(previous) => Self::set_link(&mut self.next_sibling, previous, Some(node)),
            None => Self::set_link(&mut self.first_child, parent, Some(node)),
        }
        match reference {
            Some(reference) => Self::set_link(&mut self.previous_sibling, reference, Some(node)),
            None => Self::set_link(&mut self.last_child, parent, Some(node)),
        }
        self.journal.push(Change::Inserted(node));
        Ok(())
    }

    pub fn append(&mut self, parent: NodeId, node: NodeId) -> Result<(), HierarchyError> {
        self.insert_before(parent, node, None)
    }

    /// Löst den Knoten aus seinem Elternknoten, falls er einen hat.
    pub fn remove(&mut self, node: NodeId) {
        let Some(parent) = self.parent(node) else {
            return;
        };
        let previous = self.previous_sibling(node);
        let next = self.next_sibling(node);
        match previous {
            Some(previous) => Self::set_link(&mut self.next_sibling, previous, next),
            None => Self::set_link(&mut self.first_child, parent, next),
        }
        match next {
            Some(next) => Self::set_link(&mut self.previous_sibling, next, previous),
            None => Self::set_link(&mut self.last_child, parent, previous),
        }
        Self::set_link(&mut self.parent, node, None);
        Self::set_link(&mut self.previous_sibling, node, None);
        Self::set_link(&mut self.next_sibling, node, None);
        self.journal.push(Change::Removed { node, parent });
    }

    /// Nimmt die Einträge des Journals heraus.
    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.journal)
    }
}

/// Die Kinder eines Knotens in Baumreihenfolge.
pub struct Children<'a> {
    document: &'a Document,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let node = self.next?;
        self.next = self.document.next_sibling(node);
        Some(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(document: &mut Document, name: &str) -> NodeId {
        document.create_element(Some(Namespace::Html), DomString::from(name), Vec::new())
    }

    #[test]
    fn insert_and_remove() {
        let mut document = Document::new();
        let root = document.root();
        let html = element(&mut document, "html");
        let head = element(&mut document, "head");
        let body = element(&mut document, "body");
        document.append(root, html).expect("einfügbar");
        document.append(html, body).expect("einfügbar");
        document
            .insert_before(html, head, Some(body))
            .expect("einfügbar");
        let children: Vec<NodeId> = document.children(html).collect();
        assert_eq!(children, [head, body]);
        assert_eq!(document.parent(head), Some(html));
        assert_eq!(document.previous_sibling(body), Some(head));
        assert_eq!(document.first_child(root), Some(html));
        // Umhängen entfernt zuerst.
        document.append(head, body).expect("einfügbar");
        assert_eq!(document.children(html).collect::<Vec<_>>(), [head]);
        assert_eq!(document.last_child(html), Some(head));
        assert_eq!(document.parent(body), Some(head));
        document.remove(head);
        assert_eq!(document.first_child(html), None);
        assert_eq!(document.last_child(html), None);
        assert_eq!(document.parent(head), None);
        let changes = document.take_changes();
        assert_eq!(changes.len(), 6);
        assert_eq!(
            changes.last(),
            Some(&Change::Removed {
                node: head,
                parent: html
            })
        );
        assert!(document.take_changes().is_empty());
    }

    #[test]
    fn hierarchy_errors() {
        let mut document = Document::new();
        let a = element(&mut document, "a");
        let b = element(&mut document, "b");
        let c = element(&mut document, "c");
        document.append(a, b).expect("einfügbar");
        assert_eq!(document.append(b, a), Err(HierarchyError));
        assert_eq!(document.append(a, a), Err(HierarchyError));
        assert_eq!(document.insert_before(c, a, Some(b)), Err(HierarchyError));
        let stale = NodeId {
            index: 99,
            generation: 0,
        };
        assert_eq!(document.append(a, stale), Err(HierarchyError));
        assert_eq!(document.parent(stale), None);
    }

    #[test]
    fn template_contents_and_text() {
        let mut document = Document::new();
        let template = element(&mut document, "template");
        let contents = document.template_contents(template).expect("Inhalt");
        assert_eq!(document.kind(contents), Some(NodeKind::DocumentFragment));
        assert_eq!(document.host(contents), Some(template));
        let inner = element(&mut document, "p");
        document.append(contents, inner).expect("einfügbar");
        // Das Template liegt über seinem Inhalt.
        assert!(document.is_host_including_inclusive_ancestor(template, inner));
        assert!(!document.is_inclusive_ancestor(template, inner));
        assert_eq!(document.append(inner, template), Err(HierarchyError));
        let text = document.create_text(DomString::from("a"));
        document.append_data(text, mb_base::CodePoint::from('b'));
        document.append_data(text, mb_base::CodePoint::from('c'));
        assert_eq!(
            document
                .data(text)
                .map(DomString::to_string_lossy)
                .as_deref(),
            Some("abc")
        );
        assert_eq!(
            document
                .take_changes()
                .iter()
                .filter(|c| matches!(c, Change::Data(_)))
                .count(),
            1
        );
    }
}
