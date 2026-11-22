//! Literalkompatibilität ohne ECMAScript-Konvertierung oder Wertallokation.

use super::*;

mod metadata;
mod numeric;
mod shape;
use metadata::Metadata;
use shape::{ANY, BIGINT, BOOLEAN, NULL, SEQUENCE, Shape, StringValue, UNDEFINED};

pub(super) fn check(
    index: &Index<'_>,
    members: &Members<'_>,
    types: &Types,
    work: &Work<'_>,
) -> Result<(), Error> {
    let shapes = shape::build(index, types, work)?;
    let metadata = Metadata::build(index, members, work)?;
    let checker = Checker {
        index,
        types,
        work,
        shapes,
        metadata,
    };
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        for member in members.members(id).expect("Member") {
            work.spend(1, Some(&member.location))?;
            if let Some(node) = member.types[0] {
                let ty = types.at(member.location.source, node).expect("Membertyp");
                if member.kind == crate::members::Kind::DictionaryMember {
                    checker.argument_type(ty)?;
                }
                for (literal, kind) in [
                    (member.constant, ErrorKind::ConstantValue),
                    (member.default, ErrorKind::DefaultValue),
                ] {
                    if let Some(literal) = literal {
                        checker.value(member.location.source, literal, ty, kind)?;
                    }
                }
            }
            if let Some(signature) = &member.signature {
                checker.arguments(members, signature)?;
            }
        }
        if let Some(signature) = members.callback(id) {
            checker.arguments(members, signature)?;
        }
    }
    Ok(())
}

struct Checker<'a, 's, 'b> {
    index: &'a Index<'s>,
    types: &'a Types,
    work: &'a Work<'b>,
    shapes: Vec<Shape>,
    metadata: Metadata<'s>,
}
impl Checker<'_, '_, '_> {
    fn argument_type(&self, ty: TypeId) -> Result<(), Error> {
        if self.shapes[ty.index()].flags & UNDEFINED != 0 {
            return Err(error(
                ErrorKind::UndefinedType,
                Some(&self.types.get(ty).expect("Typ").location),
            ));
        }
        Ok(())
    }
    fn arguments(
        &self,
        members: &Members<'_>,
        signature: &crate::members::Signature,
    ) -> Result<(), Error> {
        let mut later_optional = true;
        for argument in members
            .arguments(signature)
            .expect("Argumente")
            .iter()
            .rev()
        {
            self.work.spend(1, Some(&argument.identifier.location))?;
            let source = argument.identifier.location.source;
            let ty = self.types.at(source, argument.ty).expect("Argumenttyp");
            self.argument_type(ty)?;
            if let Some(default) = argument.default {
                self.value(source, default, ty, ErrorKind::DefaultValue)?;
            }
            if later_optional
                && self.shapes[ty.index()]
                    .dictionary
                    .is_some_and(|id| !self.metadata.required(id))
                && (!argument.optional || argument.default.is_none())
            {
                return Err(error(
                    ErrorKind::DictionaryArgumentDefault,
                    Some(&argument.identifier.location),
                ));
            }
            later_optional &= argument.optional;
        }
        Ok(())
    }
    fn value(&self, source: usize, node: NodeId, ty: TypeId, kind: ErrorKind) -> Result<(), Error> {
        let document = self.index.document(source).expect("Wertquelle");
        let at = location(document, source, node);
        let value = literal(document, node, self.work, &at)?;
        let s = self.shapes[ty.index()];
        let any = s.flags & ANY != 0;
        let accepted = match value {
            Literal::Boolean => any || s.flags & BOOLEAN != 0,
            Literal::Null => any || s.flags & NULL != 0,
            Literal::Undefined => any || s.flags & UNDEFINED != 0,
            Literal::Integer(raw) => {
                any || s.flags & BIGINT != 0 || s.number.is_some_and(|p| numeric::integer(raw, p))
            }
            Literal::Decimal(raw) => any || s.number.is_some_and(|p| numeric::decimal(raw, p)),
            Literal::Special => {
                any || matches!(
                    s.number,
                    Some(Primitive::UnrestrictedFloat | Primitive::UnrestrictedDouble)
                )
            }
            Literal::String(raw) => {
                any || match s.string {
                    Some(StringValue::Builtin(StringType::ByteString)) => {
                        raw.chars().all(|c| c <= '\u{ff}')
                    }
                    Some(StringValue::Builtin(_)) => true,
                    Some(StringValue::Enum(id)) => self.metadata.enum_value(id, raw),
                    None => false,
                }
            }
            Literal::Sequence => s.flags & SEQUENCE != 0,
            Literal::Dictionary => s.dictionary.is_some(),
        };
        if accepted {
            Ok(())
        } else {
            Err(error(kind, Some(&at)))
        }
    }
}

enum Literal<'a> {
    Boolean,
    Null,
    Undefined,
    Integer(&'a str),
    Decimal(&'a str),
    Special,
    String(&'a str),
    Sequence,
    Dictionary,
}
fn literal<'a>(
    document: &Document<'a>,
    root: NodeId,
    work: &Work<'_>,
    at: &Location,
) -> Result<Literal<'a>, Error> {
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        work.spend(1, Some(at))?;
        if let NodeKind::Token(kind) = document.node(id).expect("Wertknoten").kind {
            let raw = document.text(id).expect("Token");
            work.spend(raw.len() as u64, Some(at))?;
            return Ok(match kind {
                lexer::Kind::Integer => Literal::Integer(raw),
                lexer::Kind::Decimal => Literal::Decimal(raw),
                lexer::Kind::String => Literal::String(&raw[1..raw.len() - 1]),
                _ => match raw {
                    "true" | "false" => Literal::Boolean,
                    "null" => Literal::Null,
                    "undefined" => Literal::Undefined,
                    "Infinity" | "-Infinity" | "NaN" => Literal::Special,
                    "[" => Literal::Sequence,
                    "{" => Literal::Dictionary,
                    _ => unreachable!("DefaultValue-/ConstValue-Grammatik"),
                },
            });
        }
        pending.extend(document.children(id).rev());
    }
    unreachable!("nichtleere Wertproduktion")
}

#[cfg(test)]
mod tests;
