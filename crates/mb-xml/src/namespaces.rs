//! Namensräume nach Namespaces in XML 1.0 (Third Edition): Deklarationen
//! binden, Namen auflösen und die Bedingungen der Abschnitte 3 bis 6
//! prüfen.

use crate::chars::is_qname;

pub(crate) const XML: &str = "http://www.w3.org/XML/1998/namespace";
pub(crate) const XMLNS: &str = "http://www.w3.org/2000/xmlns/";

/// Ein aufgelöster Name. Der Namensraum fehlt, wenn der Name keinen hat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Name {
    pub(crate) namespace: Option<String>,
    pub(crate) prefix: Option<String>,
    pub(crate) local: String,
}

impl Name {
    /// Ein Name ohne Namensraum, so wie er dasteht.
    pub(crate) fn plain(name: &str) -> Name {
        Name {
            namespace: None,
            prefix: None,
            local: name.to_owned(),
        }
    }
}

/// Die Bindungen der offenen Elemente, die innersten zuletzt. Ein Präfix
/// `None` steht für den Standardnamensraum, eine leere URL hebt ihn auf.
#[derive(Default)]
pub(crate) struct Scopes {
    bindings: Vec<(Option<String>, String)>,
}

impl Scopes {
    /// Die Stelle, auf die [`Scopes::truncate`] am Ende des Elements
    /// zurückgeht.
    pub(crate) fn mark(&self) -> usize {
        self.bindings.len()
    }

    pub(crate) fn truncate(&mut self, mark: usize) {
        self.bindings.truncate(mark);
    }

    /// Bindet die Deklarationen unter `attributes` und löst dann den Namen
    /// des Elements und die der Attribute auf.
    pub(crate) fn start_element(
        &mut self,
        element: &str,
        attributes: &[(String, String)],
    ) -> Result<(Name, Vec<Name>), &'static str> {
        if !is_qname(element) {
            return Err("der Name des Elements ist kein QName");
        }
        for (name, value) in attributes {
            if !is_qname(name) {
                return Err("der Name des Attributs ist kein QName");
            }
            if name == "xmlns" {
                self.declare(None, value)?;
            } else if let Some(prefix) = name.strip_prefix("xmlns:") {
                self.declare(Some(prefix), value)?;
            }
        }
        let element = self.resolve(element, true)?;
        let mut names: Vec<Name> = Vec::with_capacity(attributes.len());
        for (name, _) in attributes {
            let resolved = self.resolve_attribute(name)?;
            // „Attributes Unique“: auch die erweiterten Namen sind verschieden.
            if names
                .iter()
                .any(|other| other.namespace == resolved.namespace && other.local == resolved.local)
            {
                return Err("zwei Attribute mit demselben erweiterten Namen");
            }
            names.push(resolved);
        }
        Ok((element, names))
    }

    /// Eine Deklaration mit den Bedingungen aus Abschnitt 3 („Reserved
    /// Prefixes and Namespace Names“, „No Prefix Undeclaring“).
    fn declare(&mut self, prefix: Option<&str>, url: &str) -> Result<(), &'static str> {
        match prefix {
            Some("xmlns") => return Err("das Präfix xmlns darf nicht deklariert werden"),
            Some("xml") if url != XML => {
                return Err("das Präfix xml gehört zu einem festen Namensraum");
            }
            Some("xml") => {}
            _ if url == XML => return Err("der Namensraum von xml gehört nur zum Präfix xml"),
            _ if url == XMLNS => return Err("der Namensraum von xmlns darf nicht gebunden werden"),
            Some(_) if url.is_empty() => {
                return Err("ein Präfix darf nicht an den leeren Namensraum gebunden werden");
            }
            _ => {}
        }
        self.bindings
            .push((prefix.map(str::to_owned), url.to_owned()));
        Ok(())
    }

    /// Die URL zu einem Präfix, beim Standardnamensraum auch leer.
    fn lookup(&self, prefix: Option<&str>) -> Option<&str> {
        let found = self
            .bindings
            .iter()
            .rev()
            .find(|(bound, _)| bound.as_deref() == prefix)
            .map(|(_, url)| url.as_str());
        match (found, prefix) {
            (None, Some("xml")) => Some(XML),
            (found, _) => found,
        }
    }

    fn resolve(&self, name: &str, element: bool) -> Result<Name, &'static str> {
        let Some((prefix, local)) = name.split_once(':') else {
            let namespace = if element {
                self.lookup(None).filter(|url| !url.is_empty())
            } else {
                None
            };
            return Ok(Name {
                namespace: namespace.map(str::to_owned),
                prefix: None,
                local: name.to_owned(),
            });
        };
        if element && prefix == "xmlns" {
            return Err("ein Element darf nicht das Präfix xmlns haben");
        }
        let url = self
            .lookup(Some(prefix))
            .ok_or("das Präfix ist nicht deklariert")?;
        Ok(Name {
            namespace: Some(url.to_owned()),
            prefix: Some(prefix.to_owned()),
            local: local.to_owned(),
        })
    }

    fn resolve_attribute(&self, name: &str) -> Result<Name, &'static str> {
        if name == "xmlns" {
            return Ok(Name {
                namespace: Some(XMLNS.to_owned()),
                prefix: None,
                local: name.to_owned(),
            });
        }
        if let Some(local) = name.strip_prefix("xmlns:") {
            return Ok(Name {
                namespace: Some(XMLNS.to_owned()),
                prefix: Some("xmlns".to_owned()),
                local: local.to_owned(),
            });
        }
        self.resolve(name, false)
    }
}
