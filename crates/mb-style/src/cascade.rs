//! Die Kaskade nach CSS Cascade 5: Sie sammelt für ein Element die
//! Deklarationen, die zutreffen, sortiert sie und liefert je Langform den
//! kaskadierten Wert (*cascaded value*).
//!
//! Sortiert wird nach Ursprung und Wichtigkeit, dann ob eine Deklaration am
//! Element hängt (Attribut `style`), dann nach Ebene (`@layer`),
//! Spezifität und Reihenfolge. `revert`, `revert-layer` und `revert-rule`
//! löst die Kaskade selbst auf; `initial`, `inherit` und `unset` reicht sie
//! weiter. In einer *logical property group* werden die Deklarationen der
//! logischen und der zugeordneten physischen Langform zusammen kaskadiert
//! (CSS Logical 1); dafür muss vorher der Schreibmodus feststehen.
//!
//! Die Media Queries in `@media`, nach `@import` und im Attribut `media`
//! wertet der [`StylistBuilder`] für ein Gerät aus; Regeln, deren
//! Bedingungen nicht zutreffen, und die Ebenen darin zählen nicht. Ob ein
//! [`Stylist`] auch für ein anderes Gerät gilt, sagt
//! [`Stylist::is_valid_for`].
//!
//! Es fehlen die Ursprünge für Animationen und Übergänge, Shadow Trees
//! (*context*) und `@scope`.
//!
//! `revert-layer` in einer Deklaration im Attribut `style` nimmt nur die
//! Deklarationen im Attribut zurück, als bildeten sie eine eigene Ebene.
//! So erwarten es die WPT-Tests (`css/css-cascade/revert-layer-009.html`),
//! während der Entwurf für normale Deklarationen nur sagt, sie lägen in der
//! impliziten letzten Ebene; dann fielen auch die Regeln ohne Ebene weg.

#[cfg(test)]
mod tests;

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mb_base::{Budget, DomString, Exhausted};
use mb_dom::NodeId;

use crate::computed::{Device, FontMetricsSource, NoFonts};
use crate::declarations::Declared;
use crate::media::MediaQueryList;
use crate::properties::{
    CssWideKeyword, DeclaredValue, FlowMode, Longhand, PropertyDeclaration, SpecifiedValue,
};
use crate::selectors::{Key, MatchingContext, PseudoElement, SelectorList, Specificity};
use crate::stylesheet::{CssRule, LayerName, MAX_IMPORT_DEPTH, StyleSheet};
use crate::ua::{DocumentInfo, sheets_for};
use crate::values::color::{Rgba, SystemColor};

/// Woher ein Stylesheet kommt (*cascade origin*).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    UserAgent,
    User,
    /// *Author presentational hint origin*: zwischen Nutzer und Autor; für
    /// `revert` zählt er zum Autor.
    PresentationalHint,
    Author,
}

impl Origin {
    fn index(self) -> usize {
        match self {
            Origin::UserAgent => 0,
            Origin::User => 1,
            Origin::PresentationalHint => 2,
            Origin::Author => 3,
        }
    }

    /// Wie weit `revert` zurückgeht: alles ab dieser Stufe fällt weg.
    fn revert_level(self) -> u8 {
        match self {
            Origin::UserAgent => 0,
            Origin::User => 1,
            Origin::PresentationalHint | Origin::Author => 2,
        }
    }
}

/// Ursprung und Wichtigkeit, von schwach nach stark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    UserAgent,
    User,
    PresentationalHint,
    Author,
    ImportantAuthor,
    ImportantPresentationalHint,
    ImportantUser,
    ImportantUserAgent,
}

impl Tier {
    fn new(origin: Origin, important: bool) -> Tier {
        match (origin, important) {
            (Origin::UserAgent, false) => Tier::UserAgent,
            (Origin::User, false) => Tier::User,
            (Origin::PresentationalHint, false) => Tier::PresentationalHint,
            (Origin::Author, false) => Tier::Author,
            (Origin::Author, true) => Tier::ImportantAuthor,
            (Origin::PresentationalHint, true) => Tier::ImportantPresentationalHint,
            (Origin::User, true) => Tier::ImportantUser,
            (Origin::UserAgent, true) => Tier::ImportantUserAgent,
        }
    }

