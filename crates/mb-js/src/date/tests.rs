use super::*;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn parsed(text: &str) -> f64 {
    parse(&units(text), TimeZone::Utc)
}

#[test]
fn days_match_counting_and_round_trip() {
    let leap = |year: i64| year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    // Tage bis zum 1. Januar -1200 durch Zählen der Jahre, dann Jahr für
    // Jahr und Monat für Monat weiter.
    let mut start = 0i64;
    for year in -1200..1970 {
        start -= if leap(year) { 366 } else { 365 };
    }
    for year in -1200..2800 {
        let mut first = start;
        for month in 1..=12u32 {
            let length = days_in_month(year, i64::from(month));
            let last = u32::try_from(length).expect("Tage");
            for day in [1, 15, last] {
                let days = days_from_civil(year, month, day);
                assert_eq!(days, first + i64::from(day) - 1, "{year}-{month}-{day}");
                assert_eq!(civil_from_days(days), (year, month, day));
            }
            first += length;
        }
        start += if leap(year) { 366 } else { 365 };
    }
    // Über den ganzen Bereich der Zeitwerte und darüber hinaus.
    let mut days = -100_000_002i64;
    while days <= 100_000_002 {
        let (year, month, day) = civil_from_days(days);
        assert_eq!(days_from_civil(year, month, day), days);
        assert!(civil_from_days(days + 1) > (year, month, day), "{days}");
        days += 997;
    }
}

#[test]
fn fields_of_known_time_values() {
    let epoch = fields(0.0);
    assert_eq!(
        epoch,
        Fields {
            year: 1970,
            month: 0,
            date: 1,
            weekday: 4,
            hour: 0,
            minute: 0,
            second: 0,
            millisecond: 0,
        }
    );
    let before = fields(-1.0);
    assert_eq!(
        (before.year, before.month, before.date, before.weekday),
        (1969, 11, 31, 3)
    );
    assert_eq!(
        (
            before.hour,
            before.minute,
            before.second,
            before.millisecond
        ),
        (23, 59, 59, 999)
    );
    let last = fields(MAX_TIME);
    assert_eq!((last.year, last.month, last.date), (275_760, 8, 13));
    let first = fields(-MAX_TIME);
    assert_eq!((first.year, first.month, first.date), (-271_821, 3, 20));
    assert_eq!(split(-1.0), (-1, 86_399_999));
    assert_eq!(time_within_day(-1.0), 86_399_999.0);
}

#[test]
fn make_day_and_make_time_follow_the_specification() {
    let utc_value = |year: f64, month: f64, date: f64| {
        time_clip(make_date(
            make_day(year, month, date),
            make_time(0.0, 0.0, 0.0, 0.0),
        ))
    };
    // Die Werte aus Test262 (built-ins/Date/UTC/overflow-make-day.js).
    assert_eq!(utc_value(2016.0, 12.0, 1.0), 1_483_228_800_000.0);
    assert_eq!(utc_value(2016.0, 13.0, 1.0), 1_485_907_200_000.0);
    assert_eq!(utc_value(2016.0, 144.0, 1.0), 1_830_297_600_000.0);
    assert_eq!(utc_value(2016.0, 0.0, 33.0), 1_454_371_200_000.0);
    assert_eq!(utc_value(2016.0, 2.0, -27.0), 1_454_371_200_000.0);
    assert_eq!(utc_value(-1.0, 0.0, 1.0), -62_198_755_200_000.0);
    assert_eq!(utc_value(2016.0, -1.0, 1.0), utc_value(2015.0, 11.0, 1.0));
    // MakeTime rechnet in der Reihenfolge der Spezifikation
    // (built-ins/Date/UTC/fp-evaluation-order.js).
    let time = make_time(80_063_993_375.0, 29.0, 1.0, -288_230_376_151_711_740.0);
    assert_eq!(make_date(make_day(1970.0, 0.0, 1.0), time), 29_312.0);
    assert!(make_day(f64::INFINITY, 0.0, 1.0).is_nan());
    assert!(make_day(2e13, 0.0, 1.0).is_nan());
    assert!(make_time(0.0, f64::NAN, 0.0, 0.0).is_nan());
    // Ein Tag in einem Monat weit über dem Bereich, mit Tagen zurück.
    assert_eq!(
        utc_value(275_761.0, 0.0, -200.0),
        utc_value(275_760.0, 11.0, 31.0 - 200.0)
    );
    assert!(utc_value(275_760.0, 8.0, 14.0).is_nan());
    assert_eq!(time_clip(-0.5), 0.0);
    assert!(time_clip(-0.5).is_sign_positive());
    assert_eq!(make_full_year(99.9), 1999.0);
    assert_eq!(make_full_year(-0.5), 1900.0);
    assert_eq!(make_full_year(100.0), 100.0);
}

