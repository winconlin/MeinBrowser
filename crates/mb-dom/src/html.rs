//! Was der HTML Standard an die Algorithmen des DOM hängt: die Schritte beim
//! Einfügen (*insertion steps*, *post-connection steps*), beim Entfernen
//! (*removing steps*), bei Attributänderungen und wenn der Parser ein Element
//! vom Stapel offener Elemente nimmt, dazu der Zustand der Elemente dafür.
//!
//! Bisher nur für `select`, `option` und `selectedcontent` (Abschnitt
//! „Forms“): die Auswahl (*selectedness*) und das Spiegeln der gewählten
//! `option` in `selectedcontent`. Noch nicht umgesetzt: die Formularzuordnung
//! (*reset the form owner*), *process internal resource links*, die Schritte
//! beim Verschieben (*moving steps*, es gibt noch kein `moveBefore()`), das
//! Zurücksetzen eines Formulars, Ereignisse und die Schritte der übrigen
//! Elemente, die der Parser beim Herausnehmen auslöst (`style`, `object`,
//! Medienelemente, `textarea`).

use mb_base::DomString;

use crate::{Attribute, Document, Namespace, NodeId, Payload};

/// Der Zustand eines HTML-Elements über seine Attribute hinaus.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum State {
    #[default]
    None,
    /// Eine `option`: *selectedness*, *dirtiness* und *cached nearest
    /// ancestor select element*.
    Option {
        selected: bool,
        dirty: bool,
        select: Option<NodeId>,
    },
    /// Ein `selectedcontent` mit seinem Merkmal *disabled*.
    SelectedContent { disabled: bool },
}

impl State {
    /// Der Zustand eines neuen Elements. Eine `option` mit dem Attribut
    /// `selected` ist gewählt („when the element is created, its selectedness
    /// must be set to true if the element has a selected attribute“).
    pub(crate) fn new(
        namespace: Option<Namespace>,
        local_name: &DomString,
        attributes: &[Attribute],
    ) -> State {
        if namespace != Some(Namespace::Html) {
            return State::None;
        }
        match local_name.as_ascii() {
            Some("option") => State::Option {
                selected: attributes.iter().any(|attribute| {
                    attribute.namespace.is_none() && attribute.local_name == "selected"
                }),
                dirty: false,
                select: None,
            },
            Some("selectedcontent") => State::SelectedContent { disabled: false },
            _ => State::None,
        }
    }
}

/// Eine Mikroaufgabe aus den Schritten hier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Microtask {
    /// „Update a select's descendant selectedcontent elements“
    UpdateSelectedcontents(NodeId),
}

