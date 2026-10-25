//! Die Wahl eines Schnitts aus einer Familie nach CSS Fonts 4, Abschnitt
//! „Matching font styles“, Schritt 4: erst `font-width`, dann `font-style`,
//! dann `font-weight` (Editor's Draft, csswg-drafts Commit e1bd391).
//!
//! Die Schnitte sind statisch. Wie die Eigenschaft `font-style` es
//! beschreibt, hat ein aufrechter Schnitt den Italic-Wert 0 und den
//! Oblique-Winkel 0, ein kursiver den Italic-Wert 1 und ein schräger den
//! Winkel 14deg, den `oblique` ohne Winkel bedeutet. Variable Schriften und
//! künstlich schräg gestellte Schnitte fehlen noch; der Schritt, der sie
//! erzeugt, wird übersprungen.

use crate::opentype::FaceStyle;

/// Die gewünschte Lage (`font-style`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slope {
    Normal,
    Italic,
    /// Der Winkel in Grad, positiv im Uhrzeigersinn.
    Oblique(f32),
}

/// Die Lage eines Schnitts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceSlope {
    Normal,
    Italic,
    Oblique,
}

/// Was die Wahl über einen Schnitt wissen muss.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// Die Breite in Prozent.
    pub width: f32,
    pub slope: FaceSlope,
    pub weight: f32,
}

impl Candidate {
    /// Die Werte eines Schnitts aus `OS/2`. `usWidthClass` steht nach der
    /// Tabelle bei `font-width` für 50 % bis 200 %, ein Schnitt mit dem Bit
    /// OBLIQUE gilt als schräg, auch wenn das Bit ITALIC ebenfalls gesetzt
    /// ist.
    pub fn from_style(style: FaceStyle) -> Self {
        let width = match style.width {
            1 => 50.0,
            2 => 62.5,
            3 => 75.0,
            4 => 87.5,
            6 => 112.5,
            7 => 125.0,
            8 => 150.0,
            9 => 200.0,
            _ => 100.0,
        };
        let slope = if style.oblique {
            FaceSlope::Oblique
        } else if style.italic {
            FaceSlope::Italic
        } else {
            FaceSlope::Normal
        };
        Candidate {
            width,
            slope,
            weight: f32::from(style.weight),
        }
    }

    fn value(&self, axis: Axis) -> Option<f32> {
        match (axis, self.slope) {
            (_, FaceSlope::Normal) => Some(0.0),
            (Axis::Italic, FaceSlope::Italic) => Some(1.0),
            (Axis::Oblique, FaceSlope::Oblique) => Some(DEFAULT_OBLIQUE),
            _ => None,
        }
    }
}

/// Der Winkel von `oblique` ohne Angabe.
const DEFAULT_OBLIQUE: f32 = 14.0;

/// Ab diesem Winkel wird schräg vor kursiv gesucht.
const OBLIQUE_THRESHOLD: f32 = 11.0;

/// Der Index des passenden Schnitts; bleiben mehrere übrig, der erste. `None`
/// nur ohne Schnitte.
pub fn best(candidates: &[Candidate], width: f32, slope: Slope, weight: f32) -> Option<usize> {
    let mut set: Vec<(usize, &Candidate)> = candidates.iter().enumerate().collect();
    if let Some(chosen) = choose_width(width, set.iter().map(|(_, face)| face.width)) {
        set.retain(|(_, face)| face.width == chosen);
    }
    let faces: Vec<&Candidate> = set.iter().map(|&(_, face)| face).collect();
    if let Some((axis, chosen)) = choose_slope(&faces, slope) {
        set.retain(|(_, face)| face.value(axis) == Some(chosen));
    }
    if let Some(chosen) = choose_weight(weight, set.iter().map(|(_, face)| face.weight)) {
        set.retain(|(_, face)| face.weight == chosen);
    }
    set.first().map(|&(index, _)| index)
}

/// Der kleinste oder größte Wert, der die Bedingung erfüllt.
fn first(
    values: impl Iterator<Item = f32>,
    keep: impl Fn(f32) -> bool,
    ascending: bool,
) -> Option<f32> {
    let kept = values.filter(|&value| keep(value));
    if ascending {
        kept.min_by(f32::total_cmp)
    } else {
        kept.max_by(f32::total_cmp)
    }
}

