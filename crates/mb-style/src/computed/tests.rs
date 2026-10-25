//! Tests für die berechneten Werte: je Regel aus den Spezifikationen ein
//! kleines Dokument mit dem Stylesheet der Benutzeragenten, die Werte so
//! geprüft, wie `getComputedStyle()` sie liefert.

use mb_base::CodePoint;
use mb_css::Source;
use mb_dom::Document;
use mb_html::{Options, ScriptingMode, parse_document};
use mb_url::Url;

use super::*;
use crate::cascade::{Origin, StylistBuilder};
use crate::declarations::parse_declaration_block;
use crate::properties::PropertyId;
use crate::stylesheet::StyleSheet;
use crate::ua::DocumentInfo;
use crate::values::{ParseContext, ToCss};

struct Page {
    document: Document,
    stylist: Stylist,
    base: Url,
}

fn load(html: &str, css: &str) -> Page {
    let mut document = Document::new();
    parse_document(
        &mut document,
        format!("<!DOCTYPE html><body id=body>{html}")
            .chars()
            .map(CodePoint::from),
        Options {
            scripting: ScriptingMode::Disabled,
            collect_errors: false,
        },
    );
    document.perform_microtask_checkpoint();
    let base = Url::parse("https://example.com/a/page.html", None).expect("Basis");
    let mut builder = StylistBuilder::new();
    builder.add_user_agent_sheets(DocumentInfo {
        quirks_mode: document.quirks_mode(),
        html_document: true,
        iso_8859_8: false,
    });
    let context = ParseContext {
        base_url: Some(&base),
        quirks_mode: false,
    };
    builder.add(Origin::Author, &StyleSheet::parse(css, context));
    let stylist = builder.build();
    Page {
        document,
        stylist,
        base,
    }
}

impl Page {
    fn node(&self, id: &str) -> NodeId {
        let document = &self.document;
        let mut current = Some(document.root());
        while let Some(node) = current {
            let element_id = document
                .element(node)
                .and_then(|element| element.attribute("id"));
            if element_id.is_some_and(|value| *value == id) {
                return node;
            }
            current = document.following(node, document.root());
        }
        panic!("kein Element mit der ID {id}");
    }

    /// Die berechneten Werte eines Elements, mit denen seiner Vorfahren
    /// ausgerechnet.
    fn try_computed(
        &self,
        id: &str,
        device: &Device,
        fonts: &dyn FontMetricsSource,
    ) -> Result<ComputedValues, MissingFontMetric> {
        let document = &self.document;
        let mut chain = vec![self.node(id)];
        while let Some(&last) = chain.last()
            && let Some(parent) = document.parent(last)
            && document.element(parent).is_some()
        {
            chain.push(parent);
        }
        chain.reverse();
        let context = MatchingContext::new(document);
        let parse = ParseContext {
            base_url: Some(&self.base),
            quirks_mode: false,
        };
        let mut computed: Vec<ComputedValues> = Vec::new();
        for node in chain {
            let attribute = document
                .element(node)
                .and_then(|element| element.attribute("style"))
                .map(|text| parse_declaration_block(&Source::new(&text.to_string_lossy()), parse));
            let ancestors = match (computed.first(), computed.last()) {
                (Some(root), Some(parent)) => Some(Ancestors { parent, root }),
                _ => None,
            };
            let values = ComputedValues::of_element(
                &self.stylist,
                &context,
                node,
                attribute.as_deref(),
                ancestors,
                device,
                fonts,
            )?;
            computed.push(values);
        }
        computed.pop().ok_or(MissingFontMetric)
    }

    fn computed(&self, id: &str) -> ComputedValues {
        self.try_computed(id, &test_device(), &NoFonts)
            .unwrap_or_else(|_| panic!("Schriftmaß fehlt für #{id}"))
    }

    /// Was `getComputedStyle()` für eine Eigenschaft liefert; „Layout“,
    /// wenn der Wert vom Layout abhängt.
    fn resolved(&self, id: &str, property: &str) -> String {
        resolved(&self.computed(id), property)
    }
}

