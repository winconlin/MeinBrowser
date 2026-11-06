// Eigene, vorübergehende Testanpassung: Der Fehlerzweig der originalen
// Assertions ruft über assert._toString die Funktion String auf, die es noch
// nicht gibt. Die normalen Test262-Harness-Dateien werden vorher
// unverändert ausgewertet; nur diese beiden Assertions werden ersetzt.
// INTERPRETING.md erlaubt das Ersetzen von Harness-Funktionen.
// Der originale _isSameValue-Algorithmus prüft auch NaN und +/-0. Fehler
// werfen einen Wert; sie können keinen erwarteten Engine-Fehlertyp erfüllen.
assert.sameValue = function (actual, expected) {
    if (!assert._isSameValue(actual, expected)) {
        throw "Test262Error: assert.sameValue";
    }
};
assert.notSameValue = function (actual, unexpected) {
    if (assert._isSameValue(actual, unexpected)) {
        throw "Test262Error: assert.notSameValue";
    }
};
