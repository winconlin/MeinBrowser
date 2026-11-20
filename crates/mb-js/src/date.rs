//! Zeitwerte für `Date` (Abschnitt 21.4.1): die Rechnung mit Tagen, Jahren
//! und Uhrzeiten, die Zeitzone, die Texte der Methoden von
//! %Date.prototype% und das Lesen von Datumstexten für `Date.parse`.
//!
//! Ein Zeitwert ist eine ganze Zahl von Millisekunden seit dem 1. Januar
//! 1970 UTC oder NaN, nach TimeClip höchstens 8,64 · 10¹⁵ im Betrag.
//! Kalenderfelder rechnet das Modul mit ganzen Zahlen aus der Zahl der Tage,
//! nach dem Verfahren von Howard Hinnant (`days_from_civil` und
//! `civil_from_days`), statt Jahr für Jahr zu zählen. MakeTime und MakeDate
//! rechnen dagegen mit Gleitkommazahlen in der Reihenfolge der
//! Spezifikation, denn ihre Rundung ist Teil des Ergebnisses.
//!
//! Zeitzonen sind UTC oder ein fester Abstand zu UTC, wie ihn
//! IsTimeZoneOffsetString (Abschnitt 21.4.1.33.1) beschreibt. Benannte
//! Zeitzonen mit Sommerzeit brauchen die IANA-Zeitzonendatenbank und
//! kommen mit ihr. Welche Zeitzone und welche Uhr gelten, bestimmt der
//! Host ([`crate::host::Host::time_zone`] und
//! [`crate::host::Host::current_time`]).

#![forbid(unsafe_code)]

use std::fmt::Write as _;

/// Millisekunden eines Tages (msPerDay, Abschnitt 21.4.1.2).
const MS_PER_DAY: i64 = 86_400_000;
const MS_PER_HOUR: i64 = 3_600_000;
const MS_PER_MINUTE: i64 = 60_000;
const MS_PER_SECOND: i64 = 1000;
/// Nanosekunden einer Millisekunde.
const NS_PER_MS: i64 = 1_000_000;

/// Der größte Betrag eines Zeitwerts (Abschnitt 21.4.1.1).
pub const MAX_TIME: f64 = 8.64e15;

/// Über diesem Betrag des Jahres gibt [`make_day`] NaN, eine Festlegung.
/// Bis dahin gibt es für jeden Tag einen ganzzahligen Zeitwert, wie
/// Schritt 8 von MakeDay ihn sucht, und die Zahl der Tage ist als Number
/// genau; ab etwa 1,9 · 10¹³ Jahren liegen benachbarte Numbers weiter als
/// einen Tag auseinander, und für manche Tage gibt es keinen.
const YEAR_LIMIT: f64 = 1e13;

/// Eine Zeitzone: UTC oder ein fester Abstand zu UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeZone {
    /// *"UTC"*, die Zeitzone, die jede Implementierung haben muss
    /// (Abschnitt 21.4.1.19).
    Utc,
    /// Ein fester Abstand in Nanosekunden, positiv östlich von UTC, wie
    /// ParseTimeZoneOffsetString (Abschnitt 21.4.1.33.2) ihn liefert. Der
    /// Betrag liegt unter einem Tag.
    Offset(i64),
}

impl TimeZone {
    /// Die Zeitzone zu einem Bezeichner: *"UTC"* oder ein Text nach
    /// |UTCOffset| (Abschnitt 21.4.1.33), etwa *"+01:00"* oder *"-0530"*.
    /// Andere Bezeichner kennt die Engine noch nicht.
    pub fn from_identifier(identifier: &str) -> Option<TimeZone> {
        if identifier == "UTC" {
            return Some(TimeZone::Utc);
        }
        let units: Vec<u16> = identifier.encode_utf16().collect();
        parse_offset(&units).map(TimeZone::Offset)
    }

    /// offsetMilliseconds aus LocalTime, UTC und TimeZoneString: der
    /// Abstand zur Null hin auf Millisekunden gekürzt.
    fn offset_milliseconds(self) -> i64 {
        match self {
            TimeZone::Utc => 0,
            TimeZone::Offset(nanoseconds) => nanoseconds / NS_PER_MS,
        }
    }
}

