//! Der Bereich, über den der Viewport scrollt: sein *scrollable overflow*
//! nach CSS Overflow 3, Abschnitt 2.2.
//!
//! Die Eigenschaft `overflow` fehlt noch, kein Kasten beschneidet seinen
//! Inhalt. Damit ist der Bereich die Vereinigung aus dem Viewport und
//!
//! - den Rahmenbereichen (*border box*) aller Kästen mit Fläche, auch den
//!   Teilen geteilter Inline-Kästen;
//! - den Zeilen, die ein Blockcontainer direkt enthält. Eine Zeile zählt
//!   mit ihrem Inhalt, auch wo er über sie hinausragt, etwa ein zu langes
//!   Wort; sonst ließe er sich nicht erreichen. Zeilen ohne Fläche zählen
//!   wie Rahmen ohne Fläche nicht.
//!
//! Was ganz links oder ganz oberhalb des Ursprungs liegt, lässt sich nicht
//! erreichen (*unreachable scrollable overflow region*) und zählt samt
//! seinem Inhalt nicht; ein Kasten, der genau am Ursprung endet, etwa einer
//! ohne Höhe am oberen Rand, zählt mit seinem Inhalt. Außenabstände zählen
//! nicht; die Spezifikation stellt das bei Blöcken frei.

use crate::fragment::{BoxFragment, Fragment, Rect};

/// Ein Kasten oder Kinder, die noch zu besuchen sind, mit dem Ursprung
/// ihres Koordinatensystems.
enum Entry<'a> {
    Box(&'a BoxFragment, (f64, f64)),
    Children(&'a [Fragment], (f64, f64)),
}

/// Breite und Höhe des Bereichs ab dem Ursprung, mindestens `width` ×
/// `height`. Der Baum wird mit einem eigenen Stapel durchlaufen (RS-8).
pub(crate) fn scroll_size(root: Option<&BoxFragment>, width: f64, height: f64) -> (f64, f64) {
    let (mut right, mut bottom) = (width, height);
    // Ob ein Rechteck erreichbar ist; mit Fläche dehnt es den Bereich.
    let mut reach = |rect: Rect| {
        let (end_x, end_y) = (rect.x + rect.width, rect.y + rect.height);
        if !(end_x >= 0.0 && end_y >= 0.0 && end_x.is_finite() && end_y.is_finite()) {
            return false;
        }
        if rect.width > 0.0 && rect.height > 0.0 {
            right = right.max(end_x);
            bottom = bottom.max(end_y);
        }
        true
    };
    let mut stack: Vec<Entry<'_>> = root
        .map(|root| Entry::Box(root, (0.0, 0.0)))
        .into_iter()
        .collect();
    while let Some(entry) = stack.pop() {
        match entry {
            Entry::Box(fragment, (x, y)) => {
                if !reach(fragment.rect.translate(x, y)) {
                    continue;
                }
                let content = fragment.content_rect();
                let inner = (x + content.x, y + content.y);
                if let Some(lines) = fragment.lines {
                    reach(lines.translate(inner.0, inner.1));
                }
                stack.push(Entry::Children(&fragment.children, inner));
            }
            Entry::Children(children, origin) => {
                for child in children {
                    match child {
                        Fragment::Box(child) => stack.push(Entry::Box(child, origin)),
                        // Inline-Kästen haben kein eigenes Koordinatensystem.
                        Fragment::Inline(inline) => {
                            if reach(inline.rect.translate(origin.0, origin.1)) {
                                stack.push(Entry::Children(&inline.children, origin));
                            }
                        }
                        Fragment::Text(_) | Fragment::Image(_) => {}
                    }
                }
            }
        }
    }
    (right, bottom)
}