#[test]
fn twelfths_of_large_months_are_exact() {
    // Für Monate über 2¹⁰⁰ rechnet floor_twelfth mit der Division von
    // Numbers; der Vergleich rundet floor(m / 12) aus ganzen Zahlen.
    for exponent in 100..126 {
        for mantissa in [
            1u64,
            3,
            5,
            7,
            11,
            (1 << 53) - 1,
            (1 << 52) + 1,
            0x15_5555_5555_5555,
        ] {
            let month = mantissa as f64 * 2f64.powi(exponent - 53);
            let whole = month as i128;
            for value in [month, -month] {
                let exact = (if value < 0.0 { -whole } else { whole }).div_euclid(12) as f64;
                assert_eq!(floor_twelfth(value), exact, "{value}");
            }
        }
    }
    assert_eq!(floor_twelfth(-1.0), -1.0);
    assert_eq!(floor_twelfth(11.0), 0.0);
    assert_eq!(floor_twelfth(12.0), 1.0);
    // Ein Jahr weit im Minus, das ein Monat weit im Plus ausgleicht.
    let year = 2f64.powi(60);
    assert_eq!(make_day(-year, 12.0 * year, 1.0), make_day(0.0, 0.0, 1.0));
}

#[test]
fn offsets_follow_the_grammar() {
    let offset = |text: &str| TimeZone::from_identifier(text);
    assert_eq!(offset("UTC"), Some(TimeZone::Utc));
    assert_eq!(offset("+01"), Some(TimeZone::Offset(3_600_000_000_000)));
    assert_eq!(
        offset("-05:30"),
        Some(TimeZone::Offset(-19_800_000_000_000))
    );
    assert_eq!(offset("+0530"), Some(TimeZone::Offset(19_800_000_000_000)));
    assert_eq!(offset("+00:00:01.5"), Some(TimeZone::Offset(1_500_000_000)));
    assert_eq!(
        offset("+000001,000000001"),
        Some(TimeZone::Offset(1_000_000_001))
    );
    for invalid in [
        "",
        "+",
        "+1",
        "+24",
        "+01:60",
        "+01:0000",
        "+0100:00",
        "+01:00:",
        "+01:00:00.",
        "+01:00:00.1234567890",
        "01:00",
        "utc",
        "+01:00 ",
        "Europe/Berlin",
    ] {
        assert_eq!(offset(invalid), None, "{invalid:?}");
    }
    let zone = TimeZone::Offset(-1_500_000);
    // Der Abstand wird zur Null hin auf Millisekunden gekürzt.
    assert_eq!(local_time(0.0, zone), -1.0);
    assert_eq!(utc(-1.0, zone), 0.0);
}

#[test]
fn texts_of_the_epoch() {
    let berlin = TimeZone::from_identifier("+01:00").expect("Abstand");
    let india = TimeZone::from_identifier("+05:30").expect("Abstand");
    assert_eq!(
        to_date_string(0.0, TimeZone::Utc),
        "Thu Jan 01 1970 00:00:00 GMT+0000 (Coordinated Universal Time)"
    );
    assert_eq!(
        to_date_string(0.0, india),
        "Thu Jan 01 1970 05:30:00 GMT+0530"
    );
    let west = TimeZone::from_identifier("-08:00").expect("Abstand");
    assert_eq!(
        to_date_string(0.0, west),
        "Wed Dec 31 1969 16:00:00 GMT-0800"
    );
    assert_eq!(to_date_string(f64::NAN, berlin), "Invalid Date");
    assert_eq!(to_date_only_string(0.0, west), "Wed Dec 31 1969");
    assert_eq!(to_time_only_string(0.0, berlin), "01:00:00 GMT+0100");
    assert_eq!(to_utc_string(0.0), "Thu, 01 Jan 1970 00:00:00 GMT");
    assert_eq!(to_iso_string(0.0), "1970-01-01T00:00:00.000Z");
    assert_eq!(to_iso_string(-1.0), "1969-12-31T23:59:59.999Z");
    assert_eq!(to_iso_string(MAX_TIME), "+275760-09-13T00:00:00.000Z");
    assert_eq!(to_iso_string(-MAX_TIME), "-271821-04-20T00:00:00.000Z");
    assert_eq!(
        to_iso_string(-62_198_755_200_000.0),
        "-000001-01-01T00:00:00.000Z"
    );
    assert_eq!(to_utc_string(-MAX_TIME), "Tue, 20 Apr -271821 00:00:00 GMT");
    let afternoon = 1_577_977_445_000.0;
    assert_eq!(
        to_locale_string(afternoon, TimeZone::Utc, LocaleParts::DateAndTime),
        "1/2/2020, 3:04:05 PM"
    );
    assert_eq!(
        to_locale_string(0.0, TimeZone::Utc, LocaleParts::Time),
        "12:00:00 AM"
    );
    assert_eq!(to_locale_string(0.0, west, LocaleParts::Date), "12/31/1969");
}