fn resolved(values: &ComputedValues, property: &str) -> String {
    let result = match PropertyId::from_name(property) {
        Some(PropertyId::Longhand(longhand)) => {
            values.resolved(longhand).map(|value| value.to_css_string())
        }
        Some(PropertyId::Shorthand(shorthand)) => values.resolved_shorthand(shorthand),
        None => panic!("unbekannte Eigenschaft {property}"),
    };
    result.unwrap_or_else(|NeedsLayout| "Layout".to_owned())
}

#[test]
fn font_sizes() {
    let page = load(
        "<div id=a><div id=b><div id=c></div><div id=larger></div><div id=smaller></div></div></div>\
         <div id=keyword></div><div id=math></div>",
        "html { font-size: 2rem }
         #a { font-size: 20px; padding-top: 1rem }
         #b { font-size: 150% }
         #c { font-size: 2em }
         #larger { font-size: larger }
         #smaller { font-size: smaller }
         #keyword { font-size: x-large }
         #math { font-size: math }",
    );
    // `rem` in `font-size` des Wurzelelements meint den Anfangswert, sonst
    // die Schriftgröße des Wurzelelements.
    for (id, expected) in [
        ("a", "20px"),
        ("b", "30px"),
        ("c", "60px"),
        ("larger", "36px"),
        ("smaller", "25px"),
        ("keyword", "24px"),
        ("math", "32px"),
    ] {
        assert_eq!(page.resolved(id, "font-size"), expected, "#{id}");
    }
    assert_eq!(page.resolved("a", "padding-top"), "32px");
    assert_eq!(page.computed("c").font_size(), 60.0);
}

#[test]
fn font_weights() {
    // Die Grenzen der Tabelle in Fonts 4, wie sie auch die WPT prüfen.
    for (inherited, bolder) in [
        (1.0, 400.0),
        (349.0, 400.0),
        (350.0, 700.0),
        (549.0, 700.0),
        (550.0, 900.0),
        (899.0, 900.0),
        (900.0, 900.0),
        (950.0, 950.0),
    ] {
        assert_eq!(
            font_weight(FontWeight::Bolder, inherited),
            bolder,
            "{inherited}"
        );
    }
    for (inherited, lighter) in [
        (99.0, 99.0),
        (100.0, 100.0),
        (549.0, 100.0),
        (550.0, 400.0),
        (749.0, 400.0),
        (750.0, 700.0),
        (1000.0, 700.0),
    ] {
        assert_eq!(
            font_weight(FontWeight::Lighter, inherited),
            lighter,
            "{inherited}"
        );
    }
    let page = load(
        "<div id=a><div id=b></div></div>",
        "#a { font-weight: bold } #b { font-weight: lighter }",
    );
    assert_eq!(page.resolved("a", "font-weight"), "700");
    assert_eq!(page.resolved("b", "font-weight"), "400");
}

#[test]
fn font_style_and_width() {
    let page = load(
        "<div id=a></div><div id=b></div>",
        "#a { font-style: oblique 10grad; font-width: condensed }
         #b { font-style: oblique; font-stretch: 33% }",
    );
    assert_eq!(page.resolved("a", "font-style"), "oblique 9deg");
    assert_eq!(page.resolved("a", "font-width"), "75%");
    assert_eq!(page.resolved("b", "font-style"), "oblique");
    assert_eq!(page.resolved("b", "font-width"), "33%");
    assert_eq!(page.resolved("b", "font-family"), "serif");
}

#[test]
fn line_heights() {
    let page = load(
        "<div id=a><div id=b></div><div id=c></div><div id=d></div></div>",
        "#a { font-size: 20px; line-height: 1.5 }
         #b { line-height: 150% }
         #c { font-size: 10px; line-height: 2em }
         #d { line-height: 2lh }",
    );
    assert_eq!(
        page.computed("a").get(Longhand::LineHeight).to_css_string(),
        "1.5"
    );
    // Eine Zahl wird vererbt, aufgelöst ist sie eine Länge.
    assert_eq!(page.resolved("a", "line-height"), "30px");
    assert_eq!(page.resolved("b", "line-height"), "30px");
    assert_eq!(page.resolved("c", "line-height"), "20px");
    // `lh` in `line-height` bezieht sich auf das Elternelement.
    assert_eq!(page.resolved("d", "line-height"), "60px");
    assert_eq!(load("", "").resolved("body", "line-height"), "normal");
}

