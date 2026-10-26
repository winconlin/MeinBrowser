//! Der Kastenbaum (*box tree*) nach CSS Display 3 und CSS 2, Abschnitt 9.2,
//! mit den berechneten Stilen, die beim Aufbau entstehen.
//!
//! Ein Blockcontainer enthält entweder nur blockartige Kästen oder einen
//! Inline-Formatierungskontext. Gemischter Inhalt kommt in anonyme
//! Blockkästen; ein Block in einem Inline-Kasten teilt diesen, und die Teile
//! davor und danach haben an der Teilungsstelle keine Kante. Leerraum, der
//! ganz zusammenfällt, erzeugt keinen anonymen Kasten.
//!
//! Was noch fehlt und was stattdessen geschieht: `float` und `position`
//! gibt es noch nicht; Tabellen, Flex- und Grid-Container werden zu
//! `flow-root`, interne Tabellen- und Ruby-Kästen zu Blöcken, `run-in` zu
//! einem Block, Ruby zu einem Inline-Kasten. Listenmarken, `::before`,
//! `::after` und `::first-line` fehlen. Ersetzte Elemente sind bisher nur
//! `img`; `<br>` im HTML-Namensraum ist ein erzwungener Umbruch (der
//! Standard beschreibt es mit `display-outside: newline`, das CSS nicht
//! kennt). Tiefer als [`MAX_DEPTH`] verschachtelte Elemente erzeugen keine
//! Kästen.

use std::sync::Arc;

use mb_css::Source;
use mb_dom::{Document, NodeId, NodeKind};
use mb_scene::Image;
use mb_style::properties::{Display, DisplayInside, DisplayOutside, WhiteSpaceCollapse};
use mb_style::values::ParseContext;
use mb_style::{
    Ancestors, ComputedValues, Device, MatchedDeclarations, MatchingContext, Stylist, flow_mode,
    parse_declaration_block,
};
use mb_url::Url;

use crate::fonts::Fonts;
use crate::style::Style;

/// So tief dürfen Elemente verschachtelt sein, damit Aufbau und Layout, die
/// rekursiv arbeiten, mit 2 MiB Stapel auskommen.
pub(crate) const MAX_DEPTH: usize = 200;

pub(crate) type StyleRef = Arc<ComputedValues>;

/// Ein Blockcontainer: ein Block, ein Inline-Block oder ein anonymer Block.
#[derive(Debug)]
pub(crate) struct BlockBox {
    pub(crate) style: StyleRef,
    /// Das Element; anonyme Kästen haben keins.
    pub(crate) node: Option<NodeId>,
    pub(crate) contents: BlockContents,
    /// Ob der Kasten einen eigenen Blockformatierungskontext begründet.
    pub(crate) independent: bool,
    /// Ob seine erste Zeile die erste formatierte Zeile eines Elements ist:
    /// nicht bei einem anonymen Block, der nicht das erste Kind ist.
    pub(crate) first_formatted_line: bool,
}

#[derive(Debug)]
pub(crate) enum BlockContents {
    Blocks(Vec<BlockLevel>),
    Inline(InlineContent),
}

/// Ein blockartiger Kasten.
#[derive(Debug)]
pub(crate) enum BlockLevel {
    Block(BlockBox),
    Replaced(ReplacedBox),
}

/// Ein ersetztes Element; ohne Bild hat es keine natürliche Größe.
#[derive(Debug)]
pub(crate) struct ReplacedBox {
    pub(crate) style: StyleRef,
    pub(crate) node: NodeId,
    pub(crate) image: Option<Arc<Image>>,
}

/// Ein Teil eines Inline-Kastens. Wird ein Inline-Kasten von einem Block
/// geteilt, hat jedes Teil einen eigenen Eintrag.
#[derive(Debug)]
pub(crate) struct InlineBoxData {
    pub(crate) style: StyleRef,
    pub(crate) node: Option<NodeId>,
    /// Ob das Teil links (am Anfang) und rechts (am Ende) eine Kante mit
    /// Rand, Innenabstand und Außenabstand hat.
    pub(crate) start_edge: bool,
    pub(crate) end_edge: bool,
    /// Der umgebende Inline-Kasten.
    pub(crate) parent: Option<usize>,
}

