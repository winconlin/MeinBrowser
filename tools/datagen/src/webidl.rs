//! Eigene FIRST/FOLLOW-Berechnung und LL(1)-Tabellen aus der normativen IDL-Grammatik.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

pub const DATA: &str = "data/webidl";
pub const OUTPUT: &str = "crates/mb-webidl/src/parser/tables.rs";
const NT: u16 = 0x8000;
type Rule = (usize, Vec<u16>);

fn small(value: usize) -> Result<u16, String> {
    u16::try_from(value).map_err(|_| "WebIDL-Tabelle zu groß".to_owned())
}

pub fn run(read: &dyn Fn(&str) -> Result<String, String>) -> Result<String, String> {
    generate(&read("grammar.txt")?)
}

fn first(sequence: &[u16], sets: &[BTreeSet<u16>], nullable: &[bool]) -> (BTreeSet<u16>, bool) {
    let mut result = BTreeSet::new();
    for &symbol in sequence {
        if symbol & NT == 0 {
            result.insert(symbol);
            return (result, false);
        }
        let index = usize::from(symbol & !NT);
        result.extend(&sets[index]);
        if !nullable[index] {
            return (result, false);
        }
    }
    (result, true)
}

fn generate(text: &str) -> Result<String, String> {
    let mut names = Vec::new();
    let mut raw = Vec::new();
    let mut current = None;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(name) = line.strip_suffix(':') {
            let name = name.trim();
            if !name.starts_with(|c: char| c.is_ascii_uppercase())
                || !name.bytes().all(|b| b.is_ascii_alphanumeric())
                || names.iter().any(|old| old == name)
            {
                return Err(format!("Ungültiger/doppelter Produktionsname {name:?}"));
            }
            current = Some(names.len());
            names.push(name.to_owned());
        } else {
            let lhs = current.ok_or("Alternative ohne Produktion")?;
            let rhs: Vec<String> = if line == "ε" {
                Vec::new()
            } else {
                line.split_whitespace().map(str::to_owned).collect()
            };
            raw.push((lhs, rhs));
        }
    }
    if names.len() >= usize::from(NT) {
        return Err("Zu viele Nichtterminale".to_owned());
    }
    let start = names
        .iter()
        .position(|name| name == "Definitions")
        .ok_or("Startsymbol Definitions fehlt")?;
    let mut terminals = BTreeSet::from(["$".to_owned()]);
    for (_, rhs) in &raw {
        for symbol in rhs {
            if symbol.starts_with('"') && symbol.ends_with('"') && symbol.len() > 2 {
                let literal = &symbol[1..symbol.len() - 1];
                if matches!(
                    literal,
                    "$" | "integer" | "decimal" | "identifier" | "string" | "other"
                ) {
                    return Err(format!("Reservierter Terminalname {literal}"));
                }
                terminals.insert(literal.to_owned());
            } else if matches!(
                symbol.as_str(),
                "integer" | "decimal" | "identifier" | "string" | "other"
            ) {
                terminals.insert(symbol.clone());
            } else if !names.contains(symbol) {
                return Err(format!("Unbekanntes Grammatiksymbol {symbol}"));
            }
        }
    }
    if terminals.len() >= usize::from(NT) {
        return Err("Zu viele Terminals".to_owned());
    }
    let terminals: Vec<_> = terminals.into_iter().collect();
    let eof = small(terminals.binary_search(&"$".to_owned()).expect("EOF"))?;
    let mut rules: Vec<Rule> = Vec::new();
    for (lhs, rhs) in raw {
        let mut symbols = Vec::new();
        for symbol in rhs {
            symbols.push(
                if let Some(index) = names.iter().position(|name| name == &symbol) {
                    NT | small(index)?
                } else {
                    let name = symbol
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                        .unwrap_or(&symbol);
                    small(
                        terminals
                            .binary_search_by(|s| s.as_str().cmp(name))
                            .map_err(|_| "Terminal fehlt")?,
                    )?
                },
            );
        }
        rules.push((lhs, symbols));
    }
    small(rules.len())?;
    let mut sets = vec![BTreeSet::new(); names.len()];
    let mut nullable = vec![false; names.len()];
    loop {
        let before = (sets.clone(), nullable.clone());
        for (lhs, rhs) in &rules {
            let (values, empty) = first(rhs, &sets, &nullable);
            sets[*lhs].extend(values);
            nullable[*lhs] |= empty;
        }
        if before == (sets.clone(), nullable.clone()) {
            break;
        }
    }
    if sets
        .iter()
        .zip(&nullable)
        .any(|(set, empty)| set.is_empty() && !empty)
    {
        return Err("Nicht produktive oder zyklische Produktion".to_owned());
    }
    let mut follow = vec![BTreeSet::new(); names.len()];
    follow[start].insert(eof);
    loop {
        let before = follow.clone();
        for (lhs, rhs) in &rules {
            for (at, &symbol) in rhs.iter().enumerate() {
                if symbol & NT != 0 {
                    let index = usize::from(symbol & !NT);
                    let (mut values, empty) = first(&rhs[at + 1..], &sets, &nullable);
                    if empty {
                        values.extend(&follow[*lhs]);
                    }
                    follow[index].extend(values);
                }
            }
        }
        if before == follow {
            break;
        }
    }
    let mut table = vec![BTreeMap::new(); names.len()];
    for (index, (lhs, rhs)) in rules.iter().enumerate() {
        let (mut values, empty) = first(rhs, &sets, &nullable);
        if empty {
            values.extend(&follow[*lhs]);
        }
        for terminal in values {
            if table[*lhs].insert(terminal, small(index)?).is_some() {
                return Err(format!(
                    "LL(1)-Konflikt in {} bei {}",
                    names[*lhs],
                    terminals[usize::from(terminal)]
                ));
            }
        }
    }
    emit(&names, &terminals, &rules, &table, start)
}