/// Schriftmaße als feste Anteile der Schriftgröße.
struct TestFonts;

impl FontMetricsSource for TestFonts {
    fn metrics(&self, font: &Font<'_>) -> FontMetrics {
        let size = font.size;
        FontMetrics {
            x_height: Some(0.4 * size),
            cap_height: Some(0.7 * size),
            ascent: Some(0.9 * size),
            normal_line_height: Some(1.25 * size),
            zero: Some(Advance {
                horizontal: 0.6 * size,
                vertical: 1.1 * size,
            }),
            water: Some(Advance {
                horizontal: size,
                vertical: 0.9 * size,
            }),
        }
    }
}

#[test]
fn font_relative_units() {
    let page = load(
        "<div id=a></div><div id=upright></div><div id=cap></div><div id=lh></div>",
        "div { font-size: 20px }
         #a { padding-top: 2ex; padding-right: 2ch; padding-bottom: 1ic }
         #upright { writing-mode: vertical-rl; text-orientation: upright;
                    padding-top: 1ch; padding-right: 1ic }
         #cap { padding-top: 1cap }
         #lh { padding-top: 1lh }",
    );
    let device = test_device();
    let padding = |id, fonts: &dyn FontMetricsSource| {
        let values = page.try_computed(id, &device, fonts)?;
        Ok(["padding-top", "padding-right", "padding-bottom"]
            .map(|property| resolved(&values, property))
            .join(" "))
    };
    // Ohne Schriften gelten die Ersatzwerte aus Values 4: 0,5em für `ex`
    // und `ch`, aufrecht 1em für `ch`, 1em für `ic`.
    assert_eq!(padding("a", &NoFonts), Ok("20px 20px 20px".to_owned()));
    assert_eq!(padding("upright", &NoFonts), Ok("20px 20px 0px".to_owned()));
    assert_eq!(padding("cap", &NoFonts), Err(MissingFontMetric));
    assert_eq!(padding("lh", &NoFonts), Err(MissingFontMetric));
    assert_eq!(padding("a", &TestFonts), Ok("16px 24px 20px".to_owned()));
    assert_eq!(
        padding("upright", &TestFonts),
        Ok("22px 18px 0px".to_owned())
    );
    assert_eq!(padding("cap", &TestFonts), Ok("14px 0px 0px".to_owned()));
    assert_eq!(padding("lh", &TestFonts), Ok("25px 0px 0px".to_owned()));
}

#[test]
fn viewport_units() {
    let page = load(
        "<div id=a></div><div id=b></div>",
        "#a { padding-top: 10vw; padding-right: 10vh; padding-bottom: 10vi; padding-left: 10vmax }
         #b { writing-mode: vertical-lr; padding-top: 10vi; padding-right: 10svb }",
    );
    assert_eq!(page.resolved("a", "padding"), "80px 60px 80px 80px");
    assert_eq!(page.resolved("b", "padding-top"), "60px");
    assert_eq!(page.resolved("b", "padding-right"), "80px");
}

#[test]
fn border_widths() {
    let page = load(
        "<div id=a></div><div id=b></div><div id=c></div>",
        "#a { border-style: solid; border-width: thin medium thick 0.5px }
         #b { border-top-width: 10px }
         #c { border-style: solid; border-width: 0.25px 1.3px 1px hairline }",
    );
    assert_eq!(page.resolved("a", "border-width"), "1px 3px 5px 1px");
    // Ohne Stil ist der verwendete Wert null, der berechnete bleibt.
    assert_eq!(page.resolved("b", "border-top-width"), "0px");
    assert_eq!(
        page.computed("b")
            .get(Longhand::BorderTopWidth)
            .to_css_string(),
        "10px"
    );
    let device = Device {
        device_pixel_ratio: 2.0,
        ..test_device()
    };
    let values = page.try_computed("c", &device, &NoFonts).expect("#c");
    assert_eq!(resolved(&values, "border-width"), "0.5px 1px 1px 0.5px");
}