/// ParseTimeZoneOffsetString (Abschnitt 21.4.1.33.2) für einen Text nach
/// |UTCOffset|, `None`, wenn er nicht passt. Die Form mit `:` und die ohne
/// lassen sich nicht mischen; einen Bruchteil gibt es nur nach Sekunden.
fn parse_offset(units: &[u16]) -> Option<i64> {
    let mut cursor = Cursor { units, position: 0 };
    let sign = match cursor.next()? {
        PLUS => 1,
        MINUS => -1,
        _ => return None,
    };
    let hours = cursor.two_digits()?;
    if hours > 23 {
        return None;
    }
    let mut minutes = 0;
    let mut seconds = 0;
    let mut nanoseconds = 0;
    if !cursor.at_end() {
        let extended = cursor.eat(COLON);
        minutes = cursor.two_digits()?;
        if minutes > 59 {
            return None;
        }
        if !cursor.at_end() {
            if extended && !cursor.eat(COLON) {
                return None;
            }
            seconds = cursor.two_digits()?;
            if seconds > 59 {
                return None;
            }
            if cursor.eat(DOT) || cursor.eat(COMMA) {
                let mut digits = 0;
                while let Some(digit) = cursor.digit() {
                    if digits == 9 {
                        return None;
                    }
                    nanoseconds = nanoseconds * 10 + digit;
                    digits += 1;
                }
                if digits == 0 {
                    return None;
                }
                for _ in digits..9 {
                    nanoseconds *= 10;
                }
            }
        }
    }
    if !cursor.at_end() {
        return None;
    }
    // Höchstens 23:59:59.999999999, weit unter i64::MAX.
    let total = ((hours * 60 + minutes) * 60 + seconds) * 1_000_000_000 + nanoseconds;
    Some(sign * total)
}

// Zeitwerte

/// TimeClip (Abschnitt 21.4.1.31).
pub(crate) fn time_clip(time: f64) -> f64 {
    if !time.is_finite() || time.abs() > MAX_TIME {
        return f64::NAN;
    }
    integer(time)
}

/// ToIntegerOrInfinity für eine endliche Zahl als Number: zur Null hin
/// gekürzt, -0 als +0.
fn integer(value: f64) -> f64 {
    value.trunc() + 0.0
}

/// MakeTime (Abschnitt 21.4.1.27) in der Reihenfolge der Spezifikation.
pub(crate) fn make_time(hour: f64, minute: f64, second: f64, millisecond: f64) -> f64 {
    if !(hour.is_finite() && minute.is_finite() && second.is_finite() && millisecond.is_finite()) {
        return f64::NAN;
    }
    ((integer(hour) * 3_600_000.0 + integer(minute) * 60_000.0) + integer(second) * 1000.0)
        + integer(millisecond)
}

/// MakeDay (Abschnitt 21.4.1.28). Über [`YEAR_LIMIT`] Jahre NaN.
pub(crate) fn make_day(year: f64, month: f64, date: f64) -> f64 {
    if !(year.is_finite() && month.is_finite() && date.is_finite()) {
        return f64::NAN;
    }
    let month = integer(month);
    let balanced_year = integer(year) + floor_twelfth(month);
    if !balanced_year.is_finite() || balanced_year.abs() > YEAR_LIMIT {
        return f64::NAN;
    }
    // Ganze Zahlen bis 10¹³ sind als i64 genau; der Rest liegt in 0 bis 11.
    let (Some(year), Some(month)) = (
        whole(balanced_year),
        whole(month.rem_euclid(12.0)).and_then(|month| u32::try_from(month).ok()),
    ) else {
        return f64::NAN;
    };
    let days = days_from_civil(year, month + 1, 1);
    // Unter 4 · 10¹⁵ Tage, als Number genau.
    days as f64 + integer(date) - 1.0
}

/// 𝔽(floor(ℝ(`month`) / 12)) für eine endliche ganze Zahl. Bis 2¹⁰⁰ mit
/// ganzen Zahlen. Darüber ist `month` = m · 2ᵉ mit e ≥ 47, und die
/// gerundete Division hat dasselbe Ergebnis wie das Runden von
/// floor(month / 12): Der Abstand der beiden ist kleiner als eins, und
/// zwischen ihnen liegt weder eine Number noch eine Mitte zwischen zwei
/// Numbers, denn diese sind dort Vielfache von 2⁴³, und 3 · floor(month /
/// 12) = month - r mit r in 1 bis 2 ist keines.
fn floor_twelfth(month: f64) -> f64 {
    if month.abs() < 2f64.powi(100) {
        // Eine ganze Zahl unter 2¹⁰⁰ passt genau in i128.
        let whole = month as i128;
        whole.div_euclid(12) as f64
    } else {
        (month / 12.0).floor()
    }
}

/// Eine endliche ganze Zahl unter 2⁶³ als i64.
fn whole(value: f64) -> Option<i64> {
    if value.is_finite() && value.trunc() == value && value.abs() < 9.2e18 {
        // Ganz und im Bereich: Die Umwandlung ist genau.
        Some(value as i64)
    } else {
        None
    }
}