/// Ein Inline-Kasten auf Atomebene: ein Inline-Block oder ein ersetztes
/// Element.
#[derive(Debug)]
pub(crate) enum Atomic {
    Block(BlockBox),
    Replaced(ReplacedBox),
}

impl Atomic {
    pub(crate) fn style(&self) -> &StyleRef {
        match self {
            Atomic::Block(block) => &block.style,
            Atomic::Replaced(replaced) => &replaced.style,
        }
    }
}

/// Der Inhalt eines Inline-Formatierungskontexts in Dokumentreihenfolge.
/// `parent` ist der Inline-Kasten, in dem ein Eintrag steht; ohne ihn steht
/// er im anonymen Inline-Kasten des Blockcontainers.
#[derive(Debug)]
pub(crate) enum InlineItem {
    Start(usize),
    End(usize),
    Text {
        text: Vec<u32>,
        parent: Option<usize>,
    },
    Atomic {
        atomic: Atomic,
        parent: Option<usize>,
    },
    ForcedBreak {
        parent: Option<usize>,
    },
}

#[derive(Debug, Default)]
pub(crate) struct InlineContent {
    pub(crate) items: Vec<InlineItem>,
    pub(crate) boxes: Vec<InlineBoxData>,
}

impl InlineContent {
    /// Ob der Inhalt nur aus Leerraum besteht, der ganz zusammenfällt.
    fn is_collapsible_white_space(&self, container: &StyleRef) -> bool {
        self.items.iter().all(|item| match item {
            InlineItem::Text { text, parent } => {
                let style = match parent.and_then(|index| self.boxes.get(index)) {
                    Some(data) => &data.style,
                    None => container,
                };
                style.white_space_collapse() == WhiteSpaceCollapse::Collapse
                    && text
                        .iter()
                        .all(|&code_point| matches!(code_point, 0x20 | 0x09 | 0x0A | 0x0D))
            }
            _ => false,
        })
    }
}

/// Was der Aufbau braucht.
pub(crate) struct Builder<'a> {
    pub(crate) document: &'a Document,
    pub(crate) stylist: &'a Stylist,
    pub(crate) matching: MatchingContext<'a>,
    pub(crate) device: &'a Device,
    pub(crate) fonts: &'a Fonts<'a>,
    /// Die Basis für URLs in Attributen.
    pub(crate) base: &'a Url,
    pub(crate) images: &'a dyn Fn(&Url) -> Option<Arc<Image>>,
    pub(crate) root_style: Option<StyleRef>,
    /// Das erste `body`-Kind eines `html`-Wurzelelements, mit seinem Stil,
    /// sobald er berechnet ist (für den Hintergrund der Leinwand).
    pub(crate) body: Option<NodeId>,
    pub(crate) body_style: Option<StyleRef>,
}