#[test]
fn colors() {
    let page = load(
        "<div id=a><div id=b></div><div id=c></div><div id=d></div></div>",
        "#a { color: red; border-top-color: currentcolor; background-color: transparent;
              text-decoration-color: currentcolor }
         #b { color: lime; text-decoration-color: inherit }
         #c { color: Canvas; background-color: ThreeDFace }
         #d { color: currentcolor }",
    );
    assert_eq!(page.resolved("a", "border-top-color"), "rgb(255, 0, 0)");
    assert_eq!(
        page.computed("a")
            .get(Longhand::BorderTopColor)
            .to_css_string(),
        "currentcolor"
    );
    assert_eq!(page.resolved("a", "background-color"), "rgba(0, 0, 0, 0)");
    // Geerbt wird `currentcolor`, aufgelöst mit der eigenen Farbe.
    assert_eq!(
        page.resolved("b", "text-decoration-color"),
        "rgb(0, 255, 0)"
    );
    assert_eq!(page.resolved("c", "color"), "rgb(255, 255, 255)");
    // `ThreeDFace` ist eine ⟨deprecated-color⟩ für `ButtonFace`.
    assert_eq!(page.resolved("c", "background-color"), "rgb(255, 255, 255)");
    assert_eq!(page.resolved("d", "color"), "rgb(255, 0, 0)");
    // Am Anfang `CanvasText`.
    assert_eq!(load("", "").resolved("body", "color"), "rgb(0, 0, 0)");
}

#[test]
fn display_blockification() {
    let page = load(
        "<span id=flex style='display: flex'>\
           <span id=item></span>\
           <span id=contents style='display: contents'>\
             <span id=nested style='display: inline-block'></span>\
           </span>\
           <span id=cell style='display: table-cell'></span>\
         </span>",
        "",
    );
    for (id, expected) in [
        ("flex", "flex"),
        ("item", "block"),
        ("contents", "contents"),
        ("nested", "block"),
        ("cell", "block"),
    ] {
        assert_eq!(page.resolved(id, "display"), expected, "#{id}");
    }
    // `auto` bleibt nur bei Flex- und Grid-Elementen.
    assert_eq!(page.resolved("item", "min-width"), "auto");
    assert_eq!(page.resolved("flex", "min-width"), "0px");
    assert_eq!(page.resolved("contents", "min-height"), "0px");
    for (css, expected) in [
        ("html { display: inline-table }", "table"),
        ("html { display: contents }", "block"),
        ("html { display: inline-block }", "block"),
        ("html { display: none }", "none"),
    ] {
        assert_eq!(resolved_root(&load("", css)), expected, "{css}");
    }
}

#[test]
fn display_with_other_writing_mode() {
    let page = load(
        "<div id=parent>\
           <span id=inline style='writing-mode: vertical-rl'></span>\
           <div id=block style='writing-mode: vertical-rl'></div>\
           <li id=item style='writing-mode: vertical-lr'></li>\
           <span id=flex style='display: flex; writing-mode: vertical-rl'></span>\
           <span id=same></span>\
           <span id=contents style='display: contents; writing-mode: vertical-rl'>\
             <span id=inside></span>\
           </span>\
         </div>",
        "",
    );
    for (id, expected) in [
        ("inline", "inline-block"),
        ("block", "flow-root"),
        ("item", "flow-root list-item"),
        ("flex", "flex"),
        ("same", "inline"),
        ("contents", "contents"),
        // Der Elternkasten ist der von `#parent`.
        ("inside", "inline-block"),
    ] {
        assert_eq!(page.resolved(id, "display"), expected, "#{id}");
    }
}

/// `display` des Wurzelelements.
fn resolved_root(page: &Page) -> String {
    let document = &page.document;
    let html = document
        .first_child(document.root())
        .and_then(|doctype| document.following(doctype, document.root()))
        .filter(|&node| document.element(node).is_some())
        .expect("html");
    let context = MatchingContext::new(document);
    let values = ComputedValues::of_element(
        &page.stylist,
        &context,
        html,
        None,
        None,
        &test_device(),
        &NoFonts,
    )
    .expect("html");
    resolved(&values, "display")
}