    fn origin(self) -> Origin {
        match self {
            Tier::UserAgent | Tier::ImportantUserAgent => Origin::UserAgent,
            Tier::User | Tier::ImportantUser => Origin::User,
            Tier::PresentationalHint | Tier::ImportantPresentationalHint => {
                Origin::PresentationalHint
            }
            Tier::Author | Tier::ImportantAuthor => Origin::Author,
        }
    }

    fn important(self) -> bool {
        self >= Tier::ImportantAuthor
    }
}

/// Die Stelle einer Deklaration in der Sortierung; die größere gewinnt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Priority {
    tier: Tier,
    /// Ob die Deklaration am Element hängt (*element-attached styles*).
    attached: bool,
    /// Die Stelle der Ebene; bei wichtigen Deklarationen umgekehrt, denn
    /// dort gewinnt die frühere Ebene.
    layer: u32,
    specificity: Specificity,
    /// Die Regel in der Reihenfolge des Erscheinens; das Attribut `style`
    /// kommt nach allen Stylesheets.
    rule: u32,
    /// Die Deklaration in der Regel.
    index: u32,
}

/// Die Regel des Attributs `style` in [`Priority::rule`].
const STYLE_ATTRIBUTE: u32 = u32::MAX;

/// Die Regel der *presentational hints* eines Elements in
/// [`Priority::rule`], nach den Blöcken des Standards.
const ELEMENT_HINTS: u32 = u32::MAX - 1;

/// Die Ebenen eines Ursprungs als Baum (*layer names*). Knoten 0 ist die
/// äußere Ebene für alles ohne `@layer`.
#[derive(Debug)]
struct Layers {
    nodes: Vec<LayerNode>,
}

#[derive(Debug)]
struct LayerNode {
    /// Benannte Kinder separat nachschlagen; ihre Reihenfolge bleibt in
    /// `children`. Anonyme Ebenen werden nicht in den Index aufgenommen.
    named: BTreeMap<String, usize>,
    children: Vec<usize>,
}

impl Layers {
    fn new() -> Layers {
        Layers {
            nodes: vec![LayerNode {
                named: BTreeMap::new(),
                children: Vec::new(),
            }],
        }
    }

    /// Die Ebene `name` in `parent`, neu angelegt, wenn es sie noch nicht
    /// gibt; anonyme Ebenen sind immer neu.
    fn child(
        &mut self,
        parent: usize,
        name: Option<&str>,
        budget: &Budget,
    ) -> Result<usize, Exhausted> {
        spend(budget, 1)?;
        if let Some(name) = name {
            spend(budget, u64::try_from(name.len() / 64).unwrap_or(u64::MAX))?;
        }
        if let Some(name) = name
            && let Some(&existing) = self.nodes.get(parent).and_then(|node| node.named.get(name))
        {
            return Ok(existing);
        }
        let index = self.nodes.len();
        self.nodes.push(LayerNode {
            named: BTreeMap::new(),
            children: Vec::new(),
        });
        if let Some(node) = self.nodes.get_mut(parent) {
            node.children.push(index);
            if let Some(name) = name {
                node.named.insert(name.to_owned(), index);
            }
        }
        Ok(index)
    }

    /// Die Ebene zu einem ⟨layer-name⟩ in `parent`.
    fn path(
        &mut self,
        parent: usize,
        name: &LayerName,
        budget: &Budget,
    ) -> Result<usize, Exhausted> {
        name.iter().try_fold(parent, |current, segment| {
            self.child(current, Some(segment), budget)
        })
    }

    /// Die Stelle jeder Ebene in der Reihenfolge der Ebenen: verschachtelte
    /// vor ihrer Elternebene, in der Reihenfolge ihres ersten Erscheinens;
    /// die äußere Ebene zuletzt.
    fn ranks(&self, budget: &Budget) -> Result<Vec<u32>, Exhausted> {
        spend(budget, u64::try_from(self.nodes.len()).unwrap_or(u64::MAX))?;
        let mut ranks = vec![0; self.nodes.len()];
        let mut next: u32 = 0;
        let mut stack = vec![(0, 0)];
        while let Some((node, child)) = stack.pop() {
            spend(budget, 1)?;
            let children = self
                .nodes
                .get(node)
                .map(|node| node.children.as_slice())
                .unwrap_or_default();
            match children.get(child) {
                Some(&first) => {
                    stack.push((node, child.saturating_add(1)));
                    stack.push((first, 0));
                }
                None => {
                    if let Some(rank) = ranks.get_mut(node) {
                        *rank = next;
                    }
                    next = next.saturating_add(1);
                }
            }
        }
        Ok(ranks)
    }
}

