//! Vorgeprüfte Funktionsrümpfe in `mb-js` (`mb_js::parser::PreCheck`).
//!
//! Das erste Byte wählt, wie gelesen wird, der Rest ist der Quelltext:
//!
//! - Bit 0: als Modul statt als Script.
//! - Bit 1: der Rest als UTF-16LE statt als UTF-8, damit einzelne
//!   Surrogate den Parser erreichen.
//! - Bits 2 und 3: die kleinste Zahl Knoten eines vorgeprüften Rumpfs.
//! - Bits 4 bis 6: die größte Tiefe seines Inhalts.
//! - Bit 7: Funktionsausdrücke direkt hinter `(` oder `!` bleiben ganz.
//!
//! `mb_js::pre_check::compare` liest den Quelltext einmal ganz und einmal
//! vorgeprüft, liest die Rümpfe nach, einmal erst vor dem Übersetzen und
//! einmal alle vorher, und vergleicht Fehler, Bäume, die Analyse für den
//! Übersetzer und den Bytecode. Jeder Unterschied ist ein Fund.

use mb_js::parser::{MAX_DEPTH, PreCheck};

use crate::{MAX_INPUT, utf16le};

const MIN_NODES: [usize; 4] = [0, 1, 8, 64];
const MAX_DEPTHS: [u32; 8] = [0, 1, 2, 4, 8, 16, 64, MAX_DEPTH];

pub fn js_pre_check(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let Some((&control, rest)) = data.split_first() else {
        return;
    };
    let module = control & 1 != 0;
    let source: Vec<u16> = if control & 2 != 0 {
        utf16le(rest)
    } else {
        String::from_utf8_lossy(rest).encode_utf16().collect()
    };
    let pre_check = PreCheck {
        min_nodes: MIN_NODES[usize::from(control >> 2 & 3)],
        max_depth: MAX_DEPTHS[usize::from(control >> 4 & 7)],
        immediate: control & 0x80 != 0,
    };
    if let Err(difference) = mb_js::pre_check::compare(&source, module, pre_check) {
        panic!("vorgeprüft anders als ganz gelesen: {difference:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(control: u8, source: &str) -> Vec<u8> {
        let mut data = vec![control];
        data.extend_from_slice(source.as_bytes());
        data
    }

    #[test]
    fn all_settings_agree_on_small_programs() {
        for source in [
            "function f(a, b = 2) { var c = a + b; return () => c + this.x; }",
            "class A extends Object { #x = 1; m() { return () => super.m; } static { this.y = 1; } }",
            "function f(o) { with (o) { return function () { return x; }; } }",
            "function f() { return eval('1') + arguments.length + new.target; }",
            "function* g() { yield function () { return [[[[1]]]]; }; }",
            "function f() { let a; let a; }",
            "function f() { [...(a), ]; } [...b] = c;",
            "export default function () { return import.meta; }",
            "await 1; export async function h() { return () => h; }",
            "function f() {",
            "(function () { return function () { return this; }; })(); !function () {}();",
        ] {
            for control in 0..=0xFF_u8 {
                if control & 2 == 0 {
                    js_pre_check(&input(control, source));
                }
            }
        }
    }

    #[test]
    fn start_corpus_and_empty_input() {
        js_pre_check(&[]);
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("seeds")
            .join("js-pre-check");
        let mut count = 0;
        for seed in std::fs::read_dir(directory).expect("tracked start corpus") {
            let bytes = std::fs::read(seed.expect("seed entry").path()).expect("seed bytes");
            js_pre_check(&bytes);
            count += 1;
        }
        assert!(count > 0);
    }
}
