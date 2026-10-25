//! Die Einfügemodi (HTML Standard, „The rules for parsing tokens in HTML
//! content“), in der Reihenfolge des Standards. Innerhalb eines Modus stehen
//! die Fälle in der Reihenfolge, in der der Standard sie aufzählt, soweit
//! Rust es erlaubt; Fälle mit Bedingung stehen vor den allgemeinen.

use mb_base::DomString;
use mb_dom::{self as dom, Namespace, NodeId, QuirksMode};

use super::{Flow, Formatting, Location, Mode, Scope, ScriptingMode, TreeBuilder, is_whitespace};
use crate::tokenizer::{Doctype, Tag, Token, TokenizerState};

/// Öffentliche Kennungen, die genau so den Quirks-Modus auslösen.
const QUIRKS_PUBLIC_IDS: [&str; 3] = [
    "-//W3O//DTD W3 HTML Strict 3.0//EN//",
    "-/W3C/DTD HTML 4.0 Transitional/EN",
    "HTML",
];

const QUIRKS_SYSTEM_ID: &str = "http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd";

/// Anfänge öffentlicher Kennungen für den Quirks-Modus.
const QUIRKS_PUBLIC_PREFIXES: [&str; 55] = [
    "+//Silmaril//dtd html Pro v0r11 19970101//",
    "-//AS//DTD HTML 3.0 asWedit + extensions//",
    "-//AdvaSoft Ltd//DTD HTML 3.0 asWedit + extensions//",
    "-//IETF//DTD HTML 2.0 Level 1//",
    "-//IETF//DTD HTML 2.0 Level 2//",
    "-//IETF//DTD HTML 2.0 Strict Level 1//",
    "-//IETF//DTD HTML 2.0 Strict Level 2//",
    "-//IETF//DTD HTML 2.0 Strict//",
    "-//IETF//DTD HTML 2.0//",
    "-//IETF//DTD HTML 2.1E//",
    "-//IETF//DTD HTML 3.0//",
    "-//IETF//DTD HTML 3.2 Final//",
    "-//IETF//DTD HTML 3.2//",
    "-//IETF//DTD HTML 3//",
    "-//IETF//DTD HTML Level 0//",
    "-//IETF//DTD HTML Level 1//",
    "-//IETF//DTD HTML Level 2//",
    "-//IETF//DTD HTML Level 3//",
    "-//IETF//DTD HTML Strict Level 0//",
    "-//IETF//DTD HTML Strict Level 1//",
    "-//IETF//DTD HTML Strict Level 2//",
    "-//IETF//DTD HTML Strict Level 3//",
    "-//IETF//DTD HTML Strict//",
    "-//IETF//DTD HTML//",
    "-//Metrius//DTD Metrius Presentational//",
    "-//Microsoft//DTD Internet Explorer 2.0 HTML Strict//",
    "-//Microsoft//DTD Internet Explorer 2.0 HTML//",
    "-//Microsoft//DTD Internet Explorer 2.0 Tables//",
    "-//Microsoft//DTD Internet Explorer 3.0 HTML Strict//",
    "-//Microsoft//DTD Internet Explorer 3.0 HTML//",
    "-//Microsoft//DTD Internet Explorer 3.0 Tables//",
    "-//Netscape Comm. Corp.//DTD HTML//",
    "-//Netscape Comm. Corp.//DTD Strict HTML//",
    "-//O'Reilly and Associates//DTD HTML 2.0//",
    "-//O'Reilly and Associates//DTD HTML Extended 1.0//",
    "-//O'Reilly and Associates//DTD HTML Extended Relaxed 1.0//",
    "-//SQ//DTD HTML 2.0 HoTMetaL + extensions//",
    "-//SoftQuad Software//DTD HoTMetaL PRO 6.0::19990601::extensions to HTML 4.0//",
    "-//SoftQuad//DTD HoTMetaL PRO 4.0::19971010::extensions to HTML 4.0//",
    "-//Spyglass//DTD HTML 2.0 Extended//",
    "-//Sun Microsystems Corp.//DTD HotJava HTML//",
    "-//Sun Microsystems Corp.//DTD HotJava Strict HTML//",
    "-//W3C//DTD HTML 3 1995-03-24//",
    "-//W3C//DTD HTML 3.2 Draft//",
    "-//W3C//DTD HTML 3.2 Final//",
    "-//W3C//DTD HTML 3.2//",
    "-//W3C//DTD HTML 3.2S Draft//",
    "-//W3C//DTD HTML 4.0 Frameset//",
    "-//W3C//DTD HTML 4.0 Transitional//",
    "-//W3C//DTD HTML Experimental 19960712//",
    "-//W3C//DTD HTML Experimental 970421//",
    "-//W3C//DTD W3 HTML//",
    "-//W3O//DTD W3 HTML 3.0//",
    "-//WebTechs//DTD Mozilla HTML 2.0//",
    "-//WebTechs//DTD Mozilla HTML//",
];

/// HTML 4.01 Frameset und Transitional: ohne Systemkennung Quirks, mit
/// Systemkennung eingeschränkte Quirks.
const HTML401_PUBLIC_PREFIXES: [&str; 2] = [
    "-//W3C//DTD HTML 4.01 Frameset//",
    "-//W3C//DTD HTML 4.01 Transitional//",
];

const LIMITED_QUIRKS_PUBLIC_PREFIXES: [&str; 2] = [
    "-//W3C//DTD XHTML 1.0 Frameset//",
    "-//W3C//DTD XHTML 1.0 Transitional//",
];

/// Die Start-Tags, die „in body“ einen offenen Absatz schließen und dann
/// ein Element einfügen.
const BLOCKS: [&str; 25] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "header",
    "hgroup",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "search",
    "section",
    "summary",
    "ul",
];

/// Die End-Tags, die „in body“ ein Element desselben Namens im Bereich
/// schließen.
const BLOCK_ENDS: [&str; 28] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "button",
    "center",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "header",
    "hgroup",
    "listing",
    "main",
    "menu",
    "nav",
    "ol",
    "pre",
    "search",
    "section",
    "select",
    "summary",
    "ul",
];

const HEADINGS: [&str; 6] = ["h1", "h2", "h3", "h4", "h5", "h6"];

/// Die Formatierungselemente außer `a` und `nobr`, die eigene Regeln haben.
const FORMATTING: [&str; 12] = [
    "b", "big", "code", "em", "font", "i", "s", "small", "strike", "strong", "tt", "u",
];

/// Die Start-Tags, die „in body“ nach den Regeln von „in head“ laufen.
const HEAD_ELEMENTS: [&str; 10] = [
    "base", "basefont", "bgsound", "link", "meta", "noframes", "script", "style", "template",
    "title",
];

/// Der Name eines Tags als ASCII; andere Namen kommen in keiner Regel vor.
fn name(tag: &Tag) -> &str {
    tag.name.as_ascii().unwrap_or("")
}

fn is_one_of(tag: &Tag, names: &[&str]) -> bool {
    names.contains(&name(tag))
}

/// `type=hidden` auf einem `input`.
fn is_hidden_input(tag: &Tag) -> bool {
    tag.attribute("type")
        .is_some_and(|value| value.eq_ignore_ascii_case("hidden"))
}