/// Eine Regel in der Kaskade: Selektoren und Deklarationen einer Stilregel
/// oder einer *nested declarations rule*.
#[derive(Debug)]
struct Entry {
    selectors: Arc<SelectorList>,
    declarations: Arc<[Declared]>,
    origin: Origin,
    /// Erst der Knoten in [`Layers`], nach [`StylistBuilder::build`] die
    /// Stelle der Ebene.
    layer: u32,
}

/// Ein Selektor einer Regel.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    entry: u32,
    selector: u32,
}

/// Sammelt die Stylesheets für einen [`Stylist`] und wertet dabei die
/// Media Queries für ein Gerät aus.
pub struct StylistBuilder<'a> {
    device: &'a Device,
    fonts: &'a dyn FontMetricsSource,
    layers: [Layers; 4],
    entries: Vec<Entry>,
    /// Die ausgewerteten Media Queries mit ihrem Ergebnis.
    media: Vec<(Arc<MediaQueryList>, bool)>,
    /// Nach einer abgebrochenen Ergänzung darf kein Teilstand als gültiger
    /// Stylist herausgegeben werden, auch nicht mit einem neuen Budget.
    failure: Option<Exhausted>,
}

impl std::fmt::Debug for StylistBuilder<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StylistBuilder")
            .field("device", self.device)
            .field("layers", &self.layers)
            .field("entries", &self.entries)
            .finish_non_exhaustive()
    }
}

impl Default for StylistBuilder<'static> {
    fn default() -> StylistBuilder<'static> {
        StylistBuilder::new()
    }
}

/// Das Gerät von [`StylistBuilder::new`]: ein Viewport ohne Fläche.
static NO_DEVICE: Device = Device {
    viewport_width: 0.0,
    viewport_height: 0.0,
    device_pixel_ratio: 1.0,
    system_color: black,
};

/// Die Systemfarben von [`NO_DEVICE`]; berechnete Werte entstehen damit
/// nicht.
fn black(_: SystemColor) -> Rgba {
    Rgba {
        red: 0,
        green: 0,
        blue: 0,
        alpha: 255,
    }
}

impl StylistBuilder<'static> {
    /// Ohne Gerät: Media Queries gelten für einen Viewport ohne Fläche und
    /// ohne Schriften.
    pub fn new() -> StylistBuilder<'static> {
        StylistBuilder::for_device(&NO_DEVICE, &NoFonts)
    }
}

