//! Fremdinhalt (HTML Standard, „The rules for parsing tokens in foreign
//! content“) und die Anpassungen der Namen von SVG- und MathML-Tags und
//! -Attributen aus „Creating and inserting nodes“.

use mb_base::{CodePoint, DomString};
use mb_dom::{self as dom, Namespace};

use super::{Flow, TreeBuilder, is_whitespace};
use crate::tokenizer::{Attribute, Tag, Token};

/// „Adjust SVG attributes“: Name auf dem Token, Name am Element.
const SVG_ATTRIBUTES: [(&str, &str); 58] = [
    ("attributename", "attributeName"),
    ("attributetype", "attributeType"),
    ("basefrequency", "baseFrequency"),
    ("baseprofile", "baseProfile"),
    ("calcmode", "calcMode"),
    ("clippathunits", "clipPathUnits"),
    ("diffuseconstant", "diffuseConstant"),
    ("edgemode", "edgeMode"),
    ("filterunits", "filterUnits"),
    ("glyphref", "glyphRef"),
    ("gradienttransform", "gradientTransform"),
    ("gradientunits", "gradientUnits"),
    ("kernelmatrix", "kernelMatrix"),
    ("kernelunitlength", "kernelUnitLength"),
    ("keypoints", "keyPoints"),
    ("keysplines", "keySplines"),
    ("keytimes", "keyTimes"),
    ("lengthadjust", "lengthAdjust"),
    ("limitingconeangle", "limitingConeAngle"),
    ("markerheight", "markerHeight"),
    ("markerunits", "markerUnits"),
    ("markerwidth", "markerWidth"),
    ("maskcontentunits", "maskContentUnits"),
    ("maskunits", "maskUnits"),
    ("numoctaves", "numOctaves"),
    ("pathlength", "pathLength"),
    ("patterncontentunits", "patternContentUnits"),
    ("patterntransform", "patternTransform"),
    ("patternunits", "patternUnits"),
    ("pointsatx", "pointsAtX"),
    ("pointsaty", "pointsAtY"),
    ("pointsatz", "pointsAtZ"),
    ("preservealpha", "preserveAlpha"),
    ("preserveaspectratio", "preserveAspectRatio"),
    ("primitiveunits", "primitiveUnits"),
    ("refx", "refX"),
    ("refy", "refY"),
    ("repeatcount", "repeatCount"),
    ("repeatdur", "repeatDur"),
    ("requiredextensions", "requiredExtensions"),
    ("requiredfeatures", "requiredFeatures"),
    ("specularconstant", "specularConstant"),
    ("specularexponent", "specularExponent"),
    ("spreadmethod", "spreadMethod"),
    ("startoffset", "startOffset"),
    ("stddeviation", "stdDeviation"),
    ("stitchtiles", "stitchTiles"),
    ("surfacescale", "surfaceScale"),
    ("systemlanguage", "systemLanguage"),
    ("tablevalues", "tableValues"),
    ("targetx", "targetX"),
    ("targety", "targetY"),
    ("textlength", "textLength"),
    ("viewbox", "viewBox"),
    ("viewtarget", "viewTarget"),
    ("xchannelselector", "xChannelSelector"),
    ("ychannelselector", "yChannelSelector"),
    ("zoomandpan", "zoomAndPan"),
];

/// Die Tag-Namen, die SVG in gemischter Schreibung führt.
const SVG_TAG_NAMES: [(&str, &str); 37] = [
    ("altglyph", "altGlyph"),
    ("altglyphdef", "altGlyphDef"),
    ("altglyphitem", "altGlyphItem"),
    ("animatecolor", "animateColor"),
    ("animatemotion", "animateMotion"),
    ("animatetransform", "animateTransform"),
    ("clippath", "clipPath"),
    ("feblend", "feBlend"),
    ("fecolormatrix", "feColorMatrix"),
    ("fecomponenttransfer", "feComponentTransfer"),
    ("fecomposite", "feComposite"),
    ("feconvolvematrix", "feConvolveMatrix"),
    ("fediffuselighting", "feDiffuseLighting"),
    ("fedisplacementmap", "feDisplacementMap"),
    ("fedistantlight", "feDistantLight"),
    ("fedropshadow", "feDropShadow"),
    ("feflood", "feFlood"),
    ("fefunca", "feFuncA"),
    ("fefuncb", "feFuncB"),
    ("fefuncg", "feFuncG"),
    ("fefuncr", "feFuncR"),
    ("fegaussianblur", "feGaussianBlur"),
    ("feimage", "feImage"),
    ("femerge", "feMerge"),
    ("femergenode", "feMergeNode"),
    ("femorphology", "feMorphology"),
    ("feoffset", "feOffset"),
    ("fepointlight", "fePointLight"),
    ("fespecularlighting", "feSpecularLighting"),
    ("fespotlight", "feSpotLight"),
    ("fetile", "feTile"),
    ("feturbulence", "feTurbulence"),
    ("foreignobject", "foreignObject"),
    ("glyphref", "glyphRef"),
    ("lineargradient", "linearGradient"),
    ("radialgradient", "radialGradient"),
    ("textpath", "textPath"),
];

