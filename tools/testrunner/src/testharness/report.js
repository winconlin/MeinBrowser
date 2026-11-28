// Meldet die Ergebnisse von testharness.js an den Läufer in
// tools/testrunner. Der Läufer lädt diese Datei an Stelle von
// /resources/testharnessreport.js, gleich nach testharness.js; `mbWpt`
// stellt sein Host bereit.
add_result_callback(function (test) {
    mbWpt.result(test.name, test.status, test.message);
});
add_completion_callback(function (tests, harness_status) {
    for (var i = 0; i < tests.length; i++) {
        mbWpt.last(tests[i].name, tests[i].status, tests[i].message);
    }
    mbWpt.complete(harness_status.status, harness_status.message);
});