/// MakeDate (Abschnitt 21.4.1.29).
pub(crate) fn make_date(day: f64, time: f64) -> f64 {
    if !day.is_finite() || !time.is_finite() {
        return f64::NAN;
    }
    let value = day * 86_400_000.0 + time;
    if value.is_finite() { value } else { f64::NAN }
}

/// MakeFullYear (Abschnitt 21.4.1.30).
pub(crate) fn make_full_year(year: f64) -> f64 {
    if !year.is_finite() {
        return f64::NAN;
    }
    let truncated = integer(year);
    if (0.0..=99.0).contains(&truncated) {
        1900.0 + truncated
    } else {
        truncated
    }
}

/// LocalTime (Abschnitt 21.4.1.25) für einen endlichen Zeitwert.
pub(crate) fn local_time(tv: f64, zone: TimeZone) -> f64 {
    tv + zone.offset_milliseconds() as f64
}

/// UTC (Abschnitt 21.4.1.26). Mit festem Abstand gibt es weder
/// doppelte noch übersprungene Ortszeiten.
pub(crate) fn utc(t: f64, zone: TimeZone) -> f64 {
    if !t.is_finite() {
        return f64::NAN;
    }
    t - zone.offset_milliseconds() as f64
}

/// Tage seit dem 1. Januar 1970 für Jahr, Monat (1 bis 12) und Tag des
/// proleptischen gregorianischen Kalenders. Für Jahre bis 10¹³ im Betrag
/// bleiben alle Zwischenwerte weit unter i64::MAX.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Jahr, Monat (1 bis 12) und Tag zu einer Zahl von Tagen seit dem
/// 1. Januar 1970.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    // Tag und Monat liegen in 1 bis 31 und 1 bis 12.
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

/// Die Kalenderfelder eines Zeitwerts: YearFromTime, MonthFromTime,
/// DateFromTime, WeekDay und die Felder der Uhrzeit (Abschnitte 21.4.1.7
/// bis 21.4.1.16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fields {
    pub(crate) year: i64,
    /// 0 für Januar bis 11 für Dezember.
    pub(crate) month: u32,
    /// Der Tag im Monat, 1 bis 31.
    pub(crate) date: u32,
    /// 0 für Sonntag bis 6 für Samstag.
    pub(crate) weekday: u32,
    pub(crate) hour: u32,
    pub(crate) minute: u32,
    pub(crate) second: u32,
    pub(crate) millisecond: u32,
}

/// Day (Abschnitt 21.4.1.3) und TimeWithinDay (Abschnitt 21.4.1.4) eines
/// endlichen, ganzzahligen Zeitwerts unter 2⁵³.
fn split(t: f64) -> (i64, i64) {
    let milliseconds = whole(t).unwrap_or(0);
    (
        milliseconds.div_euclid(MS_PER_DAY),
        milliseconds.rem_euclid(MS_PER_DAY),
    )
}

/// TimeWithinDay (Abschnitt 21.4.1.4) als Number.
pub(crate) fn time_within_day(t: f64) -> f64 {
    split(t).1 as f64
}

/// Die Felder eines endlichen Zeitwerts, auch knapp außerhalb des Bereichs,
/// wie ihn LocalTime liefert.
pub(crate) fn fields(t: f64) -> Fields {
    let (days, within) = split(t);
    let (year, month, date) = civil_from_days(days);
    // Die Reste liegen unter 7, 24, 60 und 1000.
    let part = |value: i64| u32::try_from(value).unwrap_or(0);
    Fields {
        year,
        month: month - 1,
        date,
        weekday: part((days + 4).rem_euclid(7)),
        hour: part(within / MS_PER_HOUR),
        minute: part(within / MS_PER_MINUTE % 60),
        second: part(within / MS_PER_SECOND % 60),
        millisecond: part(within % MS_PER_SECOND),
    }
}