/// „Adjust foreign attributes“: Name auf dem Token, Präfix, lokaler Name,
/// Namensraum.
const FOREIGN_ATTRIBUTES: [(&str, Option<&str>, &str, Namespace); 11] = [
    ("xlink:actuate", Some("xlink"), "actuate", Namespace::XLink),
    ("xlink:arcrole", Some("xlink"), "arcrole", Namespace::XLink),
    ("xlink:href", Some("xlink"), "href", Namespace::XLink),
    ("xlink:role", Some("xlink"), "role", Namespace::XLink),
    ("xlink:show", Some("xlink"), "show", Namespace::XLink),
    ("xlink:title", Some("xlink"), "title", Namespace::XLink),
    ("xlink:type", Some("xlink"), "type", Namespace::XLink),
    ("xml:lang", Some("xml"), "lang", Namespace::Xml),
    ("xml:space", Some("xml"), "space", Namespace::Xml),
    ("xmlns", None, "xmlns", Namespace::Xmlns),
    ("xmlns:xlink", Some("xmlns"), "xlink", Namespace::Xmlns),
];

/// Die Start-Tags, mit denen Fremdinhalt endet (dazu `font` mit `color`,
/// `face` oder `size`).
const BREAKOUT: [&str; 44] = [
    "b",
    "big",
    "blockquote",
    "body",
    "br",
    "center",
    "code",
    "dd",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "hr",
    "i",
    "img",
    "li",
    "listing",
    "menu",
    "meta",
    "nobr",
    "ol",
    "p",
    "pre",
    "ruby",
    "s",
    "small",
    "span",
    "strong",
    "strike",
    "sub",
    "sup",
    "table",
    "tt",
    "u",
    "ul",
    "var",
];

/// Benennt Attribute des Tokens nach einer Tabelle um.
fn rename_attributes(tag: &mut Tag, table: &[(&str, &str)]) {
    for attribute in &mut tag.attributes {
        if let Some(name) = attribute.name.as_ascii()
            && let Some((_, renamed)) = table.iter().find(|(from, _)| *from == name)
        {
            attribute.name = DomString::from(*renamed);
        }
    }
}

/// „Adjust MathML attributes“
pub(super) fn adjust_mathml_attributes(tag: &mut Tag) {
    rename_attributes(tag, &[("definitionurl", "definitionURL")]);
}

/// „Adjust SVG attributes“
pub(super) fn adjust_svg_attributes(tag: &mut Tag) {
    rename_attributes(tag, &SVG_ATTRIBUTES);
}

/// Die Tabelle der SVG-Tag-Namen aus „any other start tag“.
fn adjust_svg_tag_name(tag: &mut Tag) {
    if let Some(name) = tag.name.as_ascii()
        && let Some((_, renamed)) = SVG_TAG_NAMES.iter().find(|(from, _)| *from == name)
    {
        tag.name = DomString::from(*renamed);
    }
}

/// „Adjust foreign attributes“ für ein Attribut, zugleich die Umwandlung
/// in ein Attribut des Dokuments.
pub(super) fn adjust_foreign_attribute(attribute: &Attribute) -> dom::Attribute {
    let adjusted = attribute
        .name
        .as_ascii()
        .and_then(|name| FOREIGN_ATTRIBUTES.iter().find(|(from, ..)| *from == name));
    match adjusted {
        Some(&(_, prefix, local_name, namespace)) => dom::Attribute {
            namespace: Some(namespace),
            prefix: prefix.map(DomString::from),
            local_name: DomString::from(local_name),
            value: attribute.value.clone(),
        },
        None => dom::Attribute::new(attribute.name.clone(), attribute.value.clone()),
    }
}

