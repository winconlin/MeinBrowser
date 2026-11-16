use super::*;
use crate::regexp::compile::{CompileError, compile};
use crate::regexp::{Flags, parse};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn program(pattern: &[u16], flag_text: &str) -> Program {
    let flags = Flags::parse(&units(flag_text)).expect("Flags");
    let tree = parse(pattern, flags).expect("Muster");
    let mut spend = |_: u64| true;
    let budget = Budget {
        bytes: 1 << 26,
        spend: &mut spend,
    };
    compile(&tree, flags, budget).expect("Programm")
}

/// Der Treffer wie in JavaScript: die Stelle und die Gruppen, oder `null`.
fn find_units(pattern: &[u16], flag_text: &str, input: &[u16], last_index: usize) -> String {
    let program = program(pattern, flag_text);
    let mut spend = |_: u64| true;
    let budget = Budget {
        bytes: 1 << 24,
        spend: &mut spend,
    };
    let sticky = flag_text.contains('y');
    let Some(captures) = execute(&program, input, last_index, sticky, budget).expect("Abgleich")
    else {
        return "null".to_owned();
    };
    let groups: Vec<String> = captures
        .iter()
        .map(|span| match span {
            Some((start, end)) => {
                let text = &input[to_usize(*start)..to_usize(*end)];
                format!("\"{}\"", String::from_utf16_lossy(text))
            }
            None => "undefined".to_owned(),
        })
        .collect();
    let index = captures[0].expect("Treffer").0;
    format!("{index}:[{}]", groups.join(","))
}

/// Die Bereiche der Gruppen, für Eingaben mit einzelnen Surrogaten.
fn spans(
    pattern: &[u16],
    flag_text: &str,
    input: &[u16],
    last_index: usize,
) -> Option<Vec<Option<Span>>> {
    let program = program(pattern, flag_text);
    let mut spend = |_: u64| true;
    let budget = Budget {
        bytes: 1 << 20,
        spend: &mut spend,
    };
    execute(&program, input, last_index, false, budget).expect("Abgleich")
}

fn find(pattern: &str, flag_text: &str, input: &str) -> String {
    find_units(&units(pattern), flag_text, &units(input), 0)
}

fn matches(pattern: &str, flag_text: &str, input: &str) -> bool {
    find(pattern, flag_text, input) != "null"
}