impl<'a> StylistBuilder<'a> {
    /// Media Queries gelten für `device`; die Maße der Schriften für
    /// schriftbezogene Einheiten darin kommen aus `fonts`.
    pub fn for_device(device: &'a Device, fonts: &'a dyn FontMetricsSource) -> StylistBuilder<'a> {
        StylistBuilder {
            device,
            fonts,
            layers: [Layers::new(), Layers::new(), Layers::new(), Layers::new()],
            entries: Vec::new(),
            media: Vec::new(),
            failure: None,
        }
    }

    /// Die Stylesheets aus dem HTML Standard für ein Dokument; sie gehören
    /// vor alle anderen.
    pub fn add_user_agent_sheets(&mut self, info: DocumentInfo) {
        self.add_user_agent_sheets_within(info, &Budget::unlimited())
            .expect("unbegrenztes Budget");
    }

    /// Wie [`StylistBuilder::add_user_agent_sheets`], aber mit Arbeitsbudget.
    pub fn add_user_agent_sheets_within(
        &mut self,
        info: DocumentInfo,
        budget: &Budget,
    ) -> Result<(), Exhausted> {
        if let Some(reason) = self.failure {
            return Err(reason);
        }
        let result = (|| {
            spend(budget, 1)?;
            for (origin, sheet) in sheets_for(info) {
                self.add_within(origin, sheet, budget)?;
            }
            spend(budget, 0)
        })();
        if let Err(reason) = result {
            self.failure = Some(reason);
        }
        result
    }

    /// Ein Stylesheet; die Reihenfolge der Aufrufe ist die Reihenfolge des
    /// Erscheinens (*final CSS style sheets*). Ohne Arbeitsgrenze, für
    /// vertrauenswürdige Eingaben. Für fremde Inhalte gibt es `add_within`.
    pub fn add(&mut self, origin: Origin, sheet: &StyleSheet) {
        self.add_within(origin, sheet, &Budget::unlimited())
            .expect("unbegrenztes Budget");
    }

    /// Ergänzt Regeln, Imports und Ebenen aus `budget`. Nach einem Fehler
    /// bleibt der Builder unvollständig und kann nicht mehr gebaut werden.
    pub fn add_within(
        &mut self,
        origin: Origin,
        sheet: &StyleSheet,
        budget: &Budget,
    ) -> Result<(), Exhausted> {
        if let Some(reason) = self.failure {
            return Err(reason);
        }
        let result = self
            .sheet(origin, sheet, 0, 0, budget)
            .and_then(|()| spend(budget, 0));
        if let Err(reason) = result {
            self.failure = Some(reason);
        }
        result
    }

    fn sheet(
        &mut self,
        origin: Origin,
        sheet: &StyleSheet,
        layer: usize,
        depth: usize,
        budget: &Budget,
    ) -> Result<(), Exhausted> {
        spend(budget, 1)?;
        if let Some(media) = &sheet.media
            && !self.evaluate(media, budget)?
        {
            return Ok(());
        }
        let (before, after) = sheet
            .rules
            .split_at_checked(sheet.import_position)
            .unwrap_or((&sheet.rules, &[]));
        self.rules(origin, before, layer, None, budget)?;
        for import in &sheet.imports {
            spend(budget, 1)?;
            if !import.supports || !self.evaluate(&import.media, budget)? {
                continue;
            }
            let node = match &import.layer {
                None => layer,
                Some(Some(name)) => self.layers_mut(origin).path(layer, name, budget)?,
                Some(None) => self.layers_mut(origin).child(layer, None, budget)?,
            };
            if let Some(imported) = &import.sheet
                && depth < MAX_IMPORT_DEPTH
            {
                self.sheet(origin, imported, node, depth.saturating_add(1), budget)?;
            }
        }
        self.rules(origin, after, layer, None, budget)
    }

    /// Ein Fehler in der Auswertung darf nicht als falsche Media Query
    /// gespeichert werden; nur vollständige Ergebnisse kommen in den Cache.
    fn evaluate(
        &mut self,
        queries: &Arc<MediaQueryList>,
        budget: &Budget,
    ) -> Result<bool, Exhausted> {
        spend(budget, 1)?;
        if queries.is_empty() {
            return Ok(true);
        }
        let matches = queries.matches_within(self.device, self.fonts, budget)?;
        self.media.push((Arc::clone(queries), matches));
        Ok(matches)
    }

    fn rules(
        &mut self,
        origin: Origin,
        rules: &[CssRule],
        layer: usize,
        parent: Option<&Arc<SelectorList>>,
        budget: &Budget,
    ) -> Result<(), Exhausted> {
        for rule in rules {
            spend(budget, 1)?;
            match rule {
                CssRule::Style(rule) => {
                    self.push(origin, &rule.selectors, &rule.declarations, layer);
                    self.rules(origin, &rule.rules, layer, Some(&rule.selectors), budget)?;
                }
                CssRule::NestedDeclarations(declarations) => {
                    if let Some(parent) = parent {
                        self.push(origin, parent, declarations, layer);
                    }
                }
                CssRule::LayerBlock { name, rules } => {
                    let layers = self.layers_mut(origin);
                    let node = match name {
                        Some(name) => layers.path(layer, name, budget)?,
                        None => layers.child(layer, None, budget)?,
                    };
                    self.rules(origin, rules, node, parent, budget)?;
                }
                CssRule::LayerStatement(names) => {
                    let layers = self.layers_mut(origin);
                    for name in names {
                        layers.path(layer, name, budget)?;
                    }
                }
                CssRule::Media { queries, rules } => {
                    if self.evaluate(queries, budget)? {
                        self.rules(origin, rules, layer, parent, budget)?;
                    }
                }
                CssRule::Supports { matches, rules } => {
                    if *matches {
                        self.rules(origin, rules, layer, parent, budget)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn layers_mut(&mut self, origin: Origin) -> &mut Layers {
        let [user_agent, user, hints, author] = &mut self.layers;
        match origin {
            Origin::UserAgent => user_agent,
            Origin::User => user,
            Origin::PresentationalHint => hints,
            Origin::Author => author,
        }
    }

    fn push(
        &mut self,
        origin: Origin,
        selectors: &Arc<SelectorList>,
        declarations: &Arc<[Declared]>,
        layer: usize,
    ) {
        if declarations.is_empty() {
            return;
        }
        self.entries.push(Entry {
            selectors: Arc::clone(selectors),
            declarations: Arc::clone(declarations),
            origin,
            layer: u32::try_from(layer).unwrap_or(u32::MAX),
        });
    }

    /// Ordnet die Ebenen und sortiert die Selektoren vor.
    pub fn build(self) -> Stylist {
        self.build_within(&Budget::unlimited())
            .expect("unbegrenztes Budget")
    }

    /// Ordnet die Ebenen und indexiert die Selektoren aus `budget`.
    /// Ein zuvor abgebrochener Aufbau liefert immer einen Fehler.
    pub fn build_within(self, budget: &Budget) -> Result<Stylist, Exhausted> {
        if let Some(reason) = self.failure {
            return Err(reason);
        }
        spend(budget, 1)?;
        let mut ranks = Vec::with_capacity(self.layers.len());
        for layers in &self.layers {
            ranks.push(layers.ranks(budget)?);
        }
        let mut entries = self.entries;
        for entry in &mut entries {
            spend(budget, 1)?;
            let node = usize::try_from(entry.layer).unwrap_or(usize::MAX);
            entry.layer = ranks
                .get(entry.origin.index())
                .and_then(|ranks| ranks.get(node))
                .copied()
                .unwrap_or(0);
        }
        let mut index = Index::default();
        for (entry_number, entry) in entries.iter().enumerate() {
            spend(budget, 1)?;
            let Ok(entry_number) = u32::try_from(entry_number) else {
                break;
            };
            for (selector_number, selector) in entry.selectors.selectors().iter().enumerate() {
                spend(budget, 1)?;
                let Ok(selector_number) = u32::try_from(selector_number) else {
                    break;
                };
                let candidate = Candidate {
                    entry: entry_number,
                    selector: selector_number,
                };
                let bucket = match selector.key_within(budget)? {
                    Key::Id(id) => index.ids.entry(lower_within(id, budget)?).or_default(),
                    Key::Class(class) => index
                        .classes
                        .entry(lower_within(class, budget)?)
                        .or_default(),
                    Key::LocalName(name) => {
                        index.names.entry(lower_within(name, budget)?).or_default()
                    }
                    Key::Any => &mut index.any,
                };
                bucket.push(candidate);
            }
        }
        spend(budget, 0)?;
        Ok(Stylist {
            entries,
            index,
            media: self.media,
        })
    }
}

/// `check()` allein erkennt kein neu gesetztes Cancel; spend pollt es.
fn spend(budget: &Budget, steps: u64) -> Result<(), Exhausted> {
    budget.spend(steps);
    budget.check()
}

/// Auch sehr lange Bezeichner müssen während des Indexierens abbrechbar sein.
fn lower_within(value: &DomString, budget: &Budget) -> Result<Vec<u16>, Exhausted> {
    spend(budget, u64::try_from(value.len() / 64).unwrap_or(u64::MAX))?;
    let mut lower = Vec::with_capacity(value.len());
    for (index, unit) in value.code_units().enumerate() {
        if index % 64 == 0 {
            spend(budget, 1)?;
        }
        lower.push(if (0x41..=0x5A).contains(&unit) {
            unit + 0x20
        } else {
            unit
        });
    }
    Ok(lower)
}

/// Die Codeeinheiten in ASCII-Kleinbuchstaben, als Schlüssel.
fn lower(value: &DomString) -> Vec<u16> {
    value.to_ascii_lowercase().code_units().collect()
}

/// Die Selektoren nach dem Merkmal ihres Subjekts.
#[derive(Debug, Default)]
struct Index {
    ids: BTreeMap<Vec<u16>, Vec<Candidate>>,
    classes: BTreeMap<Vec<u16>, Vec<Candidate>>,
    names: BTreeMap<Vec<u16>, Vec<Candidate>>,
    any: Vec<Candidate>,
}

/// Alle Stylesheets eines Dokuments, bereit für die Kaskade.
#[derive(Debug)]
pub struct Stylist {
    entries: Vec<Entry>,
    index: Index,
    /// Die Media Queries, die beim Bauen ausgewertet wurden, mit ihrem
    /// Ergebnis.
    media: Vec<(Arc<MediaQueryList>, bool)>,
}

/// Leerraum in den Werten von `class` (ASCII whitespace).
fn is_whitespace(unit: u16) -> bool {
    matches!(unit, 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

impl Stylist {
    /// Ob die Regeln auch für ein anderes Gerät gelten: Jede Media Query,
    /// die beim Bauen ausgewertet wurde, ergibt dort dasselbe. Sonst muss
    /// ein neuer [`Stylist`] her.
    pub fn is_valid_for(&self, device: &Device, fonts: &dyn FontMetricsSource) -> bool {
        self.is_valid_for_within(device, fonts, &Budget::unlimited())
            .expect("unbegrenztes Budget")
    }

    /// Wie [`Stylist::is_valid_for`], aber auch die Cacheprüfung bezahlt
    /// jede Media Query aus `budget`. Abbruch ist kein Cache-Miss.
    pub fn is_valid_for_within(
        &self,
        device: &Device,
        fonts: &dyn FontMetricsSource,
        budget: &Budget,
    ) -> Result<bool, Exhausted> {
        spend(budget, 1)?;
        for (queries, matches) in &self.media {
            if queries.matches_within(device, fonts, budget)? != *matches {
                return Ok(false);
            }
        }
        spend(budget, 0)?;
        Ok(true)
    }

    /// Die Deklarationen, die für ein Element oder eines seiner
    /// Pseudoelemente gelten, sortiert. `hints` sind die *presentational
    /// hints* aus den Attributen des Elements, die nicht als Regeln in den
    /// Stylesheets stehen, `style_attribute` die Deklarationen aus dem
    /// Attribut `style`; für Pseudoelemente gilt beides nicht.
    pub fn matching<'a>(
        &'a self,
        context: &MatchingContext<'_>,
        element: NodeId,
        pseudo_element: Option<PseudoElement>,
        hints: &'a [Declared],
        style_attribute: Option<&'a [Declared]>,
    ) -> MatchedDeclarations<'a> {
        let mut declarations = Vec::new();
        let Some(data) = context.document.element(element) else {
            return MatchedDeclarations { declarations };
        };
        let mut candidates: Vec<&[Candidate]> = vec![self.index.any.as_slice()];
        let mut lookup = |map: &'a BTreeMap<Vec<u16>, Vec<Candidate>>, key: Vec<u16>| {
            if let Some(found) = map.get(&key) {
                candidates.push(found.as_slice());
            }
        };
        lookup(&self.index.names, lower(&data.local_name));
        if let Some(id) = data.attribute("id") {
            lookup(&self.index.ids, lower(id));
        }
        if let Some(classes) = data.attribute("class") {
            // Den Wert zu lesen kostet je 64 Codeeinheiten einen Schritt.
            if let Some(pass) = context.pass {
                pass.budget()
                    .spend(u64::try_from(classes.len() / 64).unwrap_or(u64::MAX));
            }
            let classes = lower(classes);
            let mut seen: BTreeSet<&[u16]> = BTreeSet::new();
            for class in classes.split(|&unit| is_whitespace(unit)) {
                if !class.is_empty() && seen.insert(class) {
                    lookup(&self.index.classes, class.to_vec());
                }
            }
        }

        // Je Regel die größte Spezifität der Selektoren, die treffen.
        let mut matched: Vec<(u32, Specificity)> = Vec::new();
        for candidate in candidates.into_iter().flatten() {
            let Some(entry) = self.entry(candidate.entry) else {
                continue;
            };
            let Some(selector) = usize::try_from(candidate.selector)
                .ok()
                .and_then(|index| entry.selectors.selectors().get(index))
            else {
                continue;
            };
            let matches = match pseudo_element {
                None => selector.matches(element, context),
                Some(pseudo) => selector.matches_pseudo_element(element, pseudo, context),
            };
            if matches {
                matched.push((candidate.entry, selector.specificity()));
            }
        }
        matched.sort_unstable();
        matched.dedup_by(|later, earlier| {
            // Gleiche Regeln stehen nebeneinander, die größte Spezifität
            // zuletzt.
            if later.0 == earlier.0 {
                earlier.1 = later.1.max(earlier.1);
                true
            } else {
                false
            }
        });

        for (number, specificity) in matched {
            let Some(entry) = self.entry(number) else {
                continue;
            };
            // Die *presentational hints* des HTML Standard haben die
            // Spezifität null.
            let specificity = if entry.origin == Origin::PresentationalHint {
                Specificity::default()
            } else {
                specificity
            };
            for (index, declared) in entry.declarations.iter().enumerate() {
                let tier = Tier::new(entry.origin, declared.important);
                let layer = if tier.important() {
                    u32::MAX.saturating_sub(entry.layer)
                } else {
                    entry.layer
                };
                declarations.push(Matched {
                    priority: Priority {
                        tier,
                        attached: false,
                        layer,
                        specificity,
                        rule: number,
                        index: u32::try_from(index).unwrap_or(u32::MAX),
                    },
                    declaration: &declared.declaration,
                });
            }
        }
        if pseudo_element.is_none() {
            for (index, declared) in hints.iter().enumerate() {
                declarations.push(Matched {
                    priority: Priority {
                        // Die Hinweise sind nie wichtig.
                        tier: Tier::PresentationalHint,
                        attached: false,
                        // Außerhalb jeder Ebene; die äußere hat den höchsten
                        // Rang.
                        layer: u32::MAX,
                        specificity: Specificity::default(),
                        rule: ELEMENT_HINTS,
                        index: u32::try_from(index).unwrap_or(u32::MAX),
                    },
                    declaration: &declared.declaration,
                });
            }
        }
        if pseudo_element.is_none()
            && let Some(attribute) = style_attribute
        {
            for (index, declared) in attribute.iter().enumerate() {
                declarations.push(Matched {
                    priority: Priority {
                        tier: Tier::new(Origin::Author, declared.important),
                        attached: true,
                        layer: 0,
                        specificity: Specificity::default(),
                        rule: STYLE_ATTRIBUTE,
                        index: u32::try_from(index).unwrap_or(u32::MAX),
                    },
                    declaration: &declared.declaration,
                });
            }
        }
        declarations.sort_unstable_by_key(|matched| Reverse(matched.priority));
        MatchedDeclarations { declarations }
    }

    fn entry(&self, number: u32) -> Option<&Entry> {
        self.entries.get(usize::try_from(number).ok()?)
    }
}

/// Eine Deklaration, die für ein Element gilt.
#[derive(Clone, Copy, Debug)]
struct Matched<'a> {
    priority: Priority,
    declaration: &'a PropertyDeclaration,
}

/// Die Deklarationen für ein Element, die stärkste zuerst (*output of the
/// cascade*, für alle Langformen zusammen). Leer ergibt die Kaskade nur
/// Anfangswerte und geerbte, wie bei anonymen Kästen.
#[derive(Clone, Debug, Default)]
pub struct MatchedDeclarations<'a> {
    declarations: Vec<Matched<'a>>,
}

/// Der kaskadierte Wert einer Langform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cascaded<'a> {
    /// Die Langform der Deklaration, die gewonnen hat. In einer *logical
    /// property group* kann das die Partnerin der Langform sein, um die es
    /// geht; `inherit` meint dann den Wert dieser Langform beim
    /// Elternelement.
    pub longhand: Longhand,
    pub value: CascadedValue<'a>,
}