fn is_breakout(tag: &Tag) -> bool {
    let name = tag.name.as_ascii().unwrap_or("");
    BREAKOUT.contains(&name)
        || (name == "font"
            && tag.attributes.iter().any(|attribute| {
                ["color", "face", "size"]
                    .iter()
                    .any(|x| attribute.name == *x)
            }))
}

impl TreeBuilder<'_> {
    /// „The rules for parsing tokens in foreign content“
    pub(super) fn foreign_content(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(c) if c.value() == 0 => {
                self.parse_error();
                self.insert_character(CodePoint::REPLACEMENT);
            }
            Token::Character(c) => {
                self.insert_character(c);
                if !is_whitespace(c) {
                    self.frameset_ok = false;
                }
            }
            Token::Comment(data) => self.insert_comment(data, None),
            Token::ProcessingInstruction { target, data } => {
                self.insert_processing_instruction(target, data, None);
            }
            Token::Doctype(_) => self.parse_error(),
            Token::StartTag(ref tag) if is_breakout(tag) => return self.break_out(token),
            Token::EndTag(ref tag) if tag.name == "br" || tag.name == "p" => {
                return self.break_out(token);
            }
            Token::StartTag(tag) => self.foreign_start_tag(tag),
            Token::EndTag(tag) => {
                if tag.name == "script"
                    && self
                        .current_node()
                        .is_some_and(|node| self.is_element(node, Namespace::Svg, "script"))
                {
                    // Ohne SVG-Skripte bleibt nur das Entfernen vom Stapel.
                    self.open.pop();
                } else {
                    return self.foreign_end_tag(tag);
                }
            }
            // Das Dateiende geht immer nach den Regeln für HTML-Inhalt.
            Token::Eof => return self.html_content(Token::Eof),
        }
        Flow::Done
    }

    /// Verlässt den Fremdinhalt und verarbeitet das Token nach dem aktuellen
    /// Einfügemodus für HTML-Inhalt.
    fn break_out(&mut self, token: Token) -> Flow {
        self.parse_error();
        while let Some(node) = self.current_node()
            && !self.is_mathml_text_integration_point(node)
            && !self.is_html_integration_point(node)
            && !self.is_html_element(node)
        {
            self.open.pop();
        }
        self.html_content(token)
    }

    /// „Any other start tag“ im Fremdinhalt.
    fn foreign_start_tag(&mut self, mut tag: Tag) {
        let Some(namespace) = self
            .adjusted_current_node()
            .and_then(|node| self.element_name(node))
            .map(|(namespace, _)| namespace)
        else {
            return;
        };
        if namespace == Namespace::MathMl {
            adjust_mathml_attributes(&mut tag);
        }
        if namespace == Namespace::Svg {
            adjust_svg_tag_name(&mut tag);
            adjust_svg_attributes(&mut tag);
        }
        self.insert_foreign_element(&tag, namespace, true);
        if tag.self_closing {
            // Ob `script` im SVG-Namensraum (dann wie `</script>`) oder
            // nicht: Beides nimmt den Knoten ohne Skriptanbindung nur vom
            // Stapel.
            self.acknowledge_self_closing();
            self.open.pop();
        }
    }

    /// „Any other end tag“ im Fremdinhalt.
    fn foreign_end_tag(&mut self, tag: Tag) -> Flow {
        let Some(mut index) = self.open.len().checked_sub(1) else {
            return Flow::Done;
        };
        let lowercase_matches = |builder: &Self, node| {
            builder
                .document
                .element(node)
                .is_some_and(|element| element.local_name.to_ascii_lowercase() == tag.name)
        };
        if let Some(&node) = self.open.get(index)
            && !lowercase_matches(self, node)
        {
            self.parse_error();
        }
        loop {
            let Some(&node) = self.open.get(index) else {
                return Flow::Done;
            };
            if index == 0 {
                return Flow::Done;
            }
            if lowercase_matches(self, node) {
                self.open.truncate(index);
                return Flow::Done;
            }
            index = index.saturating_sub(1);
            let Some(&above) = self.open.get(index) else {
                return Flow::Done;
            };
            if self.is_html_element(above) {
                return self.html_content(Token::EndTag(tag));
            }
        }
    }
}