#[test]
fn examples_from_the_specification() {
    // Die Beispiele aus den Anmerkungen in Abschnitt 22.2.2.
    assert_eq!(find("a|ab", "", "abc"), r#"0:["a"]"#);
    assert_eq!(
        find("((a)|(ab))((c)|(bc))", "", "abc"),
        r#"0:["abc","a","a",undefined,"bc",undefined,"bc"]"#
    );
    assert_eq!(find("a[a-z]{2,4}", "", "abcdefghi"), r#"0:["abcde"]"#);
    assert_eq!(find("a[a-z]{2,4}?", "", "abcdefghi"), r#"0:["abc"]"#);
    assert_eq!(
        find("(aa|aabaac|ba|b|c)*", "", "aabaac"),
        r#"0:["aaba","ba"]"#
    );
    assert_eq!(
        find("^(a+)\\1*,\\1+$", "", "aaaaaaaaaa,aaaaaaaaaaaaaaa"),
        r#"0:["aaaaaaaaaa,aaaaaaaaaaaaaaa","aaaaa"]"#
    );
    assert_eq!(
        find("(z)((a+)?(b+)?(c))*", "", "zaacbbbcac"),
        r#"0:["zaacbbbcac","z","ac","a",undefined,"c"]"#
    );
    assert_eq!(find("(a*)*", "", "b"), r#"0:["",undefined]"#);
    assert_eq!(find("(a*)b\\1+", "", "baaaac"), r#"0:["b",""]"#);
    assert_eq!(find("(?=(a+))", "", "baaabac"), r#"1:["","aaa"]"#);
    assert_eq!(find("(?=(a+))a*b\\1", "", "baaabac"), r#"3:["aba","a"]"#);
    assert_eq!(
        find("(.*?)a(?!(a+)b\\2c)\\2(.*)", "", "baaabaac"),
        r#"0:["baaabaac","ba",undefined,"abaac"]"#
    );
}

#[test]
fn alternatives_and_quantifiers() {
    assert_eq!(find("", "", "abc"), r#"0:[""]"#);
    assert_eq!(find("b", "", "abc"), r#"1:["b"]"#);
    assert_eq!(find("x", "", "abc"), "null");
    assert_eq!(find("a|b|c", "", "xcba"), r#"1:["c"]"#);
    assert_eq!(find("(?:ab|a)(?:c|bcd)", "", "abcd"), r#"0:["abc"]"#);
    assert_eq!(find("a*", "", "aaab"), r#"0:["aaa"]"#);
    assert_eq!(find("a*?", "", "aaab"), r#"0:[""]"#);
    assert_eq!(find("a+?b", "", "aaab"), r#"0:["aaab"]"#);
    assert_eq!(find("a?", "", "b"), r#"0:[""]"#);
    assert_eq!(find("ba??", "", "ba"), r#"0:["b"]"#);
    assert_eq!(find("a{0}b", "", "ab"), r#"1:["b"]"#);
    assert_eq!(find("a{2}", "", "aaaaa"), r#"0:["aa"]"#);
    assert_eq!(find("a{2,}", "", "aaaaa"), r#"0:["aaaaa"]"#);
    assert_eq!(find("a{2,}?", "", "aaaaa"), r#"0:["aa"]"#);
    assert_eq!(find("a{3,4}", "", "aa"), "null");
    assert_eq!(find("^(?:ab){2,3}$", "", "ababab"), r#"0:["ababab"]"#);
    assert_eq!(find("^(?:ab){2,3}$", "", "abababab"), "null");
    assert_eq!(
        find("(a|ab)(c|bcd)(d*)", "", "abcd"),
        r#"0:["abcd","a","bcd",""]"#
    );
    // Zähler verschachtelter Schleifen stören sich nicht.
    assert_eq!(find("^(?:a{2}b){2}$", "", "aabaab"), r#"0:["aabaab"]"#);
    assert_eq!(
        find("^(?:(?:a|b){2,3}c){2}$", "", "abcbabc"),
        r#"0:["abcbabc"]"#
    );
    assert_eq!(find("(?:a{1,2}){2}b", "", "aaab"), r#"0:["aaab"]"#);
}

#[test]
fn empty_iterations_end_a_loop() {
    // Ein Durchlauf, der nichts verbraucht, zählt nur, solange er nötig ist.
    assert_eq!(find("(?:)*", "", "a"), r#"0:[""]"#);
    assert_eq!(find("(a?)*", "", "aa"), r#"0:["aa","a"]"#);
    assert_eq!(find("(a?)*?b", "", "aab"), r#"0:["aab","a"]"#);
    assert_eq!(find("(?:a*)+", "", "b"), r#"0:[""]"#);
    assert_eq!(find("(a*)+", "", "b"), r#"0:["",""]"#);
    assert_eq!(find("(a*){2}", "", "b"), r#"0:["",""]"#);
    assert_eq!(find("(a*){2,}", "", "aab"), r#"0:["aa",""]"#);
    assert_eq!(find("(?:a?){3}b", "", "ab"), r#"0:["ab"]"#);
    assert_eq!(find("(\\2a|(b))*", "", "bba"), r#"0:["bba","a",undefined]"#);
    assert_eq!(find("(?:(?=a))*a", "", "a"), r#"0:["a"]"#);
    // Die Gruppen eines Atoms sind vor jedem Durchlauf undefiniert.
    assert_eq!(find("(?:(a)|(b))+", "", "ab"), r#"0:["ab",undefined,"b"]"#);
    assert_eq!(find("(?:(a)|b){2}", "", "ab"), r#"0:["ab",undefined]"#);
    assert_eq!(find("(?:(a)|b)*?c", "", "abc"), r#"0:["abc",undefined]"#);
}

#[test]
fn assertions() {
    assert_eq!(find("^b", "", "a\nb"), "null");
    assert_eq!(find("^b", "m", "a\nb"), r#"2:["b"]"#);
    assert_eq!(find("a$", "", "a\nb"), "null");
    assert_eq!(find("a$", "m", "a\r\nb"), r#"0:["a"]"#);
    assert_eq!(find("^$", "m", "a\u{2028}\u{2029}b"), r#"2:[""]"#);
    assert_eq!(find("\\bfoo\\b", "", "a foo b"), r#"2:["foo"]"#);
    assert_eq!(find("\\bfoo\\b", "", "afoo foob"), "null");
    assert_eq!(find("\\Boo", "", "foo"), r#"1:["oo"]"#);
    assert_eq!(find("\\B", "", ""), r#"0:[""]"#);
    assert_eq!(find("\\b", "", ""), "null");
    assert_eq!(find("\\b", "", "-a"), r#"1:[""]"#);
    // ſ und das Kelvin-Zeichen sind nur mit `i` und `u` Wortzeichen.
    assert_eq!(find("\\b", "i", "\u{17F}"), "null");
    assert_eq!(find("\\b", "u", "\u{17F}"), "null");
    assert_eq!(find("\\b", "iu", "\u{17F}"), r#"0:[""]"#);
    assert_eq!(find("\\B", "iv", "-\u{212A}"), r#"0:[""]"#);
    assert_eq!(find("\\b", "iv", "-\u{212A}"), r#"1:[""]"#);
}

#[test]
fn lookahead_and_lookbehind() {
    assert_eq!(find("a(?=b)", "", "acab"), r#"2:["a"]"#);
    assert_eq!(find("a(?!b)", "", "abac"), r#"2:["a"]"#);
    assert_eq!(find("(?!(a))b", "", "b"), r#"0:["b",undefined]"#);
    // In einen Lookahead kehrt die Rückverfolgung nicht zurück.
    assert_eq!(find("(?=(a+))a*b", "", "aab"), r#"0:["aab","aa"]"#);
    assert_eq!(find("(?=(a*))\\1b", "", "aab"), r#"0:["aab","aa"]"#);
    assert_eq!(find("^(?=(a|ab))\\1c", "", "abc"), "null");
    assert_eq!(
        find("(?<=\\$)\\d+(\\.\\d*)?", "", "cost $10.50"),
        r#"6:["10.50",".50"]"#
    );
    assert_eq!(find("(?<!\\$)\\b\\d+", "", "$10 20"), r#"4:["20"]"#);
    assert_eq!(find("(?<=a)", "", "ba"), r#"2:[""]"#);
    assert_eq!(find("(?<!a)", "", "a"), r#"0:[""]"#);
    // Rückwärts sind die Quantoren von rechts her gierig.
    assert_eq!(
        find("(?<=(\\d+)(\\d+))$", "", "1053"),
        r#"4:["","1","053"]"#
    );
    assert_eq!(
        find("(?<=(\\d+?)(\\d+))$", "", "1053"),
        r#"4:["","1","053"]"#
    );
    assert_eq!(
        find("(?<=(\\d+)(\\d+?))$", "", "1053"),
        r#"4:["","105","3"]"#
    );
    // Ein Rückverweis rechts von seiner Gruppe ist rückwärts noch leer,
    // einer links davon nicht.
    assert_eq!(find("(?<=(a)\\1)b", "", "aab"), r#"2:["b","a"]"#);
    assert_eq!(find("(?<=\\1(a))b", "", "aab"), r#"2:["b","a"]"#);
    assert_eq!(find("(?<=\\1(a))b", "", "xab"), "null");
    assert_eq!(find("(?<=ab(?=c)\\wd)e", "", "abcde"), r#"4:["e"]"#);
    assert_eq!(find("(?<=a(?<!a)c)d", "", "acd"), "null");
    assert_eq!(find("(?<=a(?<!b)c)d", "", "acd"), r#"2:["d"]"#);
    assert_eq!(find("(?<=a(?<!ba)c)d", "", "acd"), r#"2:["d"]"#);
    assert_eq!(find("(?<=^(a|bc)+)d", "", "abcad"), r#"4:["d","a"]"#);
    // Verschachtelt: Der äußere Lookahead behält die Gruppe des inneren.
    assert_eq!(find("(?=a(?=(b)))ab", "", "ab"), r#"0:["ab","b"]"#);
    assert_eq!(find("(?=a(?!(b)))a", "", "ac"), r#"0:["a",undefined]"#);
    assert_eq!(find("(?:(?=(a))a|b)*", "", "ab"), r#"0:["ab",undefined]"#);
}

#[test]
fn back_references() {
    assert_eq!(find("(a)\\1", "", "aa"), r#"0:["aa","a"]"#);
    assert_eq!(find("\\1(a)", "", "aa"), r#"0:["a","a"]"#);
    assert_eq!(find("(a)|\\1b", "", "b"), r#"0:["b",undefined]"#);
    assert_eq!(find("(ab)\\1", "i", "abAB"), r#"0:["abAB","ab"]"#);
    assert_eq!(find("(ab)\\1", "", "abAB"), "null");
    assert_eq!(find("(?<x>a)\\k<x>", "", "aa"), r#"0:["aa","a"]"#);
    // Zwei Gruppen mit einem Namen: Es zählt die, die definiert ist.
    assert_eq!(
        find("(?:(?<x>a)|(?<x>b))\\k<x>", "", "bb"),
        r#"0:["bb",undefined,"b"]"#
    );
    assert_eq!(find("(?:(?<x>a)|(?<x>b))\\k<x>", "", "ab"), "null");
    assert_eq!(
        find("(?:(?:(?<x>a)|(?<x>b))\\k<x>){2}", "", "aabb"),
        r#"0:["aabb",undefined,"b"]"#
    );
    // Mit `u` vergleicht der Rückverweis Codepunkte: Ein einzelnes
    // Surrogat ist nicht die Hälfte eines Ersatzpaars.
    let input = [0xD83D_u16, 0xD83D, 0xDE00];
    let twice = units("(.)\\1");
    assert_eq!(spans(&twice, "u", &input, 0), None);
    assert_eq!(
        spans(&twice, "", &input, 0),
        Some(vec![Some((0, 2)), Some((0, 1))])
    );
    assert_eq!(
        spans(&twice, "u", &[0xD83D, 0xD83D], 0),
        Some(vec![Some((0, 2)), Some((0, 1))])
    );
}

#[test]
fn ignore_case() {
    assert!(matches("[a-z]", "i", "B"));
    assert!(matches("B", "i", "b"));
    assert!(matches("\u{E9}", "i", "\u{C9}"));
    assert!(!matches("\u{E9}", "", "\u{C9}"));
    // ß wird groß zu zwei Zeichen und passt deshalb nur auf sich selbst.
    assert!(!matches("\u{DF}", "i", "SS"));
    assert!(!matches("\u{DF}", "i", "\u{1E9E}"));
    assert!(matches("\u{DF}", "iu", "\u{1E9E}"));
    // Ohne `u` gelangt kein Zeichen von außen in den ASCII-Bereich.
    assert!(!matches("\u{212A}", "i", "k"));
    assert!(matches("\u{212A}", "iu", "k"));
    assert!(!matches("\u{17F}", "i", "s"));
    assert!(matches("\u{17F}", "iu", "S"));
    assert!(matches("[^\\W]", "iu", "\u{17F}"));
    assert!(!matches("\\W", "iu", "\u{17F}"));
    assert!(!matches("\\W", "iu", "S"));
    assert!(matches("\\w", "iu", "\u{212A}"));
    assert!(!matches("\\w", "u", "\u{212A}"));
    assert!(matches("[\\u0390]", "iu", "\u{1FD3}"));
    assert!(!matches("[\\u0390]", "i", "\u{1FD3}"));
    // Ein Bereich zählt mit allem, was auf eines seiner Zeichen fällt.
    assert!(matches("[W-c]", "i", "x"));
    assert!(!matches("[^W-c]", "i", "x"));
    assert!(matches("[^a]", "i", "b"));
    assert!(!matches("[^a]", "i", "A"));
    // Gruppen ändern die Flags für ihren Rumpf.
    assert!(matches("^(?i:a)b$", "", "Ab"));
    assert!(!matches("^(?i:a)b$", "", "AB"));
    assert!(matches("^(?-i:a)b$", "i", "aB"));
    assert!(!matches("^(?-i:a)b$", "i", "AB"));
    assert!(matches("a(?s:.)b", "", "a\nb"));
    assert!(!matches("a.b", "", "a\nb"));
    assert!(matches("a.b", "s", "a\nb"));
    assert!(matches("a(?-s:.)b|c", "s", "c"));
    assert!(!matches("a(?-s:.)b", "s", "a\nb"));
    assert!(matches("a\n(?m:^)b", "", "a\nb"));
    assert!(matches("a\n(?m-i:^)B", "i", "a\nB"));
}

#[test]
fn characters_with_and_without_unicode() {
    let smile = "\u{1F600}";
    assert_eq!(find("^.$", "u", smile), "0:[\"\u{1F600}\"]");
    assert_eq!(find("^.$", "", smile), "null");
    assert_eq!(find("^..$", "", smile), "0:[\"\u{1F600}\"]");
    assert_eq!(find("^[^x]$", "v", smile), "0:[\"\u{1F600}\"]");
    assert_eq!(find("\\u{1F600}", "u", "a\u{1F600}"), "1:[\"\u{1F600}\"]");
    assert_eq!(
        find("[\u{1F600}-\u{1F64F}]", "u", "\u{1F610}"),
        "0:[\"\u{1F610}\"]"
    );
    assert_eq!(find("(?<=^.)a", "u", "\u{1F600}a"), r#"2:["a"]"#);
    assert_eq!(find("(?<=^.)a", "", "\u{1F600}a"), "null");
    // Ein einzelnes Surrogat passt mit `u` nicht auf die Hälfte eines
    // Ersatzpaars.
    let pair = [0xD834_u16, 0xDF06];
    assert_eq!(find_units(&[0xDF06], "u", &pair, 0), "null");
    assert_eq!(find_units(&[0xD834], "u", &pair, 0), "null");
    assert_ne!(find_units(&[0xDF06], "", &pair, 0), "null");
    assert_ne!(
        find_units(&[0xDF06], "u", &[0x61, 0xDF06, 0xD834], 0),
        "null"
    );
    assert_ne!(
        find_units(&[0xD834], "u", &[0x61, 0xDF06, 0xD834], 0),
        "null"
    );
    // Liegt der Anfang in einem Ersatzpaar, beginnt der Versuch davor; als
    // Anfang des Treffers gilt trotzdem `lastIndex`.
    let dot = units(".");
    assert_eq!(spans(&dot, "u", &pair, 1), Some(vec![Some((1, 2))]));
    assert_eq!(spans(&dot, "", &pair, 1), Some(vec![Some((1, 2))]));
    assert_eq!(spans(&dot, "u", &pair, 2), None);
    assert_eq!(spans(&dot, "u", &pair, 3), None);
    // Rückwärts wie vorwärts: zwei hohe Surrogate, dann ein niedriges.
    let input = [0xD83D_u16, 0xD83D, 0xDE00];
    assert_eq!(
        find_units(&units("(?<=(.)(.))$"), "u", &input, 0),
        "3:[\"\",\"\u{FFFD}\",\"\u{1F600}\"]"
    );
}

#[test]
fn sticky_and_last_index() {
    let (a, ba) = (units("a"), units("ba"));
    assert_eq!(find_units(&a, "y", &ba, 0), "null");
    assert_eq!(find_units(&a, "y", &ba, 1), r#"1:["a"]"#);
    assert_eq!(find_units(&a, "", &ba, 2), "null");
    assert_eq!(find_units(&units(""), "", &ba, 2), r#"2:[""]"#);
    assert_eq!(find_units(&units(""), "", &ba, 3), "null");
    assert_eq!(find_units(&units("^a"), "", &units("aa"), 1), "null");
    assert_eq!(
        find_units(&units("^a"), "m", &units("b\na"), 1),
        r#"2:["a"]"#
    );
}

#[test]
fn classes_and_escapes() {
    assert_eq!(find("[abc]+", "", "xcabz"), r#"1:["cab"]"#);
    assert_eq!(find("[^abc]+", "", "abxyc"), r#"2:["xy"]"#);
    assert_eq!(find("[]", "", "a"), "null");
    assert_eq!(find("[^]", "", "\n"), "0:[\"\n\"]");
    assert_eq!(find("[\\d-x]+", "", "1-x2a"), r#"0:["1-x2"]"#);
    assert_eq!(find("\\d+", "", "ab123c"), r#"2:["123"]"#);
    assert_eq!(find("\\D+", "", "12ab3"), r#"2:["ab"]"#);
    assert_eq!(
        find("\\s+", "", "a \t\n\u{A0}\u{FEFF}\u{2028}b"),
        "1:[\" \t\n\u{A0}\u{FEFF}\u{2028}\"]"
    );
    assert_eq!(find("\\S+", "", " ab "), r#"1:["ab"]"#);
    assert_eq!(find("\\w+", "", "-a_9Z-"), r#"1:["a_9Z"]"#);
    assert_eq!(find("\\W+", "", "ab-+c"), r#"2:["-+"]"#);
    assert_eq!(find("[\\w-]+", "", "+a-b+"), r#"1:["a-b"]"#);
    assert_eq!(find(".+", "", "ab\ncd"), r#"0:["ab"]"#);
    assert_eq!(find("\\p{Lu}+", "u", "abCDe"), r#"2:["CD"]"#);
    assert_eq!(find("\\P{Lu}+", "u", "ABcdE"), r#"2:["cd"]"#);
    assert_eq!(find("[\\p{L}\\d]+", "u", "-\u{E4}1-"), "1:[\"\u{E4}1\"]");
    assert_eq!(find("[^\\p{L}]+", "u", "ab12c"), r#"2:["12"]"#);
    assert_eq!(find("\\p{gc=Nd}+", "v", "a\u{661}2b"), "1:[\"\u{661}2\"]");
    assert_eq!(find("\\p{Any}", "u", "\n"), "0:[\"\n\"]");
    assert_eq!(find("\\p{ASCII}+", "u", "\u{E4}ab\u{E4}"), r#"1:["ab"]"#);
    assert_eq!(find("\\P{Assigned}", "u", "a\u{378}"), "1:[\"\u{378}\"]");
    // Mit `i` und `u` passt `\p{Lu}` wie `\P{Lu}` auf jeden Buchstaben mit
    // beiden Formen.
    assert!(matches("\\p{Lu}", "iu", "a"));
    assert!(matches("\\P{Lu}", "iu", "A"));
    assert!(!matches("\\p{Lu}", "iu", "1"));
}

#[test]
fn class_sets() {
    assert_eq!(find("[[a-z]--[aeiou]]+", "v", "hello"), r#"0:["h"]"#);
    assert_eq!(find("[\\w&&[^\\d]]+", "v", "12ab3"), r#"2:["ab"]"#);
    assert_eq!(find("[[a-z]&&[c-x]&&[^d]]+", "v", "abcdef"), r#"2:["c"]"#);
    assert_eq!(find("[[a-d]--b--c]+", "v", "abcd"), r#"0:["a"]"#);
    assert_eq!(find("[^[a-c]x]+", "v", "abxyzc"), r#"3:["yz"]"#);
    assert_eq!(find("[[^a]&&[^b]]", "v", "abc"), r#"2:["c"]"#);
    // Zeichenketten: die längste zuerst, die leere zuletzt.
    assert_eq!(find("[\\q{abc|d}x]", "v", "abc"), r#"0:["abc"]"#);
    assert_eq!(find("[\\q{ab|abc|}]", "v", "abd"), r#"0:["ab"]"#);
    assert_eq!(find("^[\\q{ab|abc|}]$", "v", ""), r#"0:[""]"#);
    assert_eq!(find("[\\q{ab|abc}]c", "v", "abc"), r#"0:["abc"]"#);
    assert_eq!(find("[\\q{ab|c}--\\q{ab}]", "v", "abc"), r#"2:["c"]"#);
    assert_eq!(find("[\\q{ab|c}&&\\q{ab|d}]", "v", "cdab"), r#"2:["ab"]"#);
    assert_eq!(find("(?<=[\\q{ab|b}])c", "v", "abc"), r#"2:["c"]"#);
    assert_eq!(find("(?<=^[\\q{ab|b}])c", "v", "abc"), r#"2:["c"]"#);
    assert_eq!(find("[\\q{ab}]+", "v", "ababa"), r#"0:["abab"]"#);
    // Mit `i` stehen die Mengen gefaltet.
    assert_eq!(find("[\\q{AB}]", "vi", "xaB"), r#"1:["aB"]"#);
    assert_eq!(find("[[A-Z]--[A-C]]+", "vi", "abcDe"), r#"3:["De"]"#);
    assert_eq!(find("[^A]+", "vi", "aAbB"), r#"2:["bB"]"#);
    assert_eq!(
        find("[\\w--\\d]+", "vi", "1\u{17F}K\u{212A}2"),
        "1:[\"\u{17F}K\u{212A}\"]"
    );
}

#[test]
fn unsupported_properties_are_reported() {
    for (pattern, flag_text) in [
        ("\\p{Script=Latin}", "u"),
        ("\\p{scx=Grek}", "u"),
        ("[\\p{Alphabetic}]", "v"),
        ("\\p{RGI_Emoji}", "v"),
    ] {
        let flags = Flags::parse(&units(flag_text)).expect("Flags");
        let tree = parse(&units(pattern), flags).expect("Muster");
        let mut spend = |_: u64| true;
        let budget = Budget {
            bytes: 1 << 20,
            spend: &mut spend,
        };
        assert_eq!(
            compile(&tree, flags, budget).err(),
            Some(CompileError::Unsupported),
            "{pattern}"
        );
    }
}

#[test]
fn limits_stop_compiling_and_matching() {
    let flags = Flags::default();
    // Das Programm ist zu groß.
    let tree = parse(&units("abcdefgh"), flags).expect("Muster");
    let mut spend = |_: u64| true;
    let small = Budget {
        bytes: 64,
        spend: &mut spend,
    };
    assert_eq!(
        compile(&tree, flags, small).err(),
        Some(CompileError::TooLarge)
    );
    // Die Arbeit beim Übersetzen wird gemeldet und kann abbrechen.
    let long = units(&"[a-z]\\d".repeat(400));
    let tree = parse(&long, flags).expect("Muster");
    let mut seen = 0_u64;
    let mut count = |work: u64| {
        seen += work;
        true
    };
    let budget = Budget {
        bytes: 1 << 20,
        spend: &mut count,
    };
    let program = compile(&tree, flags, budget).expect("Programm");
    assert!(seen >= 1600, "{seen}");
    // Gleiche Mengen stehen nur einmal im Programm.
    assert_eq!(program.sets.len(), 2);
    let mut refuse = |_: u64| false;
    let budget = Budget {
        bytes: 1 << 20,
        spend: &mut refuse,
    };
    assert_eq!(
        compile(&tree, flags, budget).err(),
        Some(CompileError::Stopped)
    );

    // Ein Muster mit exponentiell vielen Wegen läuft in die Arbeitsquote.
    let program = super::tests::program(&units("(a*)*b"), "");
    let input = units(&"a".repeat(40));
    let mut left = 100_000_u64;
    let mut spend = |work: u64| {
        left = left.saturating_sub(work);
        left > 0
    };
    let budget = Budget {
        bytes: 1 << 20,
        spend: &mut spend,
    };
    assert_eq!(
        execute(&program, &input[..], 0, false, budget),
        Err(Stop::Stopped)
    );
    // Der Stapel der Rückverfolgung hat eine Grenze.
    let program = super::tests::program(&units("(?:a|b)*c"), "");
    let input = units(&"ab".repeat(5000));
    let mut spend = |_: u64| true;
    let budget = Budget {
        bytes: 4096,
        spend: &mut spend,
    };
    assert_eq!(
        execute(&program, &input[..], 0, false, budget),
        Err(Stop::Memory)
    );
}

#[test]
fn deep_nesting_needs_no_deep_stack() {
    // Wie der Parser: 200.000 Gruppen und Klassen ineinander auf einem
    // Stapel von 1 MiB.
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let depth = 200_000;
            let groups = format!("{}a{}", "(".repeat(depth), ")*".repeat(depth));
            let program = program(&units(&groups), "");
            assert_eq!(program.groups, 200_000);
            let mut left = 10_000_000_u64;
            let mut spend = |work: u64| {
                left = left.saturating_sub(work);
                left > 0
            };
            let budget = Budget {
                bytes: 1 << 26,
                spend: &mut spend,
            };
            let input = units("aaa");
            let found = execute(&program, &input[..], 0, false, budget);
            assert!(
                matches!(found, Ok(Some(_)) | Err(Stop::Stopped)),
                "{found:?}"
            );
            let classes = format!("{}a{}", "[".repeat(depth), "]".repeat(depth));
            let program = self::program(&units(&classes), "v");
            assert_eq!(program.sets.len(), 1);
        })
        .expect("Thread")
        .join()
        .expect("kein Absturz");
}