#[test]
fn text_align_match_parent() {
    let page = load(
        "<div id=parent style='direction: rtl; text-align: start; text-align-last: end'>\
           <div id=child style='direction: ltr; text-align: match-parent'></div>\
           <div id=last style='text-align-last: match-parent'></div>\
         </div>",
        "",
    );
    assert_eq!(page.resolved("child", "text-align-all"), "right");
    // `text-align: match-parent` setzt beide Langformen auf match-parent.
    assert_eq!(page.resolved("child", "text-align-last"), "left");
    assert_eq!(page.resolved("child", "text-align"), "");
    assert_eq!(page.resolved("last", "text-align-last"), "left");
    let root = load("", "html { text-align: match-parent }");
    assert_eq!(root.resolved("body", "text-align-all"), "start");
}

#[test]
fn spacing() {
    let page = load(
        "<div id=a></div><div id=b></div>",
        "#a { letter-spacing: 0; word-spacing: normal }
         #b { font-size: 10px; letter-spacing: 1em; word-spacing: -5% }",
    );
    assert_eq!(page.resolved("a", "letter-spacing"), "normal");
    assert_eq!(page.resolved("a", "word-spacing"), "0px");
    assert_eq!(page.resolved("b", "letter-spacing"), "10px");
    assert_eq!(page.resolved("b", "word-spacing"), "-5%");
}

#[test]
fn layout_dependent_values() {
    let page = load(
        "<div id=a></div><div id=b></div>",
        "#a { width: 10px; margin-top: 5px; padding-top: 10%; padding-left: 2px; min-height: 20% }
         #b { display: none; width: 10%; margin-top: auto; padding-top: 10% }",
    );
    assert_eq!(page.resolved("a", "width"), "Layout");
    assert_eq!(page.resolved("a", "margin-top"), "Layout");
    assert_eq!(page.resolved("a", "padding-top"), "Layout");
    assert_eq!(page.resolved("a", "padding-left"), "2px");
    assert_eq!(page.resolved("a", "min-height"), "20%");
    assert_eq!(page.resolved("a", "margin"), "Layout");
    // Ohne Kasten der berechnete Wert.
    assert_eq!(page.resolved("b", "width"), "10%");
    assert_eq!(page.resolved("b", "margin-top"), "auto");
    assert_eq!(page.resolved("b", "padding-top"), "10%");
}

#[test]
fn logical_properties() {
    let page = load(
        "<div id=a><div id=b></div></div>",
        "#a { direction: rtl; margin-inline-start: 5px; padding-inline-start: 7px }
         #b { padding-inline-start: inherit; padding-inline-end: 3px }",
    );
    let a = page.computed("a");
    assert_eq!(a.get(Longhand::MarginRight).to_css_string(), "5px");
    assert_eq!(a.get(Longhand::MarginInlineStart).to_css_string(), "5px");
    assert_eq!(page.resolved("a", "padding-right"), "7px");
    // `#b` ist ebenfalls rtl; `inherit` nimmt den logischen Wert.
    assert_eq!(page.resolved("b", "padding-right"), "7px");
    assert_eq!(page.resolved("b", "padding-left"), "3px");
    assert_eq!(page.resolved("b", "padding-inline"), "7px 3px");
}

#[test]
fn shorthands() {
    let page = load(
        "<div id=a></div><div id=b></div><div id=c></div>",
        "#a { color: lime; border: 2px solid currentcolor; text-decoration: underline currentcolor;
              font: italic bold 20px/1.5 serif }
         #b { font: 16px serif }
         #c { font: condensed 16px 'Some Font'; font-variant-caps: unicase }",
    );
    assert_eq!(page.resolved("a", "border-color"), "rgb(0, 255, 0)");
    assert_eq!(page.resolved("a", "border-top"), "2px solid");
    assert_eq!(page.resolved("a", "border"), "2px solid");
    assert_eq!(page.resolved("a", "text-decoration"), "underline");
    assert_eq!(page.resolved("a", "font"), "italic 700 20px / 30px serif");
    assert_eq!(page.resolved("b", "font"), "16px serif");
    // `font` kann `unicase` nicht wiedergeben.
    assert_eq!(page.resolved("c", "font"), "");
    assert_eq!(page.resolved("c", "font-width"), "75%");
    assert_eq!(page.resolved("c", "all"), "");
}