// Texte

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn name(table: &[&'static str], index: u32) -> &'static str {
    usize::try_from(index)
        .ok()
        .and_then(|index| table.get(index))
        .copied()
        .unwrap_or("")
}

/// Ein Jahr wie in DateString: das Vorzeichen nur für negative Jahre, der
/// Betrag mit mindestens vier Ziffern.
fn push_year(text: &mut String, year: i64) {
    let sign = if year < 0 { "-" } else { "" };
    let _ = write!(text, "{sign}{:04}", year.unsigned_abs());
}

/// DateString (Abschnitt 21.4.4.41.2).
fn date_string(text: &mut String, t: f64) {
    let fields = fields(t);
    let _ = write!(
        text,
        "{} {} {:02} ",
        name(&WEEKDAYS, fields.weekday),
        name(&MONTHS, fields.month),
        fields.date
    );
    push_year(text, fields.year);
}

/// TimeString (Abschnitt 21.4.4.41.1).
fn time_string(text: &mut String, t: f64) {
    let fields = fields(t);
    let _ = write!(
        text,
        "{:02}:{:02}:{:02} GMT",
        fields.hour, fields.minute, fields.second
    );
}

/// TimeZoneString (Abschnitt 21.4.4.41.3). Der Name in Klammern ist eine
/// Festlegung: *"Coordinated Universal Time"* wie in V8 und SpiderMonkey
/// für UTC, für einen festen Abstand keiner.
fn time_zone_string(text: &mut String, zone: TimeZone) {
    let offset = zone.offset_milliseconds();
    let sign = if offset >= 0 { '+' } else { '-' };
    let absolute = offset.unsigned_abs();
    let hours = absolute / 3_600_000;
    let minutes = absolute / 60_000 % 60;
    let _ = write!(text, "{sign}{hours:02}{minutes:02}");
    if zone == TimeZone::Utc {
        text.push_str(" (Coordinated Universal Time)");
    }
}

/// ToDateString (Abschnitt 21.4.4.41.4).
pub(crate) fn to_date_string(tv: f64, zone: TimeZone) -> String {
    if tv.is_nan() {
        return String::from("Invalid Date");
    }
    let local = local_time(tv, zone);
    let mut text = String::new();
    date_string(&mut text, local);
    text.push(' ');
    time_string(&mut text, local);
    time_zone_string(&mut text, zone);
    text
}

/// Date.prototype.toDateString (Abschnitt 21.4.4.35) ab Schritt 4.
pub(crate) fn to_date_only_string(tv: f64, zone: TimeZone) -> String {
    if tv.is_nan() {
        return String::from("Invalid Date");
    }
    let mut text = String::new();
    date_string(&mut text, local_time(tv, zone));
    text
}

/// Date.prototype.toTimeString (Abschnitt 21.4.4.42) ab Schritt 4.
pub(crate) fn to_time_only_string(tv: f64, zone: TimeZone) -> String {
    if tv.is_nan() {
        return String::from("Invalid Date");
    }
    let mut text = String::new();
    time_string(&mut text, local_time(tv, zone));
    time_zone_string(&mut text, zone);
    text
}

/// Date.prototype.toUTCString (Abschnitt 21.4.4.43) ab Schritt 4.
pub(crate) fn to_utc_string(tv: f64) -> String {
    if tv.is_nan() {
        return String::from("Invalid Date");
    }
    let fields = fields(tv);
    let mut text = String::new();
    let _ = write!(
        text,
        "{}, {:02} {} ",
        name(&WEEKDAYS, fields.weekday),
        fields.date,
        name(&MONTHS, fields.month)
    );
    push_year(&mut text, fields.year);
    text.push(' ');
    time_string(&mut text, tv);
    text
}

/// Date.prototype.toISOString (Abschnitt 21.4.4.36) ab Schritt 6 für einen
/// endlichen Zeitwert: Jahre von 0 bis 9999 mit vier Ziffern, andere als
/// erweitertes Jahr mit Vorzeichen und sechs Ziffern. Nach TimeClip passt
/// jedes Jahr in sechs Ziffern.
pub(crate) fn to_iso_string(tv: f64) -> String {
    let fields = fields(tv);
    let mut text = String::new();
    if (0..=9999).contains(&fields.year) {
        let _ = write!(text, "{:04}", fields.year);
    } else {
        let sign = if fields.year < 0 { '-' } else { '+' };
        let _ = write!(text, "{sign}{:06}", fields.year.unsigned_abs());
    }
    let _ = write!(
        text,
        "-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        fields.month + 1,
        fields.date,
        fields.hour,
        fields.minute,
        fields.second,
        fields.millisecond
    );
    text
}

/// Welche Teile die Texte von toLocaleString, toLocaleDateString und
/// toLocaleTimeString haben.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocaleParts {
    DateAndTime,
    Date,
    Time,
}

/// Die Texte der Methoden für Gebietsschemata (Abschnitte 21.4.4.38 bis
/// 21.4.4.40) ohne ECMA-402. Die Form ist eine Festlegung, die der von
/// Browsern im Gebietsschema en-US folgt: `1/2/2020, 3:04:05 PM`, mit
/// gewöhnlichen Leerzeichen.
pub(crate) fn to_locale_string(tv: f64, zone: TimeZone, parts: LocaleParts) -> String {
    if tv.is_nan() {
        return String::from("Invalid Date");
    }
    let fields = fields(local_time(tv, zone));
    let mut text = String::new();
    if parts != LocaleParts::Time {
        let _ = write!(text, "{}/{}/{}", fields.month + 1, fields.date, fields.year);
    }
    if parts == LocaleParts::DateAndTime {
        text.push_str(", ");
    }
    if parts != LocaleParts::Date {
        let hour = match fields.hour % 12 {
            0 => 12,
            hour => hour,
        };
        let half = if fields.hour < 12 { "AM" } else { "PM" };
        let _ = write!(
            text,
            "{hour}:{:02}:{:02} {half}",
            fields.minute, fields.second
        );
    }
    text
}

// Lesen

const PLUS: u16 = b'+' as u16;
const MINUS: u16 = b'-' as u16;
const COLON: u16 = b':' as u16;
const DOT: u16 = b'.' as u16;
const COMMA: u16 = b',' as u16;
const SLASH: u16 = b'/' as u16;

/// Ein Leser über Codeeinheiten.
struct Cursor<'a> {
    units: &'a [u16],
    position: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u16> {
        self.units.get(self.position).copied()
    }

    fn next(&mut self) -> Option<u16> {
        let unit = self.peek()?;
        self.position += 1;
        Some(unit)
    }

    fn at_end(&self) -> bool {
        self.position >= self.units.len()
    }

    fn eat(&mut self, unit: u16) -> bool {
        if self.peek() == Some(unit) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn digit(&mut self) -> Option<i64> {
        let unit = self.peek()?;
        let digit = digit_value(unit)?;
        self.position += 1;
        Some(digit)
    }

    /// Genau `count` Ziffern als Zahl.
    fn digits(&mut self, count: usize) -> Option<i64> {
        let mut value = 0;
        for _ in 0..count {
            value = value * 10 + self.digit()?;
        }
        Some(value)
    }

    fn two_digits(&mut self) -> Option<i64> {
        self.digits(2)
    }
}

fn digit_value(unit: u16) -> Option<i64> {
    (u16::from(b'0')..=u16::from(b'9'))
        .contains(&unit)
        .then(|| i64::from(unit - u16::from(b'0')))
}

/// Was Date.parse (Abschnitt 21.4.3.2) aus `text` liest, oder NaN. Zuerst
/// das Date Time String Format (Abschnitt 21.4.1.32) mit erweiterten
/// Jahren; Texte, die nicht diese Form haben, liest [`parse_legacy`].
pub(crate) fn parse(text: &[u16], zone: TimeZone) -> f64 {
    match parse_iso(text, zone) {
        Iso::Value(value) => value,
        Iso::Unknown => parse_legacy(text, zone),
    }
}

/// Das Ergebnis für das Date Time String Format.
enum Iso {
    /// Der Text hat die Form des Formats; der Zeitwert, oder NaN für Werte
    /// außerhalb der Grenzen. Dann gibt es keinen Rückfall auf andere
    /// Formen, wie Abschnitt 21.4.1.32.1 es für erweiterte Jahre verlangt.
    Value(f64),
    /// Der Text hat eine andere Form.
    Unknown,
}

/// Die Tage des Monats `month` (1 bis 12) im Jahr `year`.
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Liest das Date Time String Format. Ein Text hat seine Form, wenn er
/// ganz aus den Elementen mit der richtigen Zahl von Ziffern besteht;
/// Werte außerhalb ihrer Grenzen (Monat 13, 30. Februar, 24:01) ergeben
/// dann NaN.
fn parse_iso(text: &[u16], zone: TimeZone) -> Iso {
    let mut cursor = Cursor {
        units: text,
        position: 0,
    };
    let year = match cursor.peek() {
        Some(PLUS | MINUS) => {
            let negative = cursor.next() == Some(MINUS);
            let Some(year) = cursor.digits(6) else {
                return Iso::Unknown;
            };
            // -000000 ist kein Jahr (Abschnitt 21.4.1.32.1).
            if negative && year == 0 {
                return Iso::Value(f64::NAN);
            }
            if negative { -year } else { year }
        }
        _ => match cursor.digits(4) {
            Some(year) => year,
            None => return Iso::Unknown,
        },
    };
    let mut month = 1;
    let mut day = 1;
    if cursor.eat(MINUS) {
        let Some(value) = cursor.two_digits() else {
            return Iso::Unknown;
        };
        month = value;
        if cursor.eat(MINUS) {
            let Some(value) = cursor.two_digits() else {
                return Iso::Unknown;
            };
            day = value;
        }
    }
    let mut time = None;
    let mut offset = None;
    if cursor.eat(u16::from(b'T')) {
        let Some(hour) = cursor.two_digits() else {
            return Iso::Unknown;
        };
        if !cursor.eat(COLON) {
            return Iso::Unknown;
        }
        let Some(minute) = cursor.two_digits() else {
            return Iso::Unknown;
        };
        let mut second = 0;
        let mut millisecond = 0;
        if cursor.eat(COLON) {
            let Some(value) = cursor.two_digits() else {
                return Iso::Unknown;
            };
            second = value;
            if cursor.eat(DOT) {
                let Some(value) = cursor.digits(3) else {
                    return Iso::Unknown;
                };
                millisecond = value;
            }
        }
        time = Some((hour, minute, second, millisecond));
        match cursor.peek() {
            Some(unit) if unit == u16::from(b'Z') => {
                cursor.position += 1;
                offset = Some(0);
            }
            Some(PLUS | MINUS) => {
                let sign = if cursor.next() == Some(MINUS) { -1 } else { 1 };
                let Some(hours) = cursor.two_digits() else {
                    return Iso::Unknown;
                };
                if !cursor.eat(COLON) {
                    return Iso::Unknown;
                }
                let Some(minutes) = cursor.two_digits() else {
                    return Iso::Unknown;
                };
                if hours > 23 || minutes > 59 {
                    return Iso::Value(f64::NAN);
                }
                offset = Some(sign * (hours * MS_PER_HOUR + minutes * MS_PER_MINUTE));
            }
            _ => {}
        }
    }
    if !cursor.at_end() {
        return Iso::Unknown;
    }
    // Ab hier hat der Text die Form; nur noch die Grenzen zählen.
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return Iso::Value(f64::NAN);
    }
    let (hour, minute, second, millisecond) = time.unwrap_or((0, 0, 0, 0));
    let midnight = hour == 24 && minute == 0 && second == 0 && millisecond == 0;
    if (hour > 23 && !midnight) || minute > 59 || second > 59 {
        return Iso::Value(f64::NAN);
    }
    // Jahre mit sechs Ziffern und Monate bis 12 sind als Number genau.
    let date = make_date(
        make_day(year as f64, (month - 1) as f64, day as f64),
        make_time(
            hour as f64,
            minute as f64,
            second as f64,
            millisecond as f64,
        ),
    );
    let value = match (time, offset) {
        // Ohne Abstand gilt ein Datum allein als UTC, mit Uhrzeit als
        // Ortszeit.
        (None, _) => date,
        (Some(_), Some(offset)) => date - offset as f64,
        (Some(_), None) => utc(date, zone),
    };
    Iso::Value(time_clip(value))
}