/// Der Modus, in den ein DOCTYPE das Dokument versetzt (Modus „initial“).
fn doctype_mode(doctype: &Doctype) -> QuirksMode {
    let public = doctype.public_id.as_ref();
    let system = doctype.system_id.as_ref();
    let public_is =
        |ids: &[&str]| public.is_some_and(|id| ids.iter().any(|x| id.eq_ignore_ascii_case(x)));
    let public_starts = |prefixes: &[&str]| {
        public.is_some_and(|id| prefixes.iter().any(|x| id.starts_with_ignore_ascii_case(x)))
    };
    let system_missing_or_empty = system.is_none_or(DomString::is_empty);
    if doctype.force_quirks
        || !doctype.name.as_ref().is_some_and(|name| *name == "html")
        || public_is(&QUIRKS_PUBLIC_IDS)
        || system.is_some_and(|id| id.eq_ignore_ascii_case(QUIRKS_SYSTEM_ID))
        || public_starts(&QUIRKS_PUBLIC_PREFIXES)
        || (system_missing_or_empty && public_starts(&HTML401_PUBLIC_PREFIXES))
    {
        QuirksMode::Quirks
    } else if public_starts(&LIMITED_QUIRKS_PUBLIC_PREFIXES)
        || (!system_missing_or_empty && public_starts(&HTML401_PUBLIC_PREFIXES))
    {
        QuirksMode::LimitedQuirks
    } else {
        QuirksMode::NoQuirks
    }
}