/// Ein Wert aus der Grammatik oder eines der CSS-weiten Schlüsselwörter,
/// die nach der Kaskade übrig bleiben.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CascadedValue<'a> {
    Specified(&'a SpecifiedValue),
    Initial,
    Inherit,
    Unset,
}

/// Die kaskadierten Werte aller Langformen eines Elements.
#[derive(Clone, Debug)]
pub struct CascadedValues<'a> {
    /// Je Langform außer den logischen; die teilen sich den Platz der
    /// zugeordneten physischen.
    slots: Vec<Option<Cascaded<'a>>>,
    flow: FlowMode,
}

fn slot(longhand: Longhand) -> usize {
    longhand as usize
}

impl<'a> CascadedValues<'a> {
    /// Der kaskadierte Wert einer Langform; bei einer logischen der Wert der
    /// physischen, die ihr zugeordnet ist.
    pub fn get(&self, longhand: Longhand) -> Option<Cascaded<'a>> {
        self.slots
            .get(slot(longhand.physical(self.flow)))
            .copied()
            .flatten()
    }

    /// Die Zuordnung, nach der die logischen Langformen kaskadiert wurden.
    pub fn flow(&self) -> FlowMode {
        self.flow
    }
}

/// Was nach einem `revert…` in einem Platz nicht mehr zählt.
#[derive(Clone, Copy, Debug)]
enum Skip {
    /// `revert`: alle Ursprünge ab dieser Stufe, siehe
    /// [`Origin::revert_level`].
    Origins(u8),
    /// `revert-layer`: alles bis zum Ende der normalen Stufe der Ebene,
    /// also auch was zwischen ihrer wichtigen und ihrer normalen Stufe liegt.
    From(Priority),
    /// `revert-layer` im Attribut `style`: dessen Deklarationen.
    Attached,
    /// `revert-rule`: die Regel.
    Rule(u32),
}