/// Ein Wort, das [`parse_legacy`] kennt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Word {
    Month(i64),
    Weekday,
    Am,
    Pm,
    /// `Z`, `UT`, `UTC` oder `GMT`, auf die ein Abstand folgen kann.
    Utc,
    /// Die Zeitzonen der USA aus RFC 5322 mit ihrem Abstand in Minuten.
    Zone(i64),
    /// `T` zwischen Datum und Uhrzeit.
    Separator,
}

fn word(letters: &[u8]) -> Option<Word> {
    const MONTH_NAMES: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    const WEEKDAY_NAMES: [&str; 7] = [
        "sunday",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
    ];
    let text = std::str::from_utf8(letters).ok()?;
    let found = match text {
        "am" => Word::Am,
        "pm" => Word::Pm,
        "z" | "ut" | "utc" | "gmt" => Word::Utc,
        "t" => Word::Separator,
        "est" => Word::Zone(-300),
        "edt" => Word::Zone(-240),
        "cst" => Word::Zone(-360),
        "cdt" => Word::Zone(-300),
        "mst" => Word::Zone(-420),
        "mdt" => Word::Zone(-360),
        "pst" => Word::Zone(-480),
        "pdt" => Word::Zone(-420),
        _ => {
            // Monate und Wochentage mit mindestens drei Buchstaben, ganz
            // oder abgekürzt.
            if text.len() < 3 {
                return None;
            }
            let matches = |full: &str| full.starts_with(text);
            if let Some(index) = MONTH_NAMES.iter().position(|full| matches(full)) {
                Word::Month(i64::try_from(index).ok()? + 1)
            } else if WEEKDAY_NAMES.iter().any(|full| matches(full)) {
                Word::Weekday
            } else {
                return None;
            }
        }
    };
    Some(found)
}