impl TreeBuilder<'_> {
    /// Der Modus „initial“.
    pub(super) fn initial(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(c) if is_whitespace(c) => {}
            Token::Comment(data) => {
                let location = self.document_location();
                self.insert_comment(data, Some(location));
            }
            Token::ProcessingInstruction { target, data } => {
                let location = self.document_location();
                self.insert_processing_instruction(target, data, Some(location));
            }
            Token::Doctype(doctype) => {
                let name_is_html = doctype.name.as_ref().is_some_and(|name| *name == "html");
                if !name_is_html
                    || doctype.public_id.is_some()
                    || doctype
                        .system_id
                        .as_ref()
                        .is_some_and(|id| *id != "about:legacy-compat")
                {
                    self.parse_error();
                }
                let root = self.document.root();
                let node = self.document.create_doctype(dom::Doctype {
                    name: doctype.name.clone().unwrap_or_default(),
                    public_id: doctype.public_id.clone().unwrap_or_default(),
                    system_id: doctype.system_id.clone().unwrap_or_default(),
                });
                let free = !self.document.children(root).any(|child| {
                    matches!(
                        self.document.kind(child),
                        Some(dom::NodeKind::DocumentType | dom::NodeKind::Element)
                    )
                });
                if free {
                    let appended = self.document.append(root, node);
                    debug_assert!(appended.is_ok(), "Einfügen nach dem Standard gültig");
                }
                match doctype_mode(&doctype) {
                    QuirksMode::NoQuirks => {}
                    mode => self.document.set_quirks_mode(mode),
                }
                self.mode = Mode::BeforeHtml;
            }
            token => {
                self.parse_error();
                self.document.set_quirks_mode(QuirksMode::Quirks);
                self.mode = Mode::BeforeHtml;
                return Flow::Reprocess(token);
            }
        }
        Flow::Done
    }

    /// Der Modus „before html“.
    pub(super) fn before_html(&mut self, token: Token) -> Flow {
        match token {
            Token::Doctype(_) => self.parse_error(),
            Token::Comment(data) => {
                let location = self.document_location();
                self.insert_comment(data, Some(location));
            }
            Token::ProcessingInstruction { target, data } => {
                let location = self.document_location();
                self.insert_processing_instruction(target, data, Some(location));
            }
            Token::Character(c) if is_whitespace(c) => {}
            Token::StartTag(tag) if name(&tag) == "html" => {
                let element = self.create_element_for_token(&tag, Namespace::Html, false);
                let location = self.document_location();
                self.insert_element_at(element, Some(location));
                self.open.push(element);
                self.mode = Mode::BeforeHead;
            }
            Token::EndTag(tag) if !is_one_of(&tag, &["head", "body", "html", "br"]) => {
                self.parse_error();
            }
            token => {
                let element = self.document.create_element(
                    Some(Namespace::Html),
                    DomString::from("html"),
                    Vec::new(),
                );
                let location = self.document_location();
                self.insert_element_at(element, Some(location));
                self.open.push(element);
                self.mode = Mode::BeforeHead;
                return Flow::Reprocess(token);
            }
        }
        Flow::Done
    }

    /// Der Modus „before head“.
    pub(super) fn before_head(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(c) if is_whitespace(c) => {}
            Token::Comment(data) => self.insert_comment(data, None),
            Token::ProcessingInstruction { target, data } => {
                self.insert_processing_instruction(target, data, None);
            }
            Token::Doctype(_) => self.parse_error(),
            Token::StartTag(tag) if name(&tag) == "html" => {
                return self.in_body(Token::StartTag(tag));
            }
            Token::StartTag(tag) if name(&tag) == "head" => {
                let head = self.insert_html_element(&tag);
                self.head = Some(head);
                self.mode = Mode::InHead;
            }
            Token::EndTag(tag) if !is_one_of(&tag, &["head", "body", "html", "br"]) => {
                self.parse_error();
            }
            token => {
                let head = self.insert_html_element(&Tag::named("head"));
                self.head = Some(head);
                self.mode = Mode::InHead;
                return Flow::Reprocess(token);
            }
        }
        Flow::Done
    }

    /// Der Modus „in head“.
    pub(super) fn in_head(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(c) if is_whitespace(c) => self.insert_character(c),
            Token::Comment(data) => self.insert_comment(data, None),
            Token::ProcessingInstruction { target, data } => {
                self.insert_processing_instruction(target, data, None);
            }
            Token::Doctype(_) => self.parse_error(),
            Token::StartTag(tag) => match name(&tag) {
                "html" => return self.in_body(Token::StartTag(tag)),
                "base" | "basefont" | "bgsound" | "link" => {
                    self.insert_html_element(&tag);
                    self.pop();
                    self.acknowledge_self_closing();
                }
                "meta" => {
                    // Die Eingabe ist schon dekodiert; einen Wechsel der
                    // Kodierung durch `charset` gibt es noch nicht.
                    self.insert_html_element(&tag);
                    self.pop();
                    self.acknowledge_self_closing();
                }
                "title" => self.parse_text_element(&tag, TokenizerState::Rcdata),
                "noscript" if self.scripting != ScriptingMode::Disabled => {
                    self.parse_text_element(&tag, TokenizerState::Rawtext);
                }
                "noframes" | "style" => self.parse_text_element(&tag, TokenizerState::Rawtext),
                "noscript" => {
                    self.insert_html_element(&tag);
                    self.mode = Mode::InHeadNoscript;
                }
                "script" => {
                    // *parser document*, *force async* und *already started*
                    // kommen mit der Skriptanbindung.
                    let element = self.create_element_for_token(&tag, Namespace::Html, false);
                    self.insert_element_at(element, None);
                    self.open.push(element);
                    self.tokenizer.set_state(TokenizerState::ScriptData);
                    self.original_mode = self.mode;
                    self.mode = Mode::Text;
                }
                "template" => self.template_start_tag(&tag),
                "head" => self.parse_error(),
                _ => return self.in_head_anything_else(Token::StartTag(tag)),
            },
            Token::EndTag(tag) => match name(&tag) {
                "head" => {
                    self.pop();
                    self.mode = Mode::AfterHead;
                }
                "body" | "html" | "br" => return self.in_head_anything_else(Token::EndTag(tag)),
                "template" => self.template_end_tag(),
                _ => self.parse_error(),
            },
            token => return self.in_head_anything_else(token),
        }
        Flow::Done
    }

    fn in_head_anything_else(&mut self, token: Token) -> Flow {
        self.pop();
        self.mode = Mode::AfterHead;
        Flow::Reprocess(token)
    }

    /// „A start tag whose tag name is "template"“ in „in head“. Ohne
    /// deklarative Shadow Roots und ohne Patchen bleibt vom Fall „Keep“ nur
    /// das Einfügen an der vorher bestimmten Stelle.
    fn template_start_tag(&mut self, tag: &Tag) {
        self.formatting.push(Formatting::Marker);
        self.frameset_ok = false;
        self.mode = Mode::InTemplate;
        self.template_modes.push(Mode::InTemplate);
        let template = self.create_element_for_token(tag, Namespace::Html, false);
        let location = self.appropriate_place(None);
        self.open.push(template);
        if let Some(location) = location {
            self.insert_element_at(template, Some(location));
        }
    }

    /// „An end tag whose tag name is "template"“ in „in head“.
    fn template_end_tag(&mut self) {
        if !self.has_open_html("template") {
            self.parse_error();
            return;
        }
        self.generate_implied_end_tags_thoroughly();
        if !self.current_is_html("template") {
            self.parse_error();
        }
        self.pop_until_html("template");
        self.clear_formatting_to_marker();
        self.template_modes.pop();
        self.reset_insertion_mode();
    }

    /// Der Modus „in head noscript“.
    pub(super) fn in_head_noscript(&mut self, token: Token) -> Flow {
        match token {
            Token::Doctype(_) => self.parse_error(),
            Token::StartTag(tag) if name(&tag) == "html" => {
                return self.in_body(Token::StartTag(tag));
            }
            Token::EndTag(tag) if name(&tag) == "noscript" => {
                self.pop();
                self.mode = Mode::InHead;
            }
            Token::Character(c) if is_whitespace(c) => return self.in_head(token),
            Token::Comment(_) | Token::ProcessingInstruction { .. } => return self.in_head(token),
            Token::StartTag(ref tag)
                if is_one_of(
                    tag,
                    &["basefont", "bgsound", "link", "meta", "noframes", "style"],
                ) =>
            {
                return self.in_head(token);
            }
            Token::EndTag(ref tag) if name(tag) == "br" => {
                return self.in_head_noscript_anything_else(token);
            }
            Token::StartTag(ref tag) if is_one_of(tag, &["head", "noscript"]) => self.parse_error(),
            Token::EndTag(_) => self.parse_error(),
            token => return self.in_head_noscript_anything_else(token),
        }
        Flow::Done
    }

    fn in_head_noscript_anything_else(&mut self, token: Token) -> Flow {
        self.parse_error();
        self.pop();
        self.mode = Mode::InHead;
        Flow::Reprocess(token)
    }

    /// Der Modus „after head“.
    pub(super) fn after_head(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(c) if is_whitespace(c) => self.insert_character(c),
            Token::Comment(data) => self.insert_comment(data, None),
            Token::ProcessingInstruction { target, data } => {
                self.insert_processing_instruction(target, data, None);
            }
            Token::Doctype(_) => self.parse_error(),
            Token::StartTag(tag) => match name(&tag) {
                "html" => return self.in_body(Token::StartTag(tag)),
                "body" => {
                    self.insert_html_element(&tag);
                    self.frameset_ok = false;
                    self.mode = Mode::InBody;
                }
                "frameset" => {
                    self.insert_html_element(&tag);
                    self.mode = Mode::InFrameset;
                }
                "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script"
                | "style" | "template" | "title" => {
                    self.parse_error();
                    let Some(head) = self.head else {
                        return Flow::Done;
                    };
                    self.open.push(head);
                    let flow = self.in_head(Token::StartTag(tag));
                    self.remove_from_stack(head);
                    return flow;
                }
                "head" => self.parse_error(),
                _ => return self.after_head_anything_else(Token::StartTag(tag)),
            },
            Token::EndTag(tag) => match name(&tag) {
                "template" => return self.in_head(Token::EndTag(tag)),
                "body" | "html" | "br" => {
                    return self.after_head_anything_else(Token::EndTag(tag));
                }
                _ => self.parse_error(),
            },
            token => return self.after_head_anything_else(token),
        }
        Flow::Done
    }

    fn after_head_anything_else(&mut self, token: Token) -> Flow {
        self.insert_html_element(&Tag::named("body"));
        self.frameset_ok = true;
        self.mode = Mode::InBody;
        Flow::Reprocess(token)
    }

    /// Der Modus „in body“.
    pub(super) fn in_body(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(c) if c.value() == 0 => self.parse_error(),
            Token::Character(c) => {
                self.reconstruct_formatting();
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
            Token::StartTag(tag) => return self.in_body_start_tag(tag),
            Token::EndTag(tag) => return self.in_body_end_tag(tag),
            Token::Eof => {
                if !self.template_modes.is_empty() {
                    return self.in_template(Token::Eof);
                }
                if self.has_unclosed_elements() {
                    self.parse_error();
                }
                self.stop_parsing();
            }
        }
        Flow::Done
    }

    /// Fügt die Attribute des Tokens hinzu, die `node` noch nicht hat (für
    /// `html` und `body`).
    fn add_missing_attributes(&mut self, node: NodeId, tag: &Tag) {
        for attribute in &tag.attributes {
            let present = self.document.element(node).is_some_and(|element| {
                element
                    .attributes
                    .iter()
                    .any(|existing| qualified_name_is(existing, &attribute.name))
            });
            if !present {
                self.document.append_attribute(
                    node,
                    dom::Attribute::new(attribute.name.clone(), attribute.value.clone()),
                );
            }
        }
    }

    fn in_body_start_tag(&mut self, mut tag: Tag) -> Flow {
        match name(&tag) {
            "html" => {
                self.parse_error();
                if !self.has_open_html("template")
                    && let Some(&html) = self.open.first()
                {
                    self.add_missing_attributes(html, &tag);
                }
            }
            name if HEAD_ELEMENTS.contains(&name) => return self.in_head(Token::StartTag(tag)),
            "body" => {
                self.parse_error();
                let body = self.open.get(1).copied();
                if let Some(body) = body
                    && self.is_html(body, "body")
                    && !self.has_open_html("template")
                {
                    self.frameset_ok = false;
                    self.add_missing_attributes(body, &tag);
                }
            }
            "frameset" => {
                self.parse_error();
                let Some(body) = self.open.get(1).copied() else {
                    return Flow::Done;
                };
                if !self.is_html(body, "body") || !self.frameset_ok {
                    return Flow::Done;
                }
                self.document.remove(body);
                self.pop_to(1);
                self.insert_html_element(&tag);
                self.mode = Mode::InFrameset;
            }
            name if BLOCKS.contains(&name) => {
                self.close_p_in_button_scope();
                self.insert_html_element(&tag);
            }
            name if HEADINGS.contains(&name) => {
                self.close_p_in_button_scope();
                if self
                    .current_node()
                    .and_then(|node| self.html_name(node))
                    .is_some_and(|current| HEADINGS.contains(&current))
                {
                    self.parse_error();
                    self.pop();
                }
                self.insert_html_element(&tag);
            }
            "pre" | "listing" => {
                self.close_p_in_button_scope();
                self.insert_html_element(&tag);
                self.skip_line_feed = true;
                self.frameset_ok = false;
            }
            "form" => {
                let in_template = self.parsing_template_contents();
                if self.form.is_some() && !in_template {
                    self.parse_error();
                    return Flow::Done;
                }
                self.close_p_in_button_scope();
                let form = self.insert_html_element(&tag);
                if !in_template {
                    self.form = Some(form);
                }
            }
            "li" => {
                self.frameset_ok = false;
                self.close_list_item(&["li"]);
                self.close_p_in_button_scope();
                self.insert_html_element(&tag);
            }
            "dd" | "dt" => {
                self.frameset_ok = false;
                self.close_list_item(&["dd", "dt"]);
                self.close_p_in_button_scope();
                self.insert_html_element(&tag);
            }
            "plaintext" => {
                self.close_p_in_button_scope();
                self.insert_html_element(&tag);
                self.tokenizer.set_state(TokenizerState::Plaintext);
            }
            "button" => {
                if self.has_in_scope("button", Scope::Default) {
                    self.parse_error();
                    self.generate_implied_end_tags(None);
                    self.pop_until_html("button");
                }
                self.reconstruct_formatting();
                self.insert_html_element(&tag);
                self.frameset_ok = false;
            }
            "a" => {
                let start = self.formatting_start();
                let existing =
                    self.formatting
                        .iter()
                        .skip(start)
                        .rev()
                        .find_map(|entry| match entry {
                            Formatting::Element { node, .. } if self.is_html(*node, "a") => {
                                Some(*node)
                            }
                            _ => None,
                        });
                if let Some(existing) = existing {
                    self.parse_error();
                    self.adoption_agency(&tag.name);
                    if let Some(index) = self.formatting_index(existing) {
                        self.formatting.remove(index);
                    }
                    self.remove_from_stack(existing);
                }
                self.reconstruct_formatting();
                let element = self.insert_html_element(&tag);
                self.push_formatting(element, tag);
            }
            name if FORMATTING.contains(&name) => {
                self.reconstruct_formatting();
                let element = self.insert_html_element(&tag);
                self.push_formatting(element, tag);
            }
            "nobr" => {
                self.reconstruct_formatting();
                if self.has_in_scope("nobr", Scope::Default) {
                    self.parse_error();
                    self.adoption_agency(&tag.name);
                    self.reconstruct_formatting();
                }
                let element = self.insert_html_element(&tag);
                self.push_formatting(element, tag);
            }
            "applet" | "marquee" | "object" => {
                self.reconstruct_formatting();
                self.insert_html_element(&tag);
                self.formatting.push(Formatting::Marker);
                self.frameset_ok = false;
            }
            "table" => {
                if self.document.quirks_mode() != QuirksMode::Quirks {
                    self.close_p_in_button_scope();
                }
                self.insert_html_element(&tag);
                self.frameset_ok = false;
                self.mode = Mode::InTable;
            }
            "area" | "br" | "embed" | "img" | "keygen" | "wbr" => self.void_element(&tag),
            "input" => {
                if self.context_is_html("select") {
                    self.parse_error();
                    return Flow::Done;
                }
                if self.has_in_scope("select", Scope::Default) {
                    self.parse_error();
                    self.pop_until_html("select");
                }
                self.reconstruct_formatting();
                self.insert_html_element(&tag);
                self.pop();
                self.acknowledge_self_closing();
                if !is_hidden_input(&tag) {
                    self.frameset_ok = false;
                }
            }
            "param" | "source" | "track" => {
                self.insert_html_element(&tag);
                self.pop();
                self.acknowledge_self_closing();
            }
            "hr" => {
                self.close_p_in_button_scope();
                if self.has_in_scope("select", Scope::Default) {
                    self.generate_implied_end_tags(None);
                    if self.has_in_scope("option", Scope::Default)
                        || self.has_in_scope("optgroup", Scope::Default)
                    {
                        self.parse_error();
                    }
                }
                self.insert_html_element(&tag);
                self.pop();
                self.acknowledge_self_closing();
                self.frameset_ok = false;
            }
            "image" => {
                self.parse_error();
                tag.name = DomString::from("img");
                return Flow::Reprocess(Token::StartTag(tag));
            }
            "textarea" => {
                self.insert_html_element(&tag);
                self.skip_line_feed = true;
                self.tokenizer.set_state(TokenizerState::Rcdata);
                self.original_mode = self.mode;
                self.frameset_ok = false;
                self.mode = Mode::Text;
            }
            "xmp" => {
                self.close_p_in_button_scope();
                self.reconstruct_formatting();
                self.frameset_ok = false;
                self.parse_text_element(&tag, TokenizerState::Rawtext);
            }
            "iframe" => {
                self.frameset_ok = false;
                self.parse_text_element(&tag, TokenizerState::Rawtext);
            }
            "noembed" => self.parse_text_element(&tag, TokenizerState::Rawtext),
            "noscript" if self.scripting != ScriptingMode::Disabled => {
                self.parse_text_element(&tag, TokenizerState::Rawtext);
            }
            "select" => {
                if self.context_is_html("select") {
                    self.parse_error();
                } else if self.has_in_scope("select", Scope::Default) {
                    self.parse_error();
                    self.pop_until_html("select");
                } else {
                    self.reconstruct_formatting();
                    self.insert_html_element(&tag);
                    self.frameset_ok = false;
                }
            }
            "option" => {
                if self.has_in_scope("select", Scope::Default) {
                    self.generate_implied_end_tags(Some("optgroup"));
                    if self.has_in_scope("option", Scope::Default) {
                        self.parse_error();
                    }
                } else if self.current_is_html("option") {
                    self.pop();
                }
                self.reconstruct_formatting();
                self.insert_html_element(&tag);
            }
            "optgroup" => {
                if self.has_in_scope("select", Scope::Default) {
                    self.generate_implied_end_tags(None);
                    if self.has_in_scope("option", Scope::Default)
                        || self.has_in_scope("optgroup", Scope::Default)
                    {
                        self.parse_error();
                    }
                } else if self.current_is_html("option") {
                    self.pop();
                }
                self.reconstruct_formatting();
                self.insert_html_element(&tag);
            }
            "rb" | "rtc" => {
                if self.has_in_scope("ruby", Scope::Default) {
                    self.generate_implied_end_tags(None);
                }
                if !self.current_is_html("ruby") {
                    self.parse_error();
                }
                self.insert_html_element(&tag);
            }
            "rp" | "rt" => {
                if self.has_in_scope("ruby", Scope::Default) {
                    self.generate_implied_end_tags(Some("rtc"));
                }
                if !self.current_is_html("rtc") && !self.current_is_html("ruby") {
                    self.parse_error();
                }
                self.insert_html_element(&tag);
            }
            "math" => {
                self.reconstruct_formatting();
                super::foreign::adjust_mathml_attributes(&mut tag);
                self.insert_foreign_element(&tag, Namespace::MathMl, true);
                if tag.self_closing {
                    self.pop();
                    self.acknowledge_self_closing();
                }
            }
            "svg" => {
                self.reconstruct_formatting();
                super::foreign::adjust_svg_attributes(&mut tag);
                self.insert_foreign_element(&tag, Namespace::Svg, true);
                if tag.self_closing {
                    self.pop();
                    self.acknowledge_self_closing();
                }
            }
            "caption" | "col" | "colgroup" | "frame" | "head" | "tbody" | "td" | "tfoot" | "th"
            | "thead" | "tr" => self.parse_error(),
            _ => {
                self.reconstruct_formatting();
                self.insert_html_element(&tag);
            }
        }
        Flow::Done
    }

    /// Die Schleife der Start-Tags `li`, `dd` und `dt`: schließt das
    /// nächste offene Element mit einem der Namen, falls kein besonderes
    /// Element (außer `address`, `div`, `p`) dazwischen steht.
    fn close_list_item(&mut self, names: &[&str]) {
        let mut index = self.open.len();
        while let Some(previous) = index.checked_sub(1) {
            index = previous;
            let Some(&node) = self.open.get(index) else {
                return;
            };
            if let Some(name) = self.html_name(node)
                && names.contains(&name)
            {
                let name = name.to_owned();
                self.generate_implied_end_tags(Some(&name));
                if !self.current_is_html(&name) {
                    self.parse_error();
                }
                self.pop_until_html(&name);
                return;
            }
            if self.is_special(node)
                && !matches!(self.html_name(node), Some("address" | "div" | "p"))
            {
                return;
            }
        }
    }

    /// `area`, `br`, `embed`, `img`, `keygen` und `wbr`.
    fn void_element(&mut self, tag: &Tag) {
        self.reconstruct_formatting();
        self.insert_html_element(tag);
        self.pop();
        self.acknowledge_self_closing();
        self.frameset_ok = false;
    }

    /// Ein End-Tag, das ein HTML-Element desselben Namens im Bereich
    /// schließt; `false`, wenn keins im Bereich war.
    fn close_named_element(&mut self, tag: &Tag) -> bool {
        if !self.has_named_in_scope(&tag.name, Scope::Default) {
            self.parse_error();
            return false;
        }
        self.generate_implied_end_tags(None);
        if !self
            .current_node()
            .is_some_and(|node| self.is_html_named(node, &tag.name))
        {
            self.parse_error();
        }
        self.pop_until_named(&tag.name);
        true
    }

    fn in_body_end_tag(&mut self, tag: Tag) -> Flow {
        match name(&tag) {
            "template" => return self.in_head(Token::EndTag(tag)),
            "body" | "html" => {
                if !self.has_in_scope("body", Scope::Default) {
                    self.parse_error();
                    return Flow::Done;
                }
                if self.has_unclosed_elements() {
                    self.parse_error();
                }
                self.mode = Mode::AfterBody;
                if name(&tag) == "html" {
                    return Flow::Reprocess(Token::EndTag(tag));
                }
            }
            name if BLOCK_ENDS.contains(&name) => {
                self.close_named_element(&tag);
            }
            "form" => {
                if !self.parsing_template_contents() {
                    let node = self.form.take();
                    let Some(node) =
                        node.filter(|&node| self.in_scope(Scope::Default, |open| open == node))
                    else {
                        self.parse_error();
                        return Flow::Done;
                    };
                    self.generate_implied_end_tags(None);
                    if self.current_node() != Some(node) {
                        self.parse_error();
                    }
                    self.remove_from_stack(node);
                } else {
                    if !self.has_in_scope("form", Scope::Default) {
                        self.parse_error();
                        return Flow::Done;
                    }
                    self.generate_implied_end_tags(None);
                    if !self.current_is_html("form") {
                        self.parse_error();
                    }
                    self.pop_until_html("form");
                }
            }
            "p" => {
                if !self.has_in_scope("p", Scope::Button) {
                    self.parse_error();
                    self.insert_html_element(&Tag::named("p"));
                }
                self.close_p_element();
            }
            "li" => {
                if !self.has_in_scope("li", Scope::ListItem) {
                    self.parse_error();
                    return Flow::Done;
                }
                self.generate_implied_end_tags(Some("li"));
                if !self.current_is_html("li") {
                    self.parse_error();
                }
                self.pop_until_html("li");
            }
            name @ ("dd" | "dt") => {
                if !self.has_in_scope(name, Scope::Default) {
                    self.parse_error();
                    return Flow::Done;
                }
                self.generate_implied_end_tags(Some(name));
                if !self.current_is_html(name) {
                    self.parse_error();
                }
                self.pop_until_html(name);
            }
            name if HEADINGS.contains(&name) => {
                let is_heading = |builder: &Self, node| {
                    builder
                        .html_name(node)
                        .is_some_and(|name| HEADINGS.contains(&name))
                };
                if !self.in_scope(Scope::Default, |node| is_heading(self, node)) {
                    self.parse_error();
                    return Flow::Done;
                }
                self.generate_implied_end_tags(None);
                if !self.current_is_html(name) {
                    self.parse_error();
                }
                self.pop_until(is_heading);
            }
            name if name == "a" || name == "nobr" || FORMATTING.contains(&name) => {
                self.adoption_agency(&tag.name);
            }
            "applet" | "marquee" | "object" => {
                if self.close_named_element(&tag) {
                    self.clear_formatting_to_marker();
                }
            }
            "br" => {
                self.parse_error();
                self.void_element(&Tag::named("br"));
            }
            _ => self.any_other_end_tag(&tag.name),
        }
        Flow::Done
    }

    /// Der Modus „text“.
    pub(super) fn text(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(c) => self.insert_character(c),
            Token::Eof => {
                self.parse_error();
                self.pop();
                self.mode = self.original_mode;
                return Flow::Reprocess(Token::Eof);
            }
            // Ohne Skriptanbindung sind `</script>` und jedes andere
            // End-Tag gleich.
            Token::EndTag(_) => {
                self.pop();
                self.mode = self.original_mode;
            }
            // Im RCDATA-, RAWTEXT- und Skriptzustand liefert der Tokenizer
            // nur Zeichen, End-Tags und das Dateiende.
            _ => {}
        }
        Flow::Done
    }

    fn current_is_table_like(&self) -> bool {
        self.current_node().is_some_and(|node| {
            matches!(
                self.html_name(node),
                Some("table" | "tbody" | "template" | "tfoot" | "thead" | "tr")
            )
        })
    }

    /// „Clear the stack back to a … context“: nimmt Elemente vom Stapel,
    /// bis eines der genannten HTML-Elemente der aktuelle Knoten ist.
    fn clear_stack_back_to(&mut self, names: &[&str]) {
        while let Some(node) = self.current_node()
            && !self
                .html_name(node)
                .is_some_and(|name| names.contains(&name))
        {
            self.pop();
        }
    }

    fn clear_to_table_context(&mut self) {
        self.clear_stack_back_to(&["table", "template", "html"]);
    }

    fn clear_to_table_body_context(&mut self) {
        self.clear_stack_back_to(&["tbody", "tfoot", "thead", "template", "html"]);
    }

    fn clear_to_table_row_context(&mut self) {
        self.clear_stack_back_to(&["tr", "template", "html"]);
    }

    /// Der Modus „in table“.
    pub(super) fn in_table(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(_) if self.current_is_table_like() => {
                self.pending_table_characters.clear();
                self.original_mode = self.mode;
                self.mode = Mode::InTableText;
                return Flow::Reprocess(token);
            }
            Token::Comment(data) => self.insert_comment(data, None),
            Token::ProcessingInstruction { target, data } => {
                self.insert_processing_instruction(target, data, None);
            }
            Token::Doctype(_) => self.parse_error(),
            Token::StartTag(tag) => match name(&tag) {
                "caption" => {
                    self.clear_to_table_context();
                    self.formatting.push(Formatting::Marker);
                    self.insert_html_element(&tag);
                    self.mode = Mode::InCaption;
                }
                "colgroup" => {
                    self.clear_to_table_context();
                    self.insert_html_element(&tag);
                    self.mode = Mode::InColumnGroup;
                }
                "col" => {
                    self.clear_to_table_context();
                    self.insert_html_element(&Tag::named("colgroup"));
                    self.mode = Mode::InColumnGroup;
                    return Flow::Reprocess(Token::StartTag(tag));
                }
                "tbody" | "tfoot" | "thead" => {
                    self.clear_to_table_context();
                    self.insert_html_element(&tag);
                    self.mode = Mode::InTableBody;
                }
                "td" | "th" | "tr" => {
                    self.clear_to_table_context();
                    self.insert_html_element(&Tag::named("tbody"));
                    self.mode = Mode::InTableBody;
                    return Flow::Reprocess(Token::StartTag(tag));
                }
                "table" => {
                    self.parse_error();
                    if self.has_in_scope("table", Scope::Table) {
                        self.pop_until_html("table");
                        self.reset_insertion_mode();
                        return Flow::Reprocess(Token::StartTag(tag));
                    }
                }
                "style" | "script" | "template" => return self.in_head(Token::StartTag(tag)),
                "input" if is_hidden_input(&tag) => {
                    self.parse_error();
                    self.insert_html_element(&tag);
                    self.pop();
                    self.acknowledge_self_closing();
                }
                "form" => {
                    self.parse_error();
                    let in_template = self.parsing_template_contents();
                    if self.form.is_some() && !in_template {
                        return Flow::Done;
                    }
                    let form = self.insert_html_element(&tag);
                    if !in_template {
                        self.form = Some(form);
                    }
                    self.pop();
                }
                _ => return self.in_table_anything_else(Token::StartTag(tag)),
            },
            Token::EndTag(tag) => match name(&tag) {
                "table" => {
                    if !self.has_in_scope("table", Scope::Table) {
                        self.parse_error();
                        return Flow::Done;
                    }
                    self.pop_until_html("table");
                    self.reset_insertion_mode();
                }
                "body" | "caption" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot"
                | "th" | "thead" | "tr" => self.parse_error(),
                "template" => return self.in_head(Token::EndTag(tag)),
                _ => return self.in_table_anything_else(Token::EndTag(tag)),
            },
            Token::Eof => return self.in_body(Token::Eof),
            token => return self.in_table_anything_else(token),
        }
        Flow::Done
    }

    /// „Anything else“ in „in table“: Pflegeeltern (*foster parenting*).
    fn in_table_anything_else(&mut self, token: Token) -> Flow {
        self.parse_error();
        self.foster_parenting = true;
        let flow = self.in_body(token);
        self.foster_parenting = false;
        flow
    }

    /// Der Modus „in table text“.
    pub(super) fn in_table_text(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(c) if c.value() == 0 => self.parse_error(),
            Token::Character(c) => self.pending_table_characters.push(c),
            token => {
                let pending = std::mem::take(&mut self.pending_table_characters);
                if pending.iter().any(|&c| !is_whitespace(c)) {
                    // Der Parse-Fehler entsteht mit jedem Zeichen in „anything
                    // else“ von „in table“; so zählen ihn auch die Tests.
                    for c in pending {
                        self.in_table_anything_else(Token::Character(c));
                    }
                } else {
                    for c in pending {
                        self.insert_character(c);
                    }
                }
                self.mode = self.original_mode;
                return Flow::Reprocess(token);
            }
        }
        Flow::Done
    }

    /// Schließt `caption` für „in caption“; `false`, wenn keins im
    /// Tabellenbereich war.
    fn close_caption(&mut self) -> bool {
        if !self.has_in_scope("caption", Scope::Table) {
            self.parse_error();
            return false;
        }
        self.generate_implied_end_tags(None);
        if !self.current_is_html("caption") {
            self.parse_error();
        }
        self.pop_until_html("caption");
        self.clear_formatting_to_marker();
        self.mode = Mode::InTable;
        true
    }

    /// Der Modus „in caption“.
    pub(super) fn in_caption(&mut self, token: Token) -> Flow {
        match token {
            Token::EndTag(ref tag) if name(tag) == "caption" => {
                self.close_caption();
            }
            Token::StartTag(ref tag)
                if is_one_of(
                    tag,
                    &[
                        "caption", "col", "colgroup", "tbody", "td", "tfoot", "th", "thead", "tr",
                    ],
                ) =>
            {
                if self.close_caption() {
                    return Flow::Reprocess(token);
                }
            }
            Token::EndTag(ref tag) if name(tag) == "table" => {
                if self.close_caption() {
                    return Flow::Reprocess(token);
                }
            }
            Token::EndTag(ref tag)
                if is_one_of(
                    tag,
                    &[
                        "body", "col", "colgroup", "html", "tbody", "td", "tfoot", "th", "thead",
                        "tr",
                    ],
                ) =>
            {
                self.parse_error();
            }
            token => return self.in_body(token),
        }
        Flow::Done
    }

    /// Der Modus „in column group“.
    pub(super) fn in_column_group(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(c) if is_whitespace(c) => self.insert_character(c),
            Token::Comment(data) => self.insert_comment(data, None),
            Token::ProcessingInstruction { target, data } => {
                self.insert_processing_instruction(target, data, None);
            }
            Token::Doctype(_) => self.parse_error(),
            Token::StartTag(ref tag) if name(tag) == "html" => return self.in_body(token),
            Token::StartTag(ref tag) if name(tag) == "col" => {
                self.insert_html_element(tag);
                self.pop();
                self.acknowledge_self_closing();
            }
            Token::EndTag(ref tag) if name(tag) == "colgroup" => {
                if !self.current_is_html("colgroup") {
                    self.parse_error();
                } else {
                    self.pop();
                    self.mode = Mode::InTable;
                }
            }
            Token::EndTag(ref tag) if name(tag) == "col" => self.parse_error(),
            Token::StartTag(ref tag) | Token::EndTag(ref tag) if name(tag) == "template" => {
                return self.in_head(token);
            }
            Token::Eof => return self.in_body(token),
            token => {
                if !self.current_is_html("colgroup") {
                    self.parse_error();
                    return Flow::Done;
                }
                self.pop();
                self.mode = Mode::InTable;
                return Flow::Reprocess(token);
            }
        }
        Flow::Done
    }

    /// Der Modus „in table body“.
    pub(super) fn in_table_body(&mut self, token: Token) -> Flow {
        match token {
            Token::StartTag(ref tag) if name(tag) == "tr" => {
                self.clear_to_table_body_context();
                self.insert_html_element(tag);
                self.mode = Mode::InRow;
            }
            Token::StartTag(ref tag) if is_one_of(tag, &["th", "td"]) => {
                self.parse_error();
                self.clear_to_table_body_context();
                self.insert_html_element(&Tag::named("tr"));
                self.mode = Mode::InRow;
                return Flow::Reprocess(token);
            }
            Token::EndTag(ref tag) if is_one_of(tag, &["tbody", "tfoot", "thead"]) => {
                if !self.has_named_in_scope(&tag.name, Scope::Table) {
                    self.parse_error();
                    return Flow::Done;
                }
                self.clear_to_table_body_context();
                self.pop();
                self.mode = Mode::InTable;
            }
            Token::StartTag(ref tag)
                if is_one_of(
                    tag,
                    &["caption", "col", "colgroup", "tbody", "tfoot", "thead"],
                ) =>
            {
                return self.close_table_body(token);
            }
            Token::EndTag(ref tag) if name(tag) == "table" => return self.close_table_body(token),
            Token::EndTag(ref tag)
                if is_one_of(
                    tag,
                    &[
                        "body", "caption", "col", "colgroup", "html", "td", "th", "tr",
                    ],
                ) =>
            {
                self.parse_error();
            }
            token => return self.in_table(token),
        }
        Flow::Done
    }

    /// Schließt `tbody`, `thead` oder `tfoot` und verarbeitet das Token neu.
    fn close_table_body(&mut self, token: Token) -> Flow {
        let in_scope = self.in_scope(Scope::Table, |node| {
            matches!(self.html_name(node), Some("tbody" | "thead" | "tfoot"))
        });
        if !in_scope {
            self.parse_error();
            return Flow::Done;
        }
        self.clear_to_table_body_context();
        self.pop();
        self.mode = Mode::InTable;
        Flow::Reprocess(token)
    }

    /// Der Modus „in row“.
    pub(super) fn in_row(&mut self, token: Token) -> Flow {
        match token {
            Token::StartTag(ref tag) if is_one_of(tag, &["th", "td"]) => {
                self.clear_to_table_row_context();
                self.insert_html_element(tag);
                self.mode = Mode::InCell;
                self.formatting.push(Formatting::Marker);
            }
            Token::EndTag(ref tag) if name(tag) == "tr" => {
                self.close_row();
            }
            Token::StartTag(ref tag)
                if is_one_of(
                    tag,
                    &[
                        "caption", "col", "colgroup", "tbody", "tfoot", "thead", "tr",
                    ],
                ) =>
            {
                if self.close_row() {
                    return Flow::Reprocess(token);
                }
            }
            Token::EndTag(ref tag) if name(tag) == "table" => {
                if self.close_row() {
                    return Flow::Reprocess(token);
                }
            }
            Token::EndTag(ref tag) if is_one_of(tag, &["tbody", "tfoot", "thead"]) => {
                if !self.has_named_in_scope(&tag.name, Scope::Table) {
                    self.parse_error();
                    return Flow::Done;
                }
                if !self.has_in_scope("tr", Scope::Table) {
                    return Flow::Done;
                }
                self.clear_to_table_row_context();
                self.pop();
                self.mode = Mode::InTableBody;
                return Flow::Reprocess(token);
            }
            Token::EndTag(ref tag)
                if is_one_of(
                    tag,
                    &["body", "caption", "col", "colgroup", "html", "td", "th"],
                ) =>
            {
                self.parse_error();
            }
            token => return self.in_table(token),
        }
        Flow::Done
    }

    /// Schließt `tr`; `false`, wenn keins im Tabellenbereich war.
    fn close_row(&mut self) -> bool {
        if !self.has_in_scope("tr", Scope::Table) {
            self.parse_error();
            return false;
        }
        self.clear_to_table_row_context();
        self.pop();
        self.mode = Mode::InTableBody;
        true
    }

    /// „Close the cell“
    fn close_cell(&mut self) {
        self.generate_implied_end_tags(None);
        if !self.current_is_html("td") && !self.current_is_html("th") {
            self.parse_error();
        }
        self.pop_until(|builder, node| matches!(builder.html_name(node), Some("td" | "th")));
        self.clear_formatting_to_marker();
        self.mode = Mode::InRow;
    }

    /// Der Modus „in cell“.
    pub(super) fn in_cell(&mut self, token: Token) -> Flow {
        match token {
            Token::EndTag(ref tag) if is_one_of(tag, &["td", "th"]) => {
                if !self.has_named_in_scope(&tag.name, Scope::Table) {
                    self.parse_error();
                    return Flow::Done;
                }
                self.generate_implied_end_tags(None);
                if !self
                    .current_node()
                    .is_some_and(|node| self.is_html_named(node, &tag.name))
                {
                    self.parse_error();
                }
                self.pop_until_named(&tag.name);
                self.clear_formatting_to_marker();
                self.mode = Mode::InRow;
            }
            Token::StartTag(ref tag)
                if is_one_of(
                    tag,
                    &[
                        "caption", "col", "colgroup", "tbody", "td", "tfoot", "th", "thead", "tr",
                    ],
                ) =>
            {
                debug_assert!(
                    self.in_scope(Scope::Table, |node| matches!(
                        self.html_name(node),
                        Some("td" | "th")
                    )),
                    "„in cell“ ohne Zelle im Tabellenbereich"
                );
                self.close_cell();
                return Flow::Reprocess(token);
            }
            Token::EndTag(ref tag)
                if is_one_of(tag, &["body", "caption", "col", "colgroup", "html"]) =>
            {
                self.parse_error();
            }
            Token::EndTag(ref tag)
                if is_one_of(tag, &["table", "tbody", "tfoot", "thead", "tr"]) =>
            {
                if !self.has_named_in_scope(&tag.name, Scope::Table) {
                    self.parse_error();
                    return Flow::Done;
                }
                self.close_cell();
                return Flow::Reprocess(token);
            }
            token => return self.in_body(token),
        }
        Flow::Done
    }

    /// Ersetzt den aktuellen Template-Einfügemodus und schaltet in ihn um.
    fn switch_template_mode(&mut self, mode: Mode) {
        self.template_modes.pop();
        self.template_modes.push(mode);
        self.mode = mode;
    }

    /// Der Modus „in template“.
    pub(super) fn in_template(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(_)
            | Token::Comment(_)
            | Token::ProcessingInstruction { .. }
            | Token::Doctype(_) => return self.in_body(token),
            Token::StartTag(ref tag) if HEAD_ELEMENTS.contains(&name(tag)) => {
                return self.in_head(token);
            }
            Token::EndTag(ref tag) if name(tag) == "template" => return self.in_head(token),
            Token::StartTag(ref tag) => {
                let mode = match name(tag) {
                    "caption" | "colgroup" | "tbody" | "tfoot" | "thead" => Mode::InTable,
                    "col" => Mode::InColumnGroup,
                    "tr" => Mode::InTableBody,
                    "td" | "th" => Mode::InRow,
                    _ => Mode::InBody,
                };
                self.switch_template_mode(mode);
                return Flow::Reprocess(token);
            }
            Token::EndTag(_) => self.parse_error(),
            Token::Eof => {
                if !self.has_open_html("template") {
                    self.stop_parsing();
                    return Flow::Done;
                }
                self.parse_error();
                self.pop_until_html("template");
                self.clear_formatting_to_marker();
                self.template_modes.pop();
                self.reset_insertion_mode();
                return Flow::Reprocess(Token::Eof);
            }
        }
        Flow::Done
    }

    /// Der erste Eintrag im Stapel als Einfügestelle (das Wurzelelement
    /// `html`).
    fn html_location(&self) -> Option<Location> {
        self.open.first().map(|&html| Location {
            parent: html,
            before: None,
        })
    }

    /// Der Modus „after body“.
    pub(super) fn after_body(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(c) if is_whitespace(c) => return self.in_body(token),
            Token::Comment(data) => {
                let location = self.html_location();
                self.insert_comment(data, location);
            }
            Token::ProcessingInstruction { target, data } => {
                let location = self.html_location();
                self.insert_processing_instruction(target, data, location);
            }
            Token::Doctype(_) => self.parse_error(),
            Token::StartTag(ref tag) if name(tag) == "html" => return self.in_body(token),
            Token::EndTag(ref tag) if name(tag) == "html" => {
                if self.context.is_some() {
                    self.parse_error();
                } else {
                    self.mode = Mode::AfterAfterBody;
                }
            }
            Token::Eof => self.stop_parsing(),
            token => {
                self.parse_error();
                self.mode = Mode::InBody;
                return Flow::Reprocess(token);
            }
        }
        Flow::Done
    }

    /// Der Modus „in frameset“.
    pub(super) fn in_frameset(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(c) if is_whitespace(c) => self.insert_character(c),
            Token::Comment(data) => self.insert_comment(data, None),
            Token::ProcessingInstruction { target, data } => {
                self.insert_processing_instruction(target, data, None);
            }
            Token::Doctype(_) => self.parse_error(),
            Token::StartTag(ref tag) if name(tag) == "html" => return self.in_body(token),
            Token::StartTag(ref tag) if name(tag) == "frameset" => {
                self.insert_html_element(tag);
            }
            Token::EndTag(ref tag) if name(tag) == "frameset" => {
                if self.current_is_root() {
                    self.parse_error();
                    return Flow::Done;
                }
                self.pop();
                if self.context.is_none() && !self.current_is_html("frameset") {
                    self.mode = Mode::AfterFrameset;
                }
            }
            Token::StartTag(ref tag) if name(tag) == "frame" => {
                self.insert_html_element(tag);
                self.pop();
                self.acknowledge_self_closing();
            }
            Token::StartTag(ref tag) if name(tag) == "noframes" => return self.in_head(token),
            Token::Eof => {
                if !self.current_is_root() {
                    self.parse_error();
                }
                self.stop_parsing();
            }
            _ => self.parse_error(),
        }
        Flow::Done
    }

    /// Ob der aktuelle Knoten das Wurzelelement `html` ist.
    fn current_is_root(&self) -> bool {
        self.open.len() == 1 && self.current_is_html("html")
    }

    /// Der Modus „after frameset“.
    pub(super) fn after_frameset(&mut self, token: Token) -> Flow {
        match token {
            Token::Character(c) if is_whitespace(c) => self.insert_character(c),
            Token::Comment(data) => self.insert_comment(data, None),
            Token::ProcessingInstruction { target, data } => {
                self.insert_processing_instruction(target, data, None);
            }
            Token::Doctype(_) => self.parse_error(),
            Token::StartTag(ref tag) if name(tag) == "html" => return self.in_body(token),
            Token::EndTag(ref tag) if name(tag) == "html" => {
                self.mode = Mode::AfterAfterFrameset;
            }
            Token::StartTag(ref tag) if name(tag) == "noframes" => return self.in_head(token),
            Token::Eof => self.stop_parsing(),
            _ => self.parse_error(),
        }
        Flow::Done
    }

    /// Der Modus „after after body“.
    pub(super) fn after_after_body(&mut self, token: Token) -> Flow {
        match token {
            Token::Comment(data) => {
                let location = self.document_location();
                self.insert_comment(data, Some(location));
            }
            Token::ProcessingInstruction { target, data } => {
                let location = self.document_location();
                self.insert_processing_instruction(target, data, Some(location));
            }
            Token::Doctype(_) => return self.in_body(token),
            Token::Character(c) if is_whitespace(c) => return self.in_body(token),
            Token::StartTag(ref tag) if name(tag) == "html" => return self.in_body(token),
            Token::Eof => self.stop_parsing(),
            token => {
                self.parse_error();
                self.mode = Mode::InBody;
                return Flow::Reprocess(token);
            }
        }
        Flow::Done
    }

    /// Der Modus „after after frameset“.
    pub(super) fn after_after_frameset(&mut self, token: Token) -> Flow {
        match token {
            Token::Comment(data) => {
                let location = self.document_location();
                self.insert_comment(data, Some(location));
            }
            Token::ProcessingInstruction { target, data } => {
                let location = self.document_location();
                self.insert_processing_instruction(target, data, Some(location));
            }
            Token::Doctype(_) => return self.in_body(token),
            Token::Character(c) if is_whitespace(c) => return self.in_body(token),
            Token::StartTag(ref tag) if name(tag) == "html" => return self.in_body(token),
            Token::Eof => self.stop_parsing(),
            Token::StartTag(ref tag) if name(tag) == "noframes" => return self.in_head(token),
            _ => self.parse_error(),
        }
        Flow::Done
    }
}

/// Ob ein Attribut diesen qualifizierten Namen hat (Präfix, Doppelpunkt,
/// lokaler Name).
fn qualified_name_is(attribute: &dom::Attribute, name: &DomString) -> bool {
    match &attribute.prefix {
        None => attribute.local_name == *name,
        Some(prefix) => {
            let mut qualified = prefix.clone();
            qualified.push(':');
            qualified.push_dom_string(&attribute.local_name);
            qualified == *name
        }
    }
}