impl Skip {
    fn covers(self, priority: &Priority) -> bool {
        match self {
            Skip::Origins(level) => priority.tier.origin().revert_level() >= level,
            Skip::From(threshold) => *priority >= threshold,
            Skip::Attached => priority.attached,
            Skip::Rule(rule) => priority.rule == rule,
        }
    }
}

/// Der Stand eines Platzes beim Kaskadieren.
#[derive(Clone, Debug, Default)]
struct Slot<'a> {
    done: bool,
    value: Option<Cascaded<'a>>,
    skips: Vec<Skip>,
}

impl<'a> Slot<'a> {
    /// Bietet dem Platz eine Deklaration an, in absteigender Reihenfolge.
    fn offer(&mut self, matched: &Matched<'a>) {
        if self.done || self.skips.iter().any(|skip| skip.covers(&matched.priority)) {
            return;
        }
        let priority = matched.priority;
        let origin = priority.tier.origin();
        let declaration: &'a PropertyDeclaration = matched.declaration;
        let value = match &declaration.value {
            DeclaredValue::Value(value) => CascadedValue::Specified(value),
            DeclaredValue::CssWide(keyword) => match keyword {
                CssWideKeyword::Initial => CascadedValue::Initial,
                CssWideKeyword::Inherit => CascadedValue::Inherit,
                CssWideKeyword::Unset => CascadedValue::Unset,
                // Im Ursprung der Benutzeragenten wie `unset`.
                CssWideKeyword::Revert if origin == Origin::UserAgent => CascadedValue::Unset,
                CssWideKeyword::Revert => {
                    self.skips.push(Skip::Origins(origin.revert_level()));
                    return;
                }
                CssWideKeyword::RevertLayer if priority.attached => {
                    self.skips.push(Skip::Attached);
                    return;
                }
                CssWideKeyword::RevertLayer => {
                    let layer = if priority.tier.important() {
                        u32::MAX.saturating_sub(priority.layer)
                    } else {
                        priority.layer
                    };
                    self.skips.push(Skip::From(Priority {
                        tier: Tier::new(origin, false),
                        attached: false,
                        layer,
                        specificity: Specificity::default(),
                        rule: 0,
                        index: 0,
                    }));
                    return;
                }
                CssWideKeyword::RevertRule => {
                    self.skips.push(Skip::Rule(priority.rule));
                    return;
                }
            },
        };
        self.done = true;
        self.value = Some(Cascaded {
            longhand: declaration.longhand,
            value,
        });
    }
}

