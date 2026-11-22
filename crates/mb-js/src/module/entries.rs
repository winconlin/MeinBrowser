//! Die statische Semantik eines Moduls für seinen Record: ModuleRequests,
//! ImportEntries und ExportEntries (Abschnitte 16.2.1.3 bis 16.2.3) und
//! deren Aufteilung in ParseModule (Abschnitt 16.2.1.7.1, Schritte 4 bis
//! 10).

use crate::ast::{Ast, List, Node, NodeId};
use crate::compiler::DEFAULT_BINDING;

use super::{ImportEntry, ImportName, IndirectExport, ModuleRequest};

/// Was ParseModule aus dem Syntaxbaum liest.
pub(super) struct Entries {
    pub(super) requested: Vec<ModuleRequest>,
    pub(super) imports: Vec<ImportEntry>,
    pub(super) local_exports: Vec<(Vec<u16>, Vec<u16>)>,
    pub(super) indirect_exports: Vec<IndirectExport>,
    pub(super) star_exports: Vec<usize>,
}

/// Ein ExportEntry Record vor der Aufteilung.
enum Export {
    /// Ohne [[ModuleRequest]]: Exportname und lokaler Name.
    Local(Vec<u16>, Vec<u16>),
    /// Mit [[ModuleRequest]]: Exportname, `None` bei `export *`, und
    /// [[ImportName]], `None` für ~all-but-default~.
    From {
        export: Option<Vec<u16>>,
        request: usize,
        import_name: Option<ImportName>,
    },
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

pub(super) fn collect(ast: &Ast) -> Entries {
    let mut requested: Vec<ModuleRequest> = Vec::new();
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    let mut request = |source, attributes| -> usize {
        let wanted = module_request(ast, source, attributes);
        match requested.iter().position(|known| *known == wanted) {
            Some(index) => index,
            None => {
                requested.push(wanted);
                requested.len().saturating_sub(1)
            }
        }
    };
    for &item in ast.body() {
        match *ast.node(item) {
            Node::Import {
                specifiers,
                source,
                attributes,
            } => {
                let request = request(source, attributes);
                for &specifier in ast.list(specifiers) {
                    let (import_name, local) = match *ast.node(specifier) {
                        Node::ImportDefault(local) => (ImportName::Name(units("default")), local),
                        Node::ImportNamespace(local) => (ImportName::Namespace, local),
                        Node::ImportSpecifier { imported, local } => {
                            (ImportName::Name(ast.atom(imported).to_vec()), local)
                        }
                        _ => continue,
                    };
                    imports.push(ImportEntry {
                        request,
                        import_name,
                        local: ast.atom(local).to_vec(),
                    });
                }
            }
            Node::ExportNamed {
                specifiers,
                source,
                attributes,
            } => {
                let request = source.map(|source| request(source, attributes));
                for &specifier in ast.list(specifiers) {
                    let Node::ExportSpecifier { local, exported } = *ast.node(specifier) else {
                        continue;
                    };
                    let local = ast.atom(local).to_vec();
                    let exported = ast.atom(exported).to_vec();
                    exports.push(match request {
                        Some(request) => Export::From {
                            export: Some(exported),
                            request,
                            import_name: Some(ImportName::Name(local)),
                        },
                        None => Export::Local(exported, local),
                    });
                }
            }
            Node::ExportAll {
                exported,
                source,
                attributes,
            } => {
                let request = request(source, attributes);
                exports.push(match exported {
                    Some(exported) => Export::From {
                        export: Some(ast.atom(exported).to_vec()),
                        request,
                        import_name: Some(ImportName::Namespace),
                    },
                    None => Export::From {
                        export: None,
                        request,
                        import_name: None,
                    },
                });
            }
            Node::ExportDeclaration(declaration) => {
                for name in declared_names(ast, declaration) {
                    exports.push(Export::Local(name.clone(), name));
                }
            }
            Node::ExportDefault(declaration) => {
                let name = match *ast.node(declaration) {
                    Node::FunctionDeclaration(id) => ast
                        .function(id)
                        .and_then(|function| function.name)
                        .map(|name| ast.atom(name).to_vec()),
                    Node::ClassDeclaration(id) => ast
                        .class(id)
                        .and_then(|class| class.name)
                        .map(|name| ast.atom(name).to_vec()),
                    _ => None,
                };
                let local = name.unwrap_or_else(|| units(DEFAULT_BINDING));
                exports.push(Export::Local(units("default"), local));
            }
            _ => {}
        }
    }
    // ParseModule, Schritt 10.
    let mut local_exports = Vec::new();
    let mut indirect_exports = Vec::new();
    let mut star_exports = Vec::new();
    for export in exports {
        match export {
            // Auch ein importierter Namensraum wird so weitergegeben, als
            // käme er direkt aus seinem Modul.
            Export::Local(export, local) => {
                match imports.iter().find(|entry| entry.local == local) {
                    None => local_exports.push((export, local)),
                    Some(entry) => indirect_exports.push(IndirectExport {
                        export,
                        request: entry.request,
                        import_name: entry.import_name.clone(),
                    }),
                }
            }
            Export::From {
                export: None,
                request,
                ..
            }
            | Export::From {
                request,
                import_name: None,
                ..
            } => star_exports.push(request),
            Export::From {
                export: Some(export),
                request,
                import_name: Some(import_name),
            } => indirect_exports.push(IndirectExport {
                export,
                request,
                import_name,
            }),
        }
    }
    Entries {
        requested,
        imports,
        local_exports,
        indirect_exports,
        star_exports,
    }
}

/// Ein ModuleRequest Record mit den Attributen aus WithClauseToAttributes,
/// nach Schlüssel geordnet.
fn module_request(ast: &Ast, source: crate::ast::Atom, attributes: List) -> ModuleRequest {
    let mut pairs: Vec<(Vec<u16>, Vec<u16>)> = ast
        .list(attributes)
        .iter()
        .filter_map(|&attribute| match *ast.node(attribute) {
            Node::ImportAttribute { key, value } => {
                Some((ast.atom(key).to_vec(), ast.atom(value).to_vec()))
            }
            _ => None,
        })
        .collect();
    pairs.sort();
    ModuleRequest {
        specifier: ast.atom(source).to_vec(),
        attributes: pairs,
    }
}

/// BoundNames einer Deklaration hinter `export`, in der Reihenfolge des
/// Quelltexts, ohne Rekursion.
fn declared_names(ast: &Ast, declaration: NodeId) -> Vec<Vec<u16>> {
    let mut names = Vec::new();
    match *ast.node(declaration) {
        Node::Variables { declarators, .. } => {
            for &declarator in ast.list(declarators) {
                if let Node::Declarator { target, .. } = *ast.node(declarator) {
                    let mut work = vec![target];
                    while let Some(node) = work.pop() {
                        match *ast.node(node) {
                            Node::BindingIdentifier(atom) => names.push(ast.atom(atom).to_vec()),
                            Node::ArrayPattern(list) | Node::ObjectPattern(list) => {
                                work.extend(ast.list(list).iter().rev());
                            }
                            Node::PatternProperty { value, .. } => work.push(value),
                            Node::WithDefault { target, .. } | Node::Rest(target) => {
                                work.push(target);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        Node::FunctionDeclaration(id) => {
            if let Some(name) = ast.function(id).and_then(|function| function.name) {
                names.push(ast.atom(name).to_vec());
            }
        }
        Node::ClassDeclaration(id) => {
            if let Some(name) = ast.class(id).and_then(|class| class.name) {
                names.push(ast.atom(name).to_vec());
            }
        }
        _ => {}
    }
    names
}
