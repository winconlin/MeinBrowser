//! `import` und `export` (Abschnitt 16.2).

use std::collections::BTreeSet;

use super::pattern::Names;
use super::scope::Binding;
use super::statement::Position;
use super::{ErrorKind, Parser, Result};
use crate::ast::{Atom, List, Node, NodeId};
use crate::lexer::{Goal, Punctuator, TokenKind};

/// Ein *ModuleExportName*: ein Name oder eine Zeichenkette.
struct ExportName {
    atom: Atom,
    position: usize,
    /// Der Name als Bezeichner, wenn er keine Zeichenkette ist.
    identifier: Option<String>,
}

impl Parser<'_> {
    /// *ModuleItem*
    pub(super) fn module_item(&mut self) -> Result<NodeId> {
        if self.at_keyword("import", Goal::RegExp)? {
            let expression = self.peek(Goal::RegExp, Goal::Div)?.is_some_and(|next| {
                matches!(
                    next.kind,
                    TokenKind::Punctuator(Punctuator::LeftParenthesis | Punctuator::Dot)
                )
            });
            if !expression {
                return self.import_declaration();
            }
        } else if self.at_keyword("export", Goal::RegExp)? {
            return self.export_declaration();
        }
        self.statement_list_item()
    }

    /// Am Ende des Moduls: Jeder lokale Name hinter `export { … }` muss im
    /// Modul deklariert sein.
    pub(super) fn check_exports(&mut self) -> Result<()> {
        let bindings = std::mem::take(&mut self.exported_bindings);
        for (name, position) in bindings {
            if !self.top_level_declares(name) {
                return Err(self.error_at(ErrorKind::UndeclaredExport, position));
            }
        }
        Ok(())
    }

    /// *ImportDeclaration*
    fn import_declaration(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        let mark = self.scratch.len();
        let source = if matches!(self.token(Goal::Div)?.kind, TokenKind::String(_)) {
            self.module_specifier()?
        } else {
            self.import_clause()?;
            self.expect_keyword("from", Goal::Div)?;
            self.module_specifier()?
        };
        let specifiers = self.finish_list(mark);
        let attributes = self.with_clause()?;
        self.semicolon()?;
        Ok(self.node(
            Node::Import {
                specifiers,
                source,
                attributes,
            },
            start,
        ))
    }

    /// *ImportClause*, die Bindungen nach `self.scratch`.
    fn import_clause(&mut self) -> Result<()> {
        if self.name(Goal::Div)?.is_some() {
            let (local, position) = self.imported_binding()?;
            let default = self.node(Node::ImportDefault(local), position);
            self.scratch.push(default);
            if !self.eat(Punctuator::Comma, Goal::Div)? {
                return Ok(());
            }
        }
        let start = self.start(Goal::Div)?;
        match self.punctuator(Goal::Div)? {
            Some(Punctuator::Star) => {
                self.advance();
                self.expect_keyword("as", Goal::Div)?;
                let (local, _) = self.imported_binding()?;
                let namespace = self.node(Node::ImportNamespace(local), start);
                self.scratch.push(namespace);
                Ok(())
            }
            Some(Punctuator::LeftBrace) => self.named_imports(),
            _ => Err(self.unexpected(Goal::Div)),
        }
    }

    /// *NamedImports*
    fn named_imports(&mut self) -> Result<()> {
        self.advance();
        loop {
            if self.at(Punctuator::RightBrace, Goal::Div)? {
                break;
            }
            let start = self.start(Goal::Div)?;
            let renamed = match &self.token(Goal::Div)?.kind {
                TokenKind::String(_) => true,
                TokenKind::Name(_) => self
                    .peek(Goal::Div, Goal::Div)?
                    .is_some_and(|next| matches!(&next.kind, TokenKind::Name(name) if !name.escaped && name.value == "as")),
                _ => return Err(self.unexpected(Goal::Div)),
            };
            let (imported, local) = if renamed {
                let imported = self.module_export_name()?.atom;
                self.expect_keyword("as", Goal::Div)?;
                (imported, self.imported_binding()?.0)
            } else {
                let (local, _) = self.imported_binding()?;
                (local, local)
            };
            let specifier = self.node(Node::ImportSpecifier { imported, local }, start);
            self.scratch.push(specifier);
            if !self.eat(Punctuator::Comma, Goal::Div)? {
                break;
            }
        }
        self.expect(Punctuator::RightBrace, Goal::Div)
    }

    /// *ImportedBinding*, lexikalisch deklariert im Modul.
    fn imported_binding(&mut self) -> Result<(Atom, usize)> {
        let (atom, position) = self.binding_identifier_token()?;
        self.declare_lexical(atom, Binding::Lexical, position)?;
        Ok((atom, position))
    }

    /// Eine Zeichenkette als aktuelles Token.
    fn string_literal(&mut self) -> Result<(Vec<u16>, usize)> {
        let position = self.start(Goal::Div)?;
        let TokenKind::String(string) = &self.token(Goal::Div)?.kind else {
            return Err(self.unexpected(Goal::Div));
        };
        // Module sind strikt.
        if string.legacy_escape {
            return Err(self.error(ErrorKind::StrictLegacyLiteral));
        }
        let value = string.value.clone();
        self.advance();
        Ok((value, position))
    }

    /// *ModuleSpecifier*
    fn module_specifier(&mut self) -> Result<Atom> {
        let (value, _) = self.string_literal()?;
        Ok(self.intern(&value))
    }

    /// *ModuleExportName*. Eine Zeichenkette muss gültiges UTF-16 sein.
    fn module_export_name(&mut self) -> Result<ExportName> {
        if let Some(name) = self.name(Goal::Div)? {
            let position = self.start(Goal::Div)?;
            self.advance();
            let atom = self.intern_name(&name);
            return Ok(ExportName {
                atom,
                position,
                identifier: Some(name.value),
            });
        }
        let (value, position) = self.string_literal()?;
        if char::decode_utf16(value.iter().copied()).any(|unit| unit.is_err()) {
            return Err(self.error_at(ErrorKind::InvalidExportName, position));
        }
        Ok(ExportName {
            atom: self.intern(&value),
            position,
            identifier: None,
        })
    }

    /// *WithClause*, wenn eine folgt.
    fn with_clause(&mut self) -> Result<List> {
        if !self.eat_keyword("with", Goal::Div)? {
            return Ok(List::default());
        }
        self.expect(Punctuator::LeftBrace, Goal::Div)?;
        let mark = self.scratch.len();
        let mut keys: BTreeSet<Vec<u16>> = BTreeSet::new();
        loop {
            if self.at(Punctuator::RightBrace, Goal::Div)? {
                break;
            }
            let start = self.start(Goal::Div)?;
            let key = match self.name(Goal::Div)? {
                Some(name) => {
                    self.advance();
                    name.value.encode_utf16().collect()
                }
                None => self.string_literal()?.0,
            };
            self.expect(Punctuator::Colon, Goal::Div)?;
            let (value, _) = self.string_literal()?;
            if keys.contains(&key) {
                return Err(self.error_at(ErrorKind::DuplicateImportAttribute, start));
            }
            let key_atom = self.intern(&key);
            keys.insert(key);
            let value = self.intern(&value);
            let attribute = self.node(
                Node::ImportAttribute {
                    key: key_atom,
                    value,
                },
                start,
            );
            self.scratch.push(attribute);
            if !self.eat(Punctuator::Comma, Goal::Div)? {
                break;
            }
        }
        self.expect(Punctuator::RightBrace, Goal::Div)?;
        Ok(self.finish_list(mark))
    }

    /// *ExportDeclaration*
    fn export_declaration(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        if self.at_keyword("default", Goal::RegExp)? {
            let position = self.start(Goal::RegExp)?;
            self.advance();
            self.add_export_name("default".encode_utf16().collect(), position)?;
            let declaration = self.export_default()?;
            return Ok(self.node(Node::ExportDefault(declaration), start));
        }
        match self.punctuator(Goal::RegExp)? {
            Some(Punctuator::Star) => return self.export_all(start),
            Some(Punctuator::LeftBrace) => return self.export_named(start),
            _ => {}
        }
        let declaration_start = self.start(Goal::RegExp)?;
        let Some(name) = self.name(Goal::RegExp)? else {
            return Err(self.unexpected(Goal::RegExp));
        };
        let declaration = match name.value.as_str() {
            _ if name.escaped => return Err(self.unexpected(Goal::RegExp)),
            "var" => self.statement(Position::List)?,
            "let" if self.let_declaration_follows()? => self.statement_list_item()?,
            "const" | "function" | "class" => self.statement_list_item()?,
            "async" if self.async_function_follows()? => self.statement_list_item()?,
            _ => return Err(self.unexpected(Goal::RegExp)),
        };
        let mut names: Names = Vec::new();
        match *self.ast.node(declaration) {
            Node::Variables { declarators, .. } => {
                for &declarator in self.ast.list(declarators) {
                    if let Node::Declarator { target, .. } = *self.ast.node(declarator) {
                        self.bound_names(target, &mut names);
                    }
                }
            }
            Node::FunctionDeclaration(function) => {
                if let Some(name) = self
                    .ast
                    .function(function)
                    .and_then(|function| function.name)
                {
                    names.push((name, declaration_start));
                }
            }
            Node::ClassDeclaration(class) => {
                if let Some(name) = self.ast.class(class).and_then(|class| class.name) {
                    names.push((name, declaration_start));
                }
            }
            _ => return Err(self.error_at(ErrorKind::UnexpectedToken, declaration_start)),
        }
        for (name, position) in names {
            self.add_export_name(self.ast.atom(name).to_vec(), position)?;
        }
        Ok(self.node(Node::ExportDeclaration(declaration), start))
    }

    /// Was nach `export default` steht.
    fn export_default(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        if self.at_keyword("function", Goal::RegExp)? {
            return self.function_declaration(start, false, true);
        }
        if self.at_keyword("async", Goal::RegExp)? && self.async_function_follows()? {
            self.advance();
            return self.function_declaration(start, true, true);
        }
        if self.at_keyword("class", Goal::RegExp)? {
            return self.class_declaration(true);
        }
        let expression = self.assignment(true, None)?;
        self.semicolon()?;
        Ok(expression)
    }

    /// `export * from …` und `export * as name from …`.
    fn export_all(&mut self, start: usize) -> Result<NodeId> {
        self.advance();
        let exported = if self.eat_keyword("as", Goal::Div)? {
            let name = self.module_export_name()?;
            self.add_export_name(self.ast.atom(name.atom).to_vec(), name.position)?;
            Some(name.atom)
        } else {
            None
        };
        self.expect_keyword("from", Goal::Div)?;
        let source = self.module_specifier()?;
        let attributes = self.with_clause()?;
        self.semicolon()?;
        Ok(self.node(
            Node::ExportAll {
                exported,
                source,
                attributes,
            },
            start,
        ))
    }

    /// `export { … }`, vielleicht mit `from`.
    fn export_named(&mut self, start: usize) -> Result<NodeId> {
        self.advance();
        let mark = self.scratch.len();
        let mut locals: Vec<ExportName> = Vec::new();
        loop {
            if self.at(Punctuator::RightBrace, Goal::Div)? {
                break;
            }
            let specifier_start = self.start(Goal::Div)?;
            let local = self.module_export_name()?;
            let exported = if self.eat_keyword("as", Goal::Div)? {
                self.module_export_name()?
            } else {
                ExportName {
                    atom: local.atom,
                    position: local.position,
                    identifier: None,
                }
            };
            self.add_export_name(self.ast.atom(exported.atom).to_vec(), exported.position)?;
            let specifier = self.node(
                Node::ExportSpecifier {
                    local: local.atom,
                    exported: exported.atom,
                },
                specifier_start,
            );
            self.scratch.push(specifier);
            locals.push(local);
            if !self.eat(Punctuator::Comma, Goal::Div)? {
                break;
            }
        }
        self.expect(Punctuator::RightBrace, Goal::Div)?;
        let specifiers = self.finish_list(mark);
        let (source, attributes) = if self.eat_keyword("from", Goal::Div)? {
            let source = self.module_specifier()?;
            (Some(source), self.with_clause()?)
        } else {
            // Ohne `from` sind die lokalen Namen Verweise auf Bindungen
            // (*ReferencedBindings*).
            for local in locals {
                let Some(identifier) = local.identifier else {
                    return Err(self.error_at(ErrorKind::InvalidExportName, local.position));
                };
                if super::is_reserved_word(&identifier)
                    || super::is_strict_reserved_word(&identifier)
                    || identifier == "await"
                {
                    return Err(self.error_at(ErrorKind::ReservedWord, local.position));
                }
                self.exported_bindings.push((local.atom, local.position));
            }
            (None, List::default())
        };
        self.semicolon()?;
        Ok(self.node(
            Node::ExportNamed {
                specifiers,
                source,
                attributes,
            },
            start,
        ))
    }

    /// Merkt sich einen exportierten Namen; jeder darf nur einmal vorkommen.
    fn add_export_name(&mut self, name: Vec<u16>, position: usize) -> Result<()> {
        if !self.exported_names.insert(name) {
            return Err(self.error_at(ErrorKind::DuplicateExport, position));
        }
        Ok(())
    }
}