#[test]
fn urls_and_defaulting() {
    let page = load(
        "<div id=a><div id=b></div><div id=c></div></div>",
        "#a { list-style-image: url(img.png); color: red; padding-top: 5px }
         #b { color: initial; display: inherit; padding-top: unset }
         #c { padding-top: inherit }",
    );
    assert_eq!(
        page.resolved("a", "list-style-image"),
        "url(\"https://example.com/a/img.png\")"
    );
    // `list-style-image` wird vererbt, als aufgelöste URL.
    assert_eq!(
        page.resolved("b", "list-style-image"),
        "url(\"https://example.com/a/img.png\")"
    );
    assert_eq!(page.resolved("b", "color"), "rgb(0, 0, 0)");
    assert_eq!(page.resolved("b", "display"), "block");
    assert_eq!(page.resolved("b", "padding-top"), "0px");
    assert_eq!(page.resolved("c", "padding-top"), "5px");
}

#[test]
fn backgrounds() {
    let page = load(
        "<div id=a></div><div id=b></div><div id=c></div>",
        "#a { background: url(img.png) right 10px top / 2em no-repeat, repeat-block lime;
              font-size: 20px }
         #b { writing-mode: vertical-rl; background-repeat: repeat-block, repeat-inline }
         #c { background: url(img.png) center / cover fixed content-box yellow }",
    );
    assert_eq!(
        page.resolved("a", "background-image"),
        "url(\"https://example.com/a/img.png\"), none"
    );
    // Ein Abstand mit Länge von rechts bräuchte `calc()` und behält die Kante.
    assert_eq!(
        page.resolved("a", "background-position-x"),
        "right 10px, 0%"
    );
    assert_eq!(page.resolved("a", "background-position-y"), "0%, 0%");
    assert_eq!(page.resolved("a", "background-size"), "40px auto, auto");
    assert_eq!(page.resolved("a", "background-color"), "rgb(0, 255, 0)");
    // `repeat-block` und `repeat-inline` nach dem Schreibmodus
    assert_eq!(
        page.resolved("a", "background-repeat"),
        "no-repeat, repeat-y"
    );
    assert_eq!(
        page.resolved("b", "background-repeat"),
        "repeat-x, repeat-y"
    );
    assert_eq!(
        page.resolved("c", "background"),
        "url(\"https://example.com/a/img.png\") 50% 50% / cover fixed content-box \
         rgb(255, 255, 0)"
    );
}

#[test]
fn background_position_with_one_edge_offset() {
    // Berechnet behält `right 10px` die Kante, `top` wird 0 %; der
    // Kurzschreibweise fehlt dann nichts.
    let page = load("<div id=a></div>", "#a { background: right 10px top }");
    assert_eq!(
        page.resolved("a", "background-position"),
        "right 10px top 0%"
    );
}

#[test]
fn outlines() {
    let page = load(
        "<div id=a></div><div id=b></div>",
        "#a { outline: auto; outline-offset: 0.5em; font-size: 20px }
         #b { outline: 2.5px solid; color: lime }",
    );
    // `auto` bleibt nur bei `outline-style: auto` stehen.
    assert_eq!(page.resolved("a", "outline-color"), "auto");
    assert_eq!(page.resolved("a", "outline-offset"), "10px");
    assert_eq!(page.resolved("b", "outline-color"), "rgb(0, 255, 0)");
    assert_eq!(page.resolved("b", "outline-width"), "2px");
    assert_eq!(page.resolved("b", "outline"), "2px solid rgb(0, 255, 0)");
}

#[test]
fn dimension_attributes() {
    // `display: none`, damit `width` und `height` ohne Layout berechnet
    // werden.
    let page = load(
        "<img id=a width=' 50.5%x' height=20><img id=b width=30 height=0>\
         <div id=c width=10></div><svg><image id=d width=10 /></svg>",
        "img, div, image { display: none } #b { width: 40px }",
    );
    assert_eq!(page.resolved("a", "width"), "50.5%");
    assert_eq!(page.resolved("a", "height"), "20px");
    // Autorenregeln gewinnen über die Hinweise.
    assert_eq!(page.resolved("b", "width"), "40px");
    assert_eq!(page.resolved("b", "height"), "0px");
    assert_eq!(page.resolved("c", "width"), "auto");
    assert_eq!(page.resolved("d", "width"), "auto");
}