fn emit(
    names: &[String],
    terminals: &[String],
    rules: &[Rule],
    table: &[BTreeMap<u16, u16>],
    start: usize,
) -> Result<String, String> {
    let mut out = String::from(
        "// Erzeugt von tools/datagen aus data/webidl/grammar.txt. Nicht von Hand ändern.\n// Normative Grammatik: Copyright © WHATWG (Apple, Google, Mozilla, Microsoft).\n// SPDX-License-Identifier: BSD-3-Clause; vollständige Bedingungen: data/webidl/LICENSE.txt.\n\n",
    );
    writeln!(out, "pub(super) const START: u16 = {};", NT | small(start)?).expect("String");
    for (name, entries) in [("NAMES", names), ("TERMINALS", terminals)] {
        writeln!(
            out,
            "#[rustfmt::skip]\npub(super) const {name}: &[&str] = &["
        )
        .expect("String");
        for entry in entries {
            writeln!(out, "    {entry:?},").expect("String");
        }
        out.push_str("];\n\n");
    }
    out.push_str("#[rustfmt::skip]\npub(super) const RULES: &[(u16, u16)] = &[\n");
    let mut symbols: Vec<u16> = Vec::new();
    for (_, rhs) in rules {
        writeln!(
            out,
            "    ({}, {}),",
            small(symbols.len())?,
            small(rhs.len())?
        )
        .expect("String");
        symbols.extend(rhs);
    }
    small(symbols.len())?;
    out.push_str("];\n\n#[rustfmt::skip]\npub(super) const SYMBOLS: &[u16] = &[\n");
    for symbol in symbols {
        writeln!(out, "    {symbol},").expect("String");
    }
    out.push_str("];\n\n#[rustfmt::skip]\npub(super) const ROWS: &[usize] = &[\n");
    let mut offset = 0;
    for row in table {
        writeln!(out, "    {offset},").expect("String");
        offset += row.len();
    }
    writeln!(out, "    {offset},\n];\n").expect("String");
    out.push_str("#[rustfmt::skip]\npub(super) const ENTRIES: &[(u16, u16)] = &[\n");
    for row in table {
        for (terminal, rule) in row {
            writeln!(out, "    ({terminal}, {rule}),").expect("String");
        }
    }
    out.push_str("];\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_and_follow_conflicts_and_unproductive_cycles_are_rejected() {
        for text in [
            "Definitions :\n \"a\"\n \"a\" \"b\"\n",
            "Definitions :\n A \"x\"\nA :\n \"x\"\n ε\n",
            "Definitions :\n Missing\n",
            "Definitions :\n A\nA :\n Definitions\n",
            "Definitions :\n \"$\"\n",
            "Definitions :\n \"identifier\" integer\n",
        ] {
            assert!(generate(text).is_err(), "{text}");
        }
    }
    #[test]
    fn nullable_chains_and_recursive_lists_generate_tables() {
        let text = "Definitions :\n A\nA :\n \"a\" A\n ε\n";
        let output = generate(text).expect("LL(1)");
        assert!(output.contains("(0, 0)"), "EOF/FOLLOW rule");
        assert!(output.contains("(1, 1)"), "FIRST rule");
    }
}