impl<'a> Builder<'a> {
    pub(crate) fn new(
        document: &'a Document,
        stylist: &'a Stylist,
        device: &'a Device,
        fonts: &'a Fonts<'a>,
        base: &'a Url,
        images: &'a dyn Fn(&Url) -> Option<Arc<Image>>,
    ) -> Builder<'a> {
        Builder {
            document,
            stylist,
            matching: MatchingContext::new(document),
            device,
            fonts,
            base,
            images,
            root_style: None,
            body: None,
            body_style: None,
        }
    }

    /// Der Kasten des Wurzelelements, `None` ohne Wurzel oder bei
    /// `display: none`. Er begründet den ersten
    /// Blockformatierungskontext.
    pub(crate) fn build(&mut self) -> Option<BlockBox> {
        let document = self.document;
        let root = document
            .children(document.root())
            .find(|&node| document.kind(node) == Some(NodeKind::Element))?;
        let style = self.element_style(root, None)?;
        self.root_style = Some(style.clone());
        if document
            .element(root)
            .is_some_and(|element| element.is_html("html"))
        {
            self.body = document.children(root).find(|&child| {
                document
                    .element(child)
                    .is_some_and(|element| element.is_html("body"))
            });
        }
        if style.display() == Display::None {
            return None;
        }
        let contents = self.block_container(root, &style, 1);
        Some(BlockBox {
            style,
            node: Some(root),
            contents,
            independent: true,
            first_formatted_line: true,
        })
    }

    /// Der berechnete Stil eines Elements. Fehlt ein Maß der Schrift, für
    /// das es keinen Ersatz gibt, erbt das Element nur; beim Wurzelelement
    /// gibt es dann keinen Stil.
    fn element_style(&self, node: NodeId, parent: Option<&StyleRef>) -> Option<StyleRef> {
        let attribute = self
            .document
            .element(node)
            .and_then(|element| element.attribute("style"))
            .map(|text| {
                let context = ParseContext {
                    base_url: Some(self.base),
                    quirks_mode: self.document.quirks_mode() == mb_dom::QuirksMode::Quirks,
                };
                parse_declaration_block(&Source::new(&text.to_string_lossy()), context)
            });
        let ancestors = match (parent, &self.root_style) {
            (Some(parent), Some(root)) => Some(Ancestors { parent, root }),
            _ => None,
        };
        let computed = ComputedValues::of_element(
            self.stylist,
            &self.matching,
            node,
            attribute.as_deref(),
            ancestors,
            self.device,
            self.fonts,
        );
        match computed {
            Ok(values) => Some(Arc::new(values)),
            Err(_) => parent.map(|parent| self.anonymous_style(parent)),
        }
    }

    /// Der Stil eines anonymen Kastens: geerbte Werte und Anfangswerte.
    pub(crate) fn anonymous_style(&self, parent: &StyleRef) -> StyleRef {
        let matched = MatchedDeclarations::default();
        let cascaded = matched.cascade(|values| flow_mode(values, Some(parent)));
        let root = self.root_style.as_ref().unwrap_or(parent);
        let ancestors = Some(Ancestors { parent, root });
        match ComputedValues::compute(&cascaded, ancestors, self.device, &mb_style::NoFonts) {
            Ok(values) => Arc::new(values),
            // Ohne Deklarationen braucht die Berechnung keine Schriftmaße;
            // geschähe es doch, erbte der Kasten alles.
            Err(_) => parent.clone(),
        }
    }

    /// Der Inhalt eines Blockcontainers.
    fn block_container(&mut self, node: NodeId, style: &StyleRef, depth: usize) -> BlockContents {
        let mut container = Container::new(style.clone());
        self.children(node, style, &mut container, depth);
        container.finish(self)
    }

    fn children(
        &mut self,
        node: NodeId,
        style: &StyleRef,
        container: &mut Container,
        depth: usize,
    ) {
        let document = self.document;
        for child in document.children(node) {
            self.node(child, style, container, depth);
        }
    }

    fn node(&mut self, node: NodeId, parent: &StyleRef, container: &mut Container, depth: usize) {
        let document = self.document;
        match document.kind(node) {
            Some(NodeKind::Text | NodeKind::CdataSection) => {
                if let Some(data) = document.data(node) {
                    container.text(data.code_points().map(|point| point.value()).collect());
                }
            }
            Some(NodeKind::Element) if depth < MAX_DEPTH => {
                self.element(node, parent, container, depth)
            }
            _ => {}
        }
    }

    fn element(
        &mut self,
        node: NodeId,
        parent: &StyleRef,
        container: &mut Container,
        depth: usize,
    ) {
        let Some(element) = self.document.element(node) else {
            return;
        };
        let Some(style) = self.element_style(node, Some(parent)) else {
            return;
        };
        if self.body == Some(node) {
            self.body_style = Some(style.clone());
        }
        let depth = depth.saturating_add(1);
        let (outside, inside) = match style.display() {
            Display::None => return,
            Display::Contents => {
                self.children(node, &style, container, depth);
                return;
            }
            Display::Pair {
                outside, inside, ..
            } => (outside, inside),
            Display::Internal(_) => (DisplayOutside::Block, DisplayInside::Flow),
        };
        let inline = outside == DisplayOutside::Inline;
        if element.is_html("img") {
            let image = element
                .attribute("src")
                .and_then(|src| Url::parse(&src.to_string_lossy(), Some(self.base)).ok())
                .and_then(|url| (self.images)(&url));
            let replaced = ReplacedBox { style, node, image };
            if inline {
                container.atomic(Atomic::Replaced(replaced));
            } else {
                container.block(BlockLevel::Replaced(replaced), self);
            }
            return;
        }
        if inline && element.is_html("br") {
            container.forced_break();
            return;
        }
        match (inline, inside) {
            (true, DisplayInside::Flow | DisplayInside::Ruby) => {
                container.start_inline(style.clone(), Some(node));
                self.children(node, &style, container, depth);
                container.end_inline();
            }
            (true, _) => {
                let contents = self.block_container(node, &style, depth);
                container.atomic(Atomic::Block(BlockBox {
                    style,
                    node: Some(node),
                    contents,
                    independent: true,
                    first_formatted_line: true,
                }));
            }
            (false, inside) => {
                let contents = self.block_container(node, &style, depth);
                container.block(
                    BlockLevel::Block(BlockBox {
                        style,
                        node: Some(node),
                        contents,
                        independent: inside != DisplayInside::Flow,
                        first_formatted_line: true,
                    }),
                    self,
                );
            }
        }
    }
}