#[test]
fn the_date_time_string_format() {
    assert_eq!(parsed("1970"), 0.0);
    assert_eq!(parsed("1970-01"), 0.0);
    assert_eq!(parsed("1970-01-01"), 0.0);
    assert_eq!(parsed("1970-01-01T00:00Z"), 0.0);
    assert_eq!(parsed("1970-01-01T00:00:00.001Z"), 1.0);
    assert_eq!(parsed("1970-01-01T01:00+01:00"), 0.0);
    assert_eq!(parsed("1969-12-31T24:00:00.000Z"), 0.0);
    assert_eq!(parsed("+275760-09-13T00:00:00.000Z"), MAX_TIME);
    assert_eq!(parsed("-271821-04-20T00:00:00.000Z"), -MAX_TIME);
    let zone = TimeZone::from_identifier("-02:00").expect("Abstand");
    // Ein Datum allein gilt als UTC, mit Uhrzeit als Ortszeit.
    assert_eq!(parse(&units("1970-01-01"), zone), 0.0);
    assert_eq!(parse(&units("1970-01-01T00:00"), zone), 7_200_000.0);
    for invalid in [
        "-000000-03-31T00:45Z",
        "-000000-03-31T01:45",
        "+275760-09-13T00:00:00.001Z",
        "-271821-04-19T23:59:59.999Z",
        "2019-02-29",
        "2020-13-01",
        "2020-00-01",
        "2020-01-32",
        "2020-01-01T24:01Z",
        "2020-01-01T23:60Z",
        "2020-01-01T23:59:60Z",
        "2020-01-01T00:00+24:00",
    ] {
        assert!(parsed(invalid).is_nan(), "{invalid}");
    }
    assert_eq!(parsed("2020-02-29"), 1_582_934_400_000.0);
}

#[test]
fn other_forms_like_browsers() {
    let new_year = 1_577_836_800_000.0;
    for text in [
        "Wed Jan 01 2020 00:00:00 GMT+0000 (Coordinated Universal Time)",
        "Wed, 01 Jan 2020 00:00:00 GMT",
        "Jan 1 2020",
        "Jan 1, 2020",
        "January 1, 2020 00:00:00 UTC",
        "1 January 2020",
        "Wednesday, January 1, 2020",
        "2020 Jan 1",
        "1/1/2020",
        "01/01/2020 12:00:00 AM",
        "2020/1/1",
        "2020-1-1",
        "2020-01-01 00:00:00",
        "2020-01-01t00:00:00",
        "2020-01-01T00:00:00.0000Z",
        "2020-01-01T01:00:00+0100",
        "Dec 31 2019 19:00:00 EST",
        "Dec 31 2019 16:00 PST",
        "Jan 1 20",
        "  Jan 1 2020  ",
        "Jan 1 2020 (ein (geschachtelter) Kommentar)",
        "1.1.2020",
    ] {
        assert_eq!(parsed(text), new_year, "{text}");
    }
    assert_eq!(parsed("Jan 1 1950"), -631_152_000_000.0);
    assert_eq!(parsed("Jan 1 50"), -631_152_000_000.0);
    assert_eq!(parsed("Jan 1 2020 1:02:03.4 PM"), new_year + 46_923_400.0);
    assert_eq!(parsed("Tue Apr 20 -271821 00:00:00 GMT+0000"), -MAX_TIME);
    assert_eq!(parsed("Jan 1 2020 GMT-01:30"), new_year + 5_400_000.0);
    for invalid in [
        "",
        "Jan",
        "Jan 2020",
        "foo",
        "Jan 32 2020",
        "Feb 30 2020",
        "13/1/2020",
        "1/1/2020 25:00",
        "1/1/2020 13:00 PM",
        "Jan 1 2020 1:2",
        "Jan Feb 1 2020",
        "1/1/2020/1",
        "Jan 1 2020 GMT GMT",
        "1234567890 Jan 1",
    ] {
        assert!(parsed(invalid).is_nan(), "{invalid:?}");
    }
    let zone = TimeZone::from_identifier("+09:00").expect("Abstand");
    // Ohne Zeitzone gilt die Ortszeit.
    assert_eq!(parse(&units("Jan 1 2020 09:00"), zone), new_year);
}

#[test]
fn texts_read_back() {
    let zones = [
        TimeZone::Utc,
        TimeZone::Offset(19_800_000_000_000),
        TimeZone::Offset(-28_800_000_000_000),
    ];
    let mut value = -MAX_TIME;
    while value <= MAX_TIME {
        let seconds = value - value.rem_euclid(1000.0);
        for zone in zones {
            let text = |text: String| parse(&units(&text), zone);
            assert_eq!(text(to_iso_string(value)), value, "{value}");
            if seconds.abs() <= MAX_TIME {
                assert_eq!(text(to_utc_string(value)), seconds, "{value}");
                let local = text(to_date_string(value, zone));
                // Nur am Rand des Bereichs liegt die Ortszeit außerhalb.
                if local.is_finite() {
                    assert_eq!(local, seconds, "{value}");
                }
            }
        }
        value += 7_777_777_777_777.0;
    }
}