/// ASCII-Leerraum (Infra, *ASCII whitespace*) als UTF-16-Code-Unit.
fn is_ascii_whitespace(unit: u16) -> bool {
    matches!(unit, 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

/// „Rules for parsing non-negative integers“ über die „rules for parsing
/// integers“. Sehr große Werte bleiben beim größten `u64` stehen; verglichen
/// wird nur mit kleinen Zahlen.
fn parse_non_negative_integer(input: &DomString) -> Option<u64> {
    let mut units = input.code_units().peekable();
    while units.next_if(|&unit| is_ascii_whitespace(unit)).is_some() {}
    let negative = match units.peek() {
        None => return None,
        Some(0x2D) => {
            units.next();
            true
        }
        Some(0x2B) => {
            units.next();
            false
        }
        Some(_) => false,
    };
    let mut value: u64 = 0;
    let mut any_digit = false;
    while let Some(unit) = units.next_if(|unit| (0x30..=0x39).contains(unit)) {
        value = value
            .saturating_mul(10)
            .saturating_add(u64::from(unit.saturating_sub(0x30)));
        any_digit = true;
    }
    // Kein Zeichen oder keine Ziffer nach dem Vorzeichen ist ein Fehler. Von
    // den negativen Werten ist nur null nicht negativ („-0“).
    if !any_digit || (negative && value != 0) {
        return None;
    }
    Some(value)
}

impl Document {
    /// Der lokale Name eines HTML-Elements, wenn er ASCII ist.
    fn html_name(&self, node: NodeId) -> Option<&str> {
        let element = self.element(node)?;
        if element.namespace != Some(Namespace::Html) {
            return None;
        }
        element.local_name.as_ascii()
    }

    fn has_attribute(&self, element: NodeId, name: &str) -> bool {
        self.element(element)
            .is_some_and(|element| element.attribute(name).is_some())
    }

    fn state(&self, node: NodeId) -> Option<&State> {
        Some(&self.element(node)?.state)
    }

    fn state_mut(&mut self, node: NodeId) -> Option<&mut State> {
        let slot = self.slot(node)?;
        match self.payload.get_mut(slot)? {
            Payload::Element(element) => Some(&mut element.state),
            _ => None,
        }
    }

    // Der Zustand von `option` und `selectedcontent`

    /// *selectedness* einer `option`; `false` für alles andere.
    fn selectedness(&self, option: NodeId) -> bool {
        matches!(
            self.state(option),
            Some(State::Option { selected: true, .. })
        )
    }

    fn set_selectedness(&mut self, option: NodeId, value: bool) {
        if let Some(State::Option { selected, .. }) = self.state_mut(option) {
            *selected = value;
        }
    }

    fn dirtiness(&self, option: NodeId) -> bool {
        matches!(self.state(option), Some(State::Option { dirty: true, .. }))
    }

    /// *cached nearest ancestor select element*
    fn cached_select(&self, option: NodeId) -> Option<NodeId> {
        match self.state(option) {
            Some(State::Option { select, .. }) => *select,
            _ => None,
        }
    }

    fn set_cached_select(&mut self, option: NodeId, value: Option<NodeId>) {
        if let Some(State::Option { select, .. }) = self.state_mut(option) {
            *select = value;
        }
    }

    fn selectedcontent_disabled(&self, node: NodeId) -> bool {
        matches!(
            self.state(node),
            Some(State::SelectedContent { disabled: true })
        )
    }

    fn set_selectedcontent_disabled(&mut self, node: NodeId, value: bool) {
        if let Some(State::SelectedContent { disabled }) = self.state_mut(node) {
            *disabled = value;
        }
    }

    // Die Schritte an den Algorithmen des DOM

    /// Die *insertion steps* des HTML Standard.
    pub(crate) fn insertion_steps(&mut self, node: NodeId) {
        match self.html_name(node) {
            // Die zurückgegebenen `select` bleiben hier unbenutzt; das
            // Spiegeln übernehmen die *post-connection steps*.
            Some("option") => {
                self.update_nearest_ancestor_select(node);
            }
            Some("selectedcontent") => {
                self.recalculate_disabledness(node);
            }
            _ => {}
        }
    }

    /// Ob es für den Knoten *post-connection steps* gibt.
    pub(crate) fn has_post_connection_steps(&self, node: NodeId) -> bool {
        matches!(self.html_name(node), Some("option" | "selectedcontent"))
    }

    /// Die *post-connection steps* des HTML Standard.
    pub(crate) fn post_connection_steps(&mut self, node: NodeId) {
        match self.html_name(node) {
            Some("option") => self.update_selectedcontents_for_option(node),
            Some("selectedcontent") => {
                let select = self.recalculate_disabledness(node);
                if let Some(select) = select
                    && !self.selectedcontent_disabled(node)
                    && !self.has_attribute(select, "multiple")
                {
                    self.update_selectedcontent(select, node);
                }
            }
            _ => {}
        }
    }

    /// Die *removing steps* des HTML Standard.
    pub(crate) fn removing_steps(&mut self, node: NodeId) {
        if self.html_name(node) == Some("option") {
            for select in self.update_nearest_ancestor_select(node) {
                self.microtasks
                    .push_back(Microtask::UpdateSelectedcontents(select));
            }
        }
    }

    /// Die Schritte, wenn ein Attribut ohne Namensraum gesetzt wird oder
    /// wegfällt: `was_present` und `is_present` sagen, ob es vorher und
    /// nachher da ist.
    pub(crate) fn attribute_changed(
        &mut self,
        element: NodeId,
        local_name: &DomString,
        was_present: bool,
        is_present: bool,
    ) {
        let toggled = was_present != is_present;
        match (self.html_name(element), local_name.as_ascii()) {
            (Some("option"), Some("disabled")) if toggled => {
                if let Some(select) = self.cached_select(element) {
                    self.reset_selectedness(select, false);
                }
            }
            (Some("option"), Some("selected")) if toggled => {
                let mut has_changed = false;
                if !self.dirtiness(element) && self.selectedness(element) != is_present {
                    self.set_selectedness(element, is_present);
                    has_changed = true;
                }
                if let Some(select) = self.cached_select(element) {
                    if has_changed && is_present {
                        self.deselect_others(select, element);
                    }
                    self.reset_selectedness(select, has_changed);
                }
            }
            (Some("select"), Some("size")) => self.reset_selectedness(element, false),
            (Some("select"), Some("multiple")) if toggled => {
                if !is_present {
                    let mut first = true;
                    for option in self.list_of_options(element) {
                        if self.selectedness(option) {
                            if first {
                                first = false;
                            } else {
                                self.set_selectedness(option, false);
                            }
                        }
                    }
                }
                self.reset_selectedness(element, true);
            }
            _ => {}
        }
    }

    pub(crate) fn run_microtask(&mut self, task: Microtask) {
        match task {
            Microtask::UpdateSelectedcontents(select) => self.update_selectedcontents(select),
        }
    }

    /// Ein HTML- oder XML-Parser hat `element` vom Stapel offener Elemente
    /// genommen („popped off the stack of open elements“). Bisher hat nur
    /// `option` Schritte dafür.
    pub fn popped_by_parser(&mut self, element: NodeId) {
        if self.html_name(element) == Some("option") {
            self.update_selectedcontents_for_option(element);
        }
    }

    // `select` und `option`

    /// „Get the nearest ancestor select“
    fn nearest_ancestor_select(&self, element: NodeId) -> Option<NodeId> {
        let mut seen_optgroup = false;
        let mut ancestor = self.parent(element);
        while let Some(node) = ancestor {
            match self.html_name(node) {
                Some("datalist" | "hr" | "option") => return None,
                Some("optgroup") => {
                    if seen_optgroup {
                        return None;
                    }
                    seen_optgroup = true;
                }
                Some("select") => return Some(node),
                _ => {}
            }
            ancestor = self.parent(node);
        }
        None
    }

    /// „Update an option's nearest ancestor select“: die `select`, deren
    /// `selectedcontent` neu zu füllen sind.
    fn update_nearest_ancestor_select(&mut self, option: NodeId) -> Vec<NodeId> {
        let old = self.cached_select(option);
        let new = self.nearest_ancestor_select(option);
        self.set_cached_select(option, new);
        if old == new {
            return Vec::new();
        }
        let mut result = Vec::new();
        for select in [old, new].into_iter().flatten() {
            if Some(select) == new && self.selectedness(option) {
                self.deselect_others(select, option);
            }
            let has_changed = self.selectedness_setting(select);
            if self.selectedness(option) || has_changed {
                result.push(select);
            }
        }
        result
    }

    /// Ohne `multiple` bleibt nur `option` gewählt: „whenever an option
    /// element in the select element's list of options has its selectedness
    /// set to true, and whenever an option element with its selectedness set
    /// to true is added to the select element's list of options, the user
    /// agent must set the selectedness of all the other option elements in
    /// its list of options to false.“
    fn deselect_others(&mut self, select: NodeId, option: NodeId) {
        if self.has_attribute(select, "multiple") {
            return;
        }
        for other in self.list_of_options(select) {
            if other != option {
                self.set_selectedness(other, false);
            }
        }
    }

    /// „Get the list of options“
    fn list_of_options(&self, select: NodeId) -> Vec<NodeId> {
        let mut options = Vec::new();
        let mut node = self.first_child(select);
        while let Some(current) = node {
            let name = self.html_name(current);
            if name == Some("option") {
                options.push(current);
            }
            let skip = matches!(name, Some("select" | "hr" | "option" | "datalist"))
                || (name == Some("optgroup") && self.has_optgroup_between(current, select));
            node = if skip {
                self.following_skipping_children(current, select)
            } else {
                self.following(current, select)
            };
        }
        options
    }

    /// Ob zwischen `node` und seinem Vorfahren `select` ein `optgroup` liegt.
    fn has_optgroup_between(&self, node: NodeId, select: NodeId) -> bool {
        let mut ancestor = self.parent(node);
        while let Some(current) = ancestor {
            if current == select {
                return false;
            }
            if self.html_name(current) == Some("optgroup") {
                return true;
            }
            ancestor = self.parent(current);
        }
        false
    }

    /// Ob eine `option` abgeschaltet ist (*disabled*).
    fn option_is_disabled(&self, option: NodeId) -> bool {
        if self.has_attribute(option, "disabled") {
            return true;
        }
        let mut ancestor = self.parent(option);
        while let Some(node) = ancestor {
            match self.html_name(node) {
                Some("select" | "hr" | "datalist" | "option") => return false,
                Some("optgroup") => return self.has_attribute(node, "disabled"),
                _ => {}
            }
            ancestor = self.parent(node);
        }
        false
    }

    /// *display size* eines `select`
    fn display_size(&self, select: NodeId) -> u64 {
        let size = self
            .element(select)
            .and_then(|element| element.attribute("size"))
            .and_then(parse_non_negative_integer);
        match size {
            Some(size) => size,
            None if self.has_attribute(select, "multiple") => 4,
            None => 1,
        }
    }

    /// „Selectedness setting algorithm“: `true`, wenn sich die Auswahl
    /// geändert hat.
    fn selectedness_setting(&mut self, select: NodeId) -> bool {
        if self.has_attribute(select, "multiple") {
            return false;
        }
        let mut has_changed = false;
        let mut first_enabled = None;
        let mut last_selected: Option<NodeId> = None;
        for option in self.list_of_options(select) {
            if self.selectedness(option) {
                if let Some(last) = last_selected {
                    self.set_selectedness(last, false);
                    has_changed = true;
                }
                last_selected = Some(option);
            }
            if first_enabled.is_none() && !self.option_is_disabled(option) {
                first_enabled = Some(option);
            }
        }
        if last_selected.is_none()
            && let Some(first) = first_enabled
            && self.display_size(select) == 1
        {
            self.set_selectedness(first, true);
            has_changed = true;
        }
        has_changed
    }

    /// „Reset a select's selectedness“
    fn reset_selectedness(&mut self, select: NodeId, has_changed: bool) {
        let mut has_changed = has_changed;
        if self.selectedness_setting(select) {
            has_changed = true;
        }
        if has_changed {
            self.update_selectedcontents(select);
        }
    }

    // `selectedcontent`

    /// „Update descendant selectedcontent elements for an option“
    fn update_selectedcontents_for_option(&mut self, option: NodeId) {
        let Some(select) = self.cached_select(option) else {
            return;
        };
        if self.selectedness(option) {
            self.update_selectedcontents(select);
        }
    }

    /// „Update a select's descendant selectedcontent elements“
    fn update_selectedcontents(&mut self, select: NodeId) {
        if self.has_attribute(select, "multiple") {
            return;
        }
        let mut targets = Vec::new();
        let mut node = self.first_child(select);
        while let Some(current) = node {
            if self.html_name(current) == Some("selectedcontent")
                && !self.selectedcontent_disabled(current)
            {
                targets.push(current);
            }
            node = self.following(current, select);
        }
        for selectedcontent in targets {
            self.update_selectedcontent(select, selectedcontent);
        }
    }

    /// „Update a selectedcontent“
    fn update_selectedcontent(&mut self, select: NodeId, selectedcontent: NodeId) {
        if self.selectedcontent_disabled(selectedcontent) {
            return;
        }
        let option = self
            .list_of_options(select)
            .into_iter()
            .find(|&option| self.selectedness(option));
        match option {
            // „Clear a selectedcontent“
            None => self.replace_all_unchecked(None, selectedcontent),
            Some(option) => self.clone_option_into(option, selectedcontent),
        }
    }

    /// „Clone an option into a selectedcontent“
    fn clone_option_into(&mut self, option: NodeId, selectedcontent: NodeId) {
        let fragment = self.create_fragment();
        let children: Vec<NodeId> = self.children(option).collect();
        for child in children {
            if let Some(copy) = self.clone_node(child, true) {
                self.insert(copy, fragment, None);
            }
        }
        self.replace_all_unchecked(Some(fragment), selectedcontent);
    }

    /// „Recalculate a selectedcontent element's disabledness“: gibt das
    /// nächste `select` darüber zurück.
    fn recalculate_disabledness(&mut self, selectedcontent: NodeId) -> Option<NodeId> {
        let mut nearest = None;
        let mut disabled = false;
        let mut ancestor = self.parent(selectedcontent);
        while let Some(node) = ancestor {
            match self.html_name(node) {
                Some("select") if nearest.is_none() => nearest = Some(node),
                Some("select" | "option" | "selectedcontent") => {
                    disabled = true;
                    break;
                }
                _ => {}
            }
            ancestor = self.parent(node);
        }
        self.set_selectedcontent_disabled(selectedcontent, disabled);
        nearest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(document: &mut Document, name: &str, attributes: &[(&str, &str)]) -> NodeId {
        let attributes = attributes
            .iter()
            .map(|&(name, value)| Attribute::new(DomString::from(name), DomString::from(value)))
            .collect();
        document.create_element(Some(Namespace::Html), DomString::from(name), attributes)
    }

    fn append(document: &mut Document, parent: NodeId, child: NodeId) {
        document.append(parent, child).expect("einfügbar");
    }

    /// Eine `option` mit einem Textknoten.
    fn option(document: &mut Document, label: &str, attributes: &[(&str, &str)]) -> NodeId {
        let option = element(document, "option", attributes);
        let text = document.create_text(DomString::from(label));
        append(document, option, text);
        option
    }

    /// Der Text der Kinder, Elemente als `<name>`.
    fn contents(document: &Document, node: NodeId) -> String {
        let mut out = String::new();
        let mut current = document.first_child(node);
        while let Some(child) = current {
            match (document.element(child), document.data(child)) {
                (Some(element), _) => {
                    out.push('<');
                    out.push_str(&element.local_name.to_string_lossy());
                    out.push('>');
                }
                (None, Some(data)) => out.push_str(&data.to_string_lossy()),
                (None, None) => {}
            }
            current = document.following(child, node);
        }
        out
    }

    #[test]
    fn parse_non_negative_integers() {
        let parse = |input: &str| parse_non_negative_integer(&DomString::from(input));
        assert_eq!(parse("3"), Some(3));
        assert_eq!(parse(" \t+12px"), Some(12));
        assert_eq!(parse("-0"), Some(0));
        assert_eq!(parse("-1"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("+"), None);
        assert_eq!(parse("x1"), None);
        assert_eq!(parse("99999999999999999999999"), Some(u64::MAX));
    }

    #[test]
    fn options_follow_insertion_and_removal() {
        let mut document = Document::new();
        let root = document.root();
        let select = element(&mut document, "select", &[]);
        append(&mut document, root, select);
        let a = option(&mut document, "A", &[]);
        append(&mut document, select, a);
        // Ohne gewählte Option wählt der Algorithmus die erste mögliche.
        assert!(document.selectedness(a));
        let b = option(&mut document, "B", &[("selected", "")]);
        append(&mut document, select, b);
        assert!(!document.selectedness(a));
        assert!(document.selectedness(b));
        assert_eq!(document.cached_select(b), Some(select));
        document.remove(b);
        assert_eq!(document.cached_select(b), None);
        assert!(document.selectedness(a));
        // Mit `size` über 1 wählt niemand von selbst.
        let list = element(&mut document, "select", &[("size", "2")]);
        let c = option(&mut document, "C", &[]);
        append(&mut document, list, c);
        assert!(!document.selectedness(c));
    }

    #[test]
    fn list_of_options_skips_nested_groups() {
        let mut document = Document::new();
        let select = element(&mut document, "select", &[]);
        let group = element(&mut document, "optgroup", &[]);
        let inner = element(&mut document, "optgroup", &[]);
        let hidden = option(&mut document, "versteckt", &[]);
        let div = element(&mut document, "div", &[]);
        let shown = option(&mut document, "sichtbar", &[]);
        let datalist = element(&mut document, "datalist", &[]);
        let listed = option(&mut document, "Liste", &[]);
        append(&mut document, select, group);
        append(&mut document, group, inner);
        append(&mut document, inner, hidden);
        append(&mut document, select, div);
        append(&mut document, div, shown);
        append(&mut document, select, datalist);
        append(&mut document, datalist, listed);
        assert_eq!(document.list_of_options(select), [shown]);
        assert_eq!(document.nearest_ancestor_select(hidden), None);
        assert_eq!(document.nearest_ancestor_select(shown), Some(select));
        // Ein `disabled` am `optgroup` gilt für seine Optionen.
        let disabled_group = element(&mut document, "optgroup", &[("disabled", "")]);
        let grouped = option(&mut document, "G", &[]);
        append(&mut document, disabled_group, grouped);
        assert!(document.option_is_disabled(grouped));
        assert!(!document.option_is_disabled(shown));
    }

    #[test]
    fn selected_option_deselects_the_others() {
        let mut document = Document::new();
        let root = document.root();
        let select = element(&mut document, "select", &[]);
        let button = element(&mut document, "button", &[]);
        let selectedcontent = element(&mut document, "selectedcontent", &[]);
        append(&mut document, root, select);
        append(&mut document, select, button);
        append(&mut document, button, selectedcontent);
        let a = option(&mut document, "A", &[]);
        let b = option(&mut document, "B", &[("selected", "")]);
        append(&mut document, select, a);
        append(&mut document, select, b);
        assert_eq!(contents(&document, selectedcontent), "B");
        // `selected` an einer früheren Option wählt sie statt der letzten.
        document.append_attribute(
            a,
            Attribute::new(DomString::from("selected"), DomString::new()),
        );
        assert!(document.selectedness(a));
        assert!(!document.selectedness(b));
        assert_eq!(contents(&document, selectedcontent), "A");
        // Ebenso eine gewählte Option, die davor eingefügt wird.
        let c = option(&mut document, "C", &[("selected", "")]);
        document
            .insert_before(select, c, Some(a))
            .expect("einfügbar");
        assert!(document.selectedness(c));
        assert!(!document.selectedness(a));
        assert_eq!(contents(&document, selectedcontent), "C");
        // Mit `multiple` bleiben alle gewählt.
        document.append_attribute(
            select,
            Attribute::new(DomString::from("multiple"), DomString::new()),
        );
        let d = option(&mut document, "D", &[("selected", "")]);
        document
            .insert_before(select, d, Some(c))
            .expect("einfügbar");
        assert!(document.selectedness(c));
        assert!(document.selectedness(d));
    }

    #[test]
    fn selectedcontent_mirrors_the_selected_option() {
        let mut document = Document::new();
        let root = document.root();
        let select = element(&mut document, "select", &[]);
        let button = element(&mut document, "button", &[]);
        let selectedcontent = element(&mut document, "selectedcontent", &[]);
        append(&mut document, root, select);
        append(&mut document, select, button);
        append(&mut document, button, selectedcontent);
        let a = option(&mut document, "A", &[]);
        let bold = element(&mut document, "b", &[]);
        append(&mut document, a, bold);
        append(&mut document, select, a);
        assert_eq!(contents(&document, selectedcontent), "A<b>");
        let b = option(&mut document, "B", &[]);
        append(&mut document, select, b);
        assert_eq!(contents(&document, selectedcontent), "A<b>");
        // Das Entfernen meldet sich über eine Mikroaufgabe.
        document.remove(a);
        assert_eq!(contents(&document, selectedcontent), "A<b>");
        document.perform_microtask_checkpoint();
        assert_eq!(contents(&document, selectedcontent), "B");
        // Das Attribut `selected` wählt um.
        let c = option(&mut document, "C", &[]);
        append(&mut document, select, c);
        document.append_attribute(
            c,
            Attribute::new(DomString::from("selected"), DomString::new()),
        );
        assert_eq!(contents(&document, selectedcontent), "C");
        // Ein `selectedcontent` in einer `option` bleibt außen vor.
        let nested = element(&mut document, "selectedcontent", &[]);
        append(&mut document, c, nested);
        assert!(document.selectedcontent_disabled(nested));
        assert!(!document.selectedcontent_disabled(selectedcontent));
        // `multiple` schaltet das Spiegeln ab: Ohne bliebe nach dem Entfernen
        // von C nur B.
        document.append_attribute(
            select,
            Attribute::new(DomString::from("multiple"), DomString::new()),
        );
        document.remove(c);
        document.perform_microtask_checkpoint();
        assert_eq!(contents(&document, selectedcontent), "C");
    }
}