/// Sammelt die Kinder eines Blockcontainers.
struct Container {
    style: StyleRef,
    blocks: Vec<BlockLevel>,
    inline: InlineContent,
    /// Die offenen Inline-Kästen, der innerste zuletzt.
    open: Vec<usize>,
}

impl Container {
    fn new(style: StyleRef) -> Container {
        Container {
            style,
            blocks: Vec::new(),
            inline: InlineContent::default(),
            open: Vec::new(),
        }
    }

    fn parent(&self) -> Option<usize> {
        self.open.last().copied()
    }

    fn start_inline(&mut self, style: StyleRef, node: Option<NodeId>) {
        let index = self.inline.boxes.len();
        self.inline.boxes.push(InlineBoxData {
            style,
            node,
            start_edge: true,
            end_edge: true,
            parent: self.parent(),
        });
        self.inline.items.push(InlineItem::Start(index));
        self.open.push(index);
    }

    fn end_inline(&mut self) {
        if let Some(index) = self.open.pop() {
            self.inline.items.push(InlineItem::End(index));
        }
    }

    fn text(&mut self, text: Vec<u32>) {
        if text.is_empty() {
            return;
        }
        let parent = self.parent();
        self.inline.items.push(InlineItem::Text { text, parent });
    }

    fn atomic(&mut self, atomic: Atomic) {
        let parent = self.parent();
        self.inline
            .items
            .push(InlineItem::Atomic { atomic, parent });
    }

    fn forced_break(&mut self) {
        let parent = self.parent();
        self.inline.items.push(InlineItem::ForcedBreak { parent });
    }

    /// Ein blockartiges Kind. Offene Inline-Kästen werden geteilt.
    fn block(&mut self, block: BlockLevel, builder: &Builder<'_>) {
        let open: Vec<(StyleRef, Option<NodeId>)> = self
            .open
            .iter()
            .filter_map(|&index| self.inline.boxes.get(index))
            .map(|data| (data.style.clone(), data.node))
            .collect();
        for &index in self.open.iter().rev() {
            if let Some(data) = self.inline.boxes.get_mut(index) {
                data.end_edge = false;
            }
            self.inline.items.push(InlineItem::End(index));
        }
        self.open.clear();
        self.flush(builder);
        self.blocks.push(block);
        for (style, node) in open {
            self.start_inline(style, node);
            if let Some(data) = self.inline.boxes.last_mut() {
                data.start_edge = false;
            }
        }
    }

    /// Legt den gesammelten Inline-Inhalt in einen anonymen Block.
    fn flush(&mut self, builder: &Builder<'_>) {
        let inline = std::mem::take(&mut self.inline);
        if inline.items.is_empty() || inline.is_collapsible_white_space(&self.style) {
            return;
        }
        let style = builder.anonymous_style(&self.style);
        self.blocks.push(BlockLevel::Block(BlockBox {
            style,
            node: None,
            contents: BlockContents::Inline(inline),
            independent: false,
            first_formatted_line: self.blocks.is_empty(),
        }));
    }

    fn finish(mut self, builder: &Builder<'_>) -> BlockContents {
        while !self.open.is_empty() {
            self.end_inline();
        }
        if self.blocks.is_empty() {
            return BlockContents::Inline(self.inline);
        }
        self.flush(builder);
        BlockContents::Blocks(self.blocks)
    }
}