impl<'a> MatchedDeclarations<'a> {
    /// Kaskadiert alle Langformen. Zuerst die ohne *logical property group*,
    /// darunter `writing-mode`, `direction` und `text-orientation`; aus
    /// ihnen bestimmt `flow` die Zuordnung, nach der dann die Gruppen
    /// kaskadiert werden.
    pub fn cascade(
        &self,
        flow: impl FnOnce(&CascadedValues<'a>) -> FlowMode,
    ) -> CascadedValues<'a> {
        let mut slots: Vec<Slot<'a>> = vec![Slot::default(); Longhand::ALL.len()];
        let in_group = |longhand: Longhand| longhand.info().group.is_some();
        for matched in &self.declarations {
            let longhand = matched.declaration.longhand;
            if !in_group(longhand)
                && let Some(slot) = slots.get_mut(slot(longhand))
            {
                slot.offer(matched);
            }
        }
        let mut values = CascadedValues {
            slots: slots.iter().map(|slot| slot.value).collect(),
            flow: FlowMode::default(),
        };
        let mode = flow(&values);
        values.flow = mode;
        for matched in &self.declarations {
            let longhand = matched.declaration.longhand;
            if in_group(longhand)
                && let Some(slot) = slots.get_mut(slot(longhand.physical(mode)))
            {
                slot.offer(matched);
            }
        }
        values.slots = slots.into_iter().map(|slot| slot.value).collect();
        values
    }

    pub fn is_empty(&self) -> bool {
        self.declarations.is_empty()
    }
}