/// Eine Zahl im Text, ein Teil eines Datums: ihr Wert und die Zahl ihrer
/// Ziffern.
#[derive(Clone, Copy, Debug)]
struct Number {
    value: i64,
    digits: usize,
}

/// Liest Texte, die nicht das Date Time String Format haben, wie Browser
/// es tun, eine Festlegung nach Abschnitt 21.4.3.2. Erkannt werden die
/// Texte von toString und toUTCString und die üblichen Formen mit
/// Monatsnamen (`Jan 2 2020`, `2 January 2020`, `Thursday, January 2,
/// 2020`) oder Zahlen (`1/2/2020` als Monat, Tag, Jahr, `2020/1/2` und
/// `2020-1-2` als Jahr, Monat, Tag), dazu eine Uhrzeit `h:mm[:ss[.sss]]`
/// mit `AM` oder `PM` und eine Zeitzone: `Z`, `UT`, `UTC` oder `GMT`,
/// optional mit Abstand wie `+0100` oder `+01:00`, ein Abstand allein nach
/// der Uhrzeit oder eine der Zeitzonen der USA. Text in Klammern zählt
/// nicht. Zweistellige Jahre unter 50 liegen nach 2000, andere nach 1900.
/// Ohne Zeitzone gilt die Ortszeit. Alles andere ergibt NaN.
fn parse_legacy(text: &[u16], zone: TimeZone) -> f64 {
    legacy(text, zone).unwrap_or(f64::NAN)
}