/// Schritt 4.1.
fn choose_width(desired: f32, widths: impl Iterator<Item = f32> + Clone) -> Option<f32> {
    if widths.clone().any(|width| width == desired) {
        return Some(desired);
    }
    let below = first(widths.clone(), |width| width < desired, false);
    let above = first(widths, |width| width > desired, true);
    if desired <= 100.0 {
        below.or(above)
    } else {
        above.or(below)
    }
}

/// Schritt 4.3.
fn choose_weight(desired: f32, weights: impl Iterator<Item = f32> + Clone) -> Option<f32> {
    if weights.clone().any(|weight| weight == desired) {
        return Some(desired);
    }
    let below = first(weights.clone(), |weight| weight < desired, false);
    if (400.0..=500.0).contains(&desired) {
        first(
            weights.clone(),
            |weight| weight > desired && weight <= 500.0,
            true,
        )
        .or(below)
        .or_else(|| first(weights, |weight| weight > 500.0, true))
    } else {
        let above = first(weights, |weight| weight > desired, true);
        if desired < 400.0 {
            below.or(above)
        } else {
            above.or(below)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Italic,
    Oblique,
}

/// Eine Grenze eines Suchbereichs.
#[derive(Clone, Copy)]
enum Limit {
    Open(f32),
    Closed(f32),
}

/// Ein Suchschritt: die Werte einer Achse in einem Bereich, auf- oder
/// absteigend.
#[derive(Clone, Copy)]
struct Step {
    axis: Axis,
    low: Limit,
    high: Limit,
    ascending: bool,
}

impl Step {
    fn keeps(&self, value: f32) -> bool {
        let above_low = match self.low {
            Limit::Open(low) => value > low,
            Limit::Closed(low) => value >= low,
        };
        let below_high = match self.high {
            Limit::Open(high) => value < high,
            Limit::Closed(high) => value <= high,
        };
        above_low && below_high
    }
}

const fn step(axis: Axis, low: Limit, high: Limit, ascending: bool) -> Step {
    Step {
        axis,
        low,
        high,
        ascending,
    }
}

/// Schritt 4.2: der gefundene Wert und seine Achse, `None`, wenn keiner
/// passt. Negative Winkel suchen wie positive, mit umgekehrten Vorzeichen
/// aller Werte.
fn choose_slope(set: &[&Candidate], slope: Slope) -> Option<(Axis, f32)> {
    use Axis::{Italic, Oblique};
    use Limit::{Closed, Open};
    const INFINITE: f32 = f32::INFINITY;
    let (sign, steps) = match slope {
        Slope::Normal => (
            1.0,
            vec![
                step(Oblique, Closed(0.0), Open(INFINITE), true),
                step(Italic, Closed(0.0), Open(INFINITE), true),
                step(Oblique, Open(-INFINITE), Open(0.0), false),
                step(Italic, Open(-INFINITE), Open(0.0), false),
            ],
        ),
        Slope::Italic => (
            1.0,
            vec![
                step(Italic, Closed(1.0), Closed(1.0), true),
                step(Italic, Open(1.0), Open(INFINITE), true),
                step(Italic, Open(0.0), Open(1.0), false),
                step(Oblique, Closed(OBLIQUE_THRESHOLD), Open(INFINITE), true),
                step(Oblique, Open(0.0), Open(OBLIQUE_THRESHOLD), false),
                step(Italic, Open(-INFINITE), Closed(0.0), false),
                step(Oblique, Open(-INFINITE), Closed(0.0), false),
            ],
        ),
        Slope::Oblique(angle) => {
            let (sign, angle) = if angle < 0.0 {
                (-1.0, -angle)
            } else {
                (1.0, angle)
            };
            let steps = if angle >= OBLIQUE_THRESHOLD {
                vec![
                    step(Oblique, Closed(angle), Closed(angle), true),
                    step(Oblique, Open(angle), Open(INFINITE), true),
                    step(Oblique, Open(0.0), Open(angle), false),
                    step(Italic, Closed(1.0), Open(INFINITE), true),
                    step(Italic, Open(0.0), Open(1.0), false),
                    step(Oblique, Open(-INFINITE), Closed(0.0), false),
                    step(Italic, Open(-INFINITE), Closed(0.0), false),
                ]
            } else {
                vec![
                    step(Oblique, Closed(angle), Closed(angle), true),
                    step(Oblique, Open(0.0), Open(angle), false),
                    step(Oblique, Open(angle), Open(INFINITE), true),
                    // Wörtlich „less than 1“ und „above 1“: Der Wert 1
                    // selbst kommt hier nicht vor.
                    step(Italic, Open(0.0), Open(1.0), false),
                    step(Italic, Open(1.0), Open(INFINITE), true),
                    step(Oblique, Open(-INFINITE), Closed(0.0), false),
                    step(Italic, Open(-INFINITE), Closed(0.0), false),
                ]
            };
            (sign, steps)
        }
    };
    for step in steps {
        // `0.0 - value` statt `-value`, damit aus 0 nicht -0 wird.
        let values = set
            .iter()
            .filter_map(|face| face.value(step.axis))
            .map(|value| if sign < 0.0 { 0.0 - value } else { value });
        if let Some(found) = first(values, |value| step.keeps(value), step.ascending) {
            let found = if sign < 0.0 { 0.0 - found } else { found };
            return Some((step.axis, found));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(width: f32, slope: FaceSlope, weight: f32) -> Candidate {
        Candidate {
            width,
            slope,
            weight,
        }
    }

    fn weights(weights: &[f32]) -> Vec<Candidate> {
        weights
            .iter()
            .map(|&weight| face(100.0, FaceSlope::Normal, weight))
            .collect()
    }

    /// Welches Gewicht für 100 bis 900 gewählt wird.
    fn chosen_weights(family: &[f32]) -> Vec<f32> {
        let candidates = weights(family);
        (1..=9_u8)
            .map(|step| {
                let index = best(&candidates, 100.0, Slope::Normal, f32::from(step) * 100.0)
                    .expect("ein Schnitt");
                candidates[index].weight
            })
            .collect()
    }

    #[test]
    fn weight_search_directions() {
        // Die Familien der CSSTest-Schriften aus den WPT.
        assert_eq!(
            chosen_weights(&[100.0, 400.0, 700.0, 900.0]),
            [
                100.0, 100.0, 100.0, 400.0, 400.0, 700.0, 700.0, 900.0, 900.0
            ]
        );
        assert_eq!(
            chosen_weights(&[100.0, 500.0]),
            [
                100.0, 100.0, 100.0, 500.0, 500.0, 500.0, 500.0, 500.0, 500.0
            ]
        );
        assert_eq!(
            chosen_weights(&[200.0, 400.0]),
            [
                200.0, 200.0, 200.0, 400.0, 400.0, 400.0, 400.0, 400.0, 400.0
            ]
        );
        assert_eq!(
            chosen_weights(&[200.0, 500.0, 600.0, 900.0]),
            [
                200.0, 200.0, 200.0, 500.0, 500.0, 600.0, 900.0, 900.0, 900.0
            ]
        );
        assert_eq!(
            chosen_weights(&[300.0, 500.0, 800.0, 900.0]),
            [
                300.0, 300.0, 300.0, 500.0, 500.0, 800.0, 800.0, 800.0, 900.0
            ]
        );
        assert_eq!(
            chosen_weights(&[400.0, 700.0]),
            [
                400.0, 400.0, 400.0, 400.0, 400.0, 700.0, 700.0, 700.0, 700.0
            ]
        );
        // Zwischen 400 und 500 erst aufwärts bis 500, dann abwärts, dann
        // über 500.
        let candidates = weights(&[300.0, 450.0, 600.0]);
        let chosen = |weight| {
            candidates[best(&candidates, 100.0, Slope::Normal, weight).expect("Schnitt")].weight
        };
        assert_eq!(chosen(420.0), 450.0);
        assert_eq!(chosen(470.0), 450.0);
        let candidates = weights(&[300.0, 600.0]);
        assert_eq!(
            candidates[best(&candidates, 100.0, Slope::Normal, 450.0).expect("Schnitt")].weight,
            300.0
        );
        let candidates = weights(&[600.0, 700.0]);
        assert_eq!(
            candidates[best(&candidates, 100.0, Slope::Normal, 450.0).expect("Schnitt")].weight,
            600.0
        );
    }

    #[test]
    fn width_comes_before_weight() {
        let candidates = [
            face(75.0, FaceSlope::Normal, 400.0),
            face(87.5, FaceSlope::Normal, 700.0),
            face(112.5, FaceSlope::Normal, 400.0),
            face(150.0, FaceSlope::Normal, 400.0),
        ];
        // Bis 100 % erst schmaler, absteigend, dann breiter.
        assert_eq!(best(&candidates, 100.0, Slope::Normal, 400.0), Some(1));
        assert_eq!(best(&candidates, 90.0, Slope::Normal, 400.0), Some(1));
        assert_eq!(best(&candidates, 62.5, Slope::Normal, 400.0), Some(0));
        // Über 100 % erst breiter, aufsteigend, dann schmaler.
        assert_eq!(best(&candidates, 110.0, Slope::Normal, 400.0), Some(2));
        assert_eq!(best(&candidates, 125.0, Slope::Normal, 400.0), Some(3));
        assert_eq!(best(&candidates, 200.0, Slope::Normal, 400.0), Some(3));
        assert_eq!(best(&candidates, 150.0, Slope::Normal, 700.0), Some(3));
    }

    fn slopes(family: &[FaceSlope], slope: Slope) -> FaceSlope {
        let candidates: Vec<Candidate> = family
            .iter()
            .map(|&slope| face(100.0, slope, 400.0))
            .collect();
        candidates[best(&candidates, 100.0, slope, 400.0).expect("ein Schnitt")].slope
    }

    #[test]
    fn style_search_order() {
        use FaceSlope::{Italic, Normal, Oblique};
        let all = [Normal, Italic, Oblique];
        assert_eq!(slopes(&all, Slope::Normal), Normal);
        assert_eq!(slopes(&all, Slope::Italic), Italic);
        assert_eq!(slopes(&all, Slope::Oblique(14.0)), Oblique);
        assert_eq!(slopes(&all, Slope::Oblique(5.0)), Oblique);
        assert_eq!(slopes(&all, Slope::Oblique(0.0)), Normal);
        assert_eq!(slopes(&all, Slope::Oblique(-14.0)), Normal);
        // Ohne aufrechten Schnitt: normal sucht erst schräg, dann kursiv.
        assert_eq!(slopes(&[Italic, Oblique], Slope::Normal), Oblique);
        assert_eq!(slopes(&[Italic], Slope::Normal), Italic);
        // Kursiv: dann schräg, dann aufrecht.
        assert_eq!(slopes(&[Normal, Oblique], Slope::Italic), Oblique);
        assert_eq!(slopes(&[Normal], Slope::Italic), Normal);
        // Schräg ab 11deg: dann kursiv, dann aufrecht.
        assert_eq!(slopes(&[Normal, Italic], Slope::Oblique(20.0)), Italic);
        assert_eq!(slopes(&[Normal, Oblique], Slope::Oblique(20.0)), Oblique);
        // Unter 11deg gilt der Wert 1 der kursiven Schnitte wörtlich weder
        // als „less than 1“ noch als „above 1“, also aufrecht.
        assert_eq!(slopes(&[Normal, Italic], Slope::Oblique(5.0)), Normal);
        // Passt kein Wert, bleiben alle Schnitte.
        assert_eq!(slopes(&[Italic], Slope::Oblique(5.0)), Italic);
        // Negative Winkel mit umgekehrten Vorzeichen: Der aufrechte Schnitt
        // hat 0, kursiv und schräg zählen als negativ und kommen danach.
        assert_eq!(slopes(&[Italic, Oblique], Slope::Oblique(-20.0)), Oblique);
        assert_eq!(slopes(&[Italic], Slope::Oblique(-20.0)), Italic);
    }

    #[test]
    fn style_comes_before_weight() {
        let candidates = [
            face(100.0, FaceSlope::Normal, 400.0),
            face(100.0, FaceSlope::Normal, 700.0),
            face(100.0, FaceSlope::Italic, 400.0),
        ];
        assert_eq!(best(&candidates, 100.0, Slope::Italic, 700.0), Some(2));
        assert_eq!(best(&candidates, 100.0, Slope::Normal, 700.0), Some(1));
        assert_eq!(best(&[], 100.0, Slope::Normal, 400.0), None);
        // Gleichstand: der erste.
        let twins = [candidates[0], candidates[0]];
        assert_eq!(best(&twins, 100.0, Slope::Normal, 400.0), Some(0));
    }

    #[test]
    fn values_from_os2() {
        let style = |weight, width, italic, oblique| FaceStyle {
            weight,
            width,
            italic,
            oblique,
        };
        assert_eq!(
            Candidate::from_style(style(700, 5, true, false)),
            face(100.0, FaceSlope::Italic, 700.0)
        );
        assert_eq!(
            Candidate::from_style(style(400, 3, true, true)),
            face(75.0, FaceSlope::Oblique, 400.0)
        );
        assert_eq!(
            Candidate::from_style(style(100, 9, false, false)).width,
            200.0
        );
        assert_eq!(
            Candidate::from_style(style(400, 1, false, false)).width,
            50.0
        );
    }
}