fn legacy(text: &[u16], zone: TimeZone) -> Option<f64> {
    let mut cursor = Cursor {
        units: text,
        position: 0,
    };
    let mut numbers: Vec<Number> = Vec::new();
    let mut month_name: Option<i64> = None;
    let mut time: Option<(i64, i64, i64, i64)> = None;
    let mut half: Option<Word> = None;
    let mut offset: Option<i64> = None;
    let mut negative_year = false;
    // Ob vor dem nächsten Zeichen Leerraum stand.
    let mut spaced = true;
    while let Some(unit) = cursor.peek() {
        if unit == u16::from(b' ') || unit == u16::from(b'\t') || unit == COMMA {
            cursor.position += 1;
            spaced = true;
            continue;
        }
        if unit == u16::from(b'(') {
            // Klammern, auch geschachtelt, bis zur passenden.
            let mut depth = 0usize;
            while let Some(unit) = cursor.next() {
                if unit == u16::from(b'(') {
                    depth += 1;
                } else if unit == u16::from(b')') {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            spaced = true;
            continue;
        }
        if digit_value(unit).is_some() {
            let number = read_number(&mut cursor)?;
            if cursor.peek() == Some(COLON) {
                if time.is_some() {
                    return None;
                }
                time = Some(read_time(&mut cursor, number)?);
            } else {
                if numbers.len() == 3 {
                    return None;
                }
                numbers.push(number);
            }
            spaced = false;
            continue;
        }
        if unit == PLUS || unit == MINUS {
            cursor.position += 1;
            let starts_number = cursor.peek().and_then(digit_value).is_some();
            if starts_number && (time.is_some() && offset.is_none() || offset == Some(0) && !spaced)
            {
                // Ein Abstand nach der Uhrzeit oder direkt nach GMT.
                let sign = if unit == MINUS { -1 } else { 1 };
                offset = Some(sign * read_offset(&mut cursor)?);
            } else if unit == MINUS && spaced && starts_number && time.is_none() {
                // Ein negatives Jahr wie in `Apr 20 -271821`.
                if negative_year {
                    return None;
                }
                negative_year = true;
                let number = read_number(&mut cursor)?;
                numbers.push(Number {
                    value: -number.value,
                    digits: number.digits.max(3),
                });
            } else if spaced {
                return None;
            }
            // Sonst trennt das Zeichen Teile eines Datums.
            spaced = false;
            continue;
        }
        if unit == SLASH || unit == DOT {
            cursor.position += 1;
            spaced = false;
            continue;
        }
        let mut letters = Vec::new();
        while let Some(unit) = cursor.peek() {
            let Ok(byte) = u8::try_from(unit) else { break };
            if !byte.is_ascii_alphabetic() {
                break;
            }
            letters.push(byte.to_ascii_lowercase());
            cursor.position += 1;
        }
        if letters.is_empty() {
            return None;
        }
        match word(&letters)? {
            Word::Month(month) => {
                if month_name.is_some() {
                    return None;
                }
                month_name = Some(month);
            }
            Word::Weekday | Word::Separator => {}
            found @ (Word::Am | Word::Pm) if time.is_some() && half.is_none() => {
                half = Some(found);
            }
            Word::Am | Word::Pm => return None,
            Word::Utc if offset.is_none() => offset = Some(0),
            Word::Zone(minutes) if offset.is_none() => offset = Some(minutes * MS_PER_MINUTE),
            Word::Utc | Word::Zone(_) => return None,
        }
        spaced = false;
    }
    let (year, month, day) = legacy_date(&numbers, month_name)?;
    let (mut hour, minute, second, millisecond) = time.unwrap_or((0, 0, 0, 0));
    match half {
        Some(Word::Am) | Some(Word::Pm) if !(1..=12).contains(&hour) => return None,
        Some(Word::Am) if hour == 12 => hour = 0,
        Some(Word::Pm) if hour != 12 => hour += 12,
        _ => {}
    }
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 24
        || (hour == 24 && (minute, second, millisecond) != (0, 0, 0))
        || minute > 59
        || second > 59
    {
        return None;
    }
    // Jahre unter 10⁹ und Monate bis 12 sind als Number genau.
    let date = make_date(
        make_day(year as f64, (month - 1) as f64, day as f64),
        make_time(
            hour as f64,
            minute as f64,
            second as f64,
            millisecond as f64,
        ),
    );
    let value = match offset {
        Some(offset) => date - offset as f64,
        None => utc(date, zone),
    };
    Some(time_clip(value))
}

/// Eine Folge von Ziffern; mehr als neun ergeben keinen Teil eines Datums.
fn read_number(cursor: &mut Cursor<'_>) -> Option<Number> {
    let mut value = 0i64;
    let mut digits = 0usize;
    while let Some(digit) = cursor.digit() {
        if digits == 9 {
            return None;
        }
        value = value * 10 + digit;
        digits += 1;
    }
    Some(Number { value, digits })
}

/// Eine Uhrzeit `h:mm[:ss[.sss]]` ab dem Doppelpunkt nach der Stunde.
fn read_time(cursor: &mut Cursor<'_>, hour: Number) -> Option<(i64, i64, i64, i64)> {
    if hour.digits > 2 {
        return None;
    }
    cursor.eat(COLON);
    let minute = read_number(cursor)?;
    if minute.digits != 2 {
        return None;
    }
    let mut second = 0;
    let mut millisecond = 0;
    if cursor.eat(COLON) {
        let value = read_number(cursor)?;
        if value.digits != 2 {
            return None;
        }
        second = value.value;
        if cursor.eat(DOT) {
            // Nur die ersten drei Ziffern zählen.
            let mut digits = 0;
            while let Some(digit) = cursor.digit() {
                if digits < 3 {
                    millisecond = millisecond * 10 + digit;
                }
                digits += 1;
            }
            if digits == 0 {
                return None;
            }
            for _ in digits..3 {
                millisecond *= 10;
            }
        }
    }
    Some((hour.value, minute.value, second, millisecond))
}

/// Ein Abstand `hh`, `hhmm` oder `hh:mm` nach seinem Vorzeichen, in
/// Millisekunden.
fn read_offset(cursor: &mut Cursor<'_>) -> Option<i64> {
    let first = read_number(cursor)?;
    let (hours, minutes) = if cursor.eat(COLON) {
        let minutes = read_number(cursor)?;
        if first.digits > 2 || minutes.digits != 2 {
            return None;
        }
        (first.value, minutes.value)
    } else {
        match first.digits {
            1 | 2 => (first.value, 0),
            4 => (first.value / 100, first.value % 100),
            _ => return None,
        }
    };
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(hours * MS_PER_HOUR + minutes * MS_PER_MINUTE)
}

/// Jahr, Monat und Tag aus den Zahlen und dem Monatsnamen.
fn legacy_date(numbers: &[Number], month_name: Option<i64>) -> Option<(i64, i64, i64)> {
    // Eine Zahl mit drei oder mehr Ziffern oder über 31 ist ein Jahr.
    let is_year = |number: &Number| number.digits >= 3 || number.value > 31 || number.value < 0;
    let (year, month, day) = match (month_name, numbers) {
        (Some(month), [first, second]) => {
            if is_year(first) {
                (*first, month, second.value)
            } else {
                (*second, month, first.value)
            }
        }
        (None, [first, second, third]) => {
            if is_year(first) {
                (*first, second.value, third.value)
            } else {
                (*third, first.value, second.value)
            }
        }
        _ => return None,
    };
    let full = if year.digits <= 2 && year.value >= 0 {
        if year.value < 50 {
            2000 + year.value
        } else {
            1900 + year.value
        }
    } else {
        year.value
    };
    Some((full, month, day))
}

#[cfg(test)]
mod tests;
