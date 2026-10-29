//! iCalendar (RFC 5545) output: invitations attached to guest emails (`REQUEST`, `CANCEL`), the
//! guest's `.ics` download (`PUBLISH`) and staff calendar feeds. Times are written in UTC.

use crate::Instant;

#[derive(Debug, Clone)]
pub struct Event {
    pub uid: String,
    /// Grows with every change, so calendars replace the earlier version.
    pub sequence: i64,
    pub stamp: Instant,
    pub start: Instant,
    pub end: Instant,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    /// `CONFIRMED`, `TENTATIVE` or `CANCELLED`.
    pub status: &'static str,
    /// `(name, email)`.
    pub organizer: Option<(String, String)>,
    pub attendees: Vec<(String, String)>,
}

/// TEXT value escaping: backslash, semicolon, comma and line breaks; other controls dropped.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\\n");
            }
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Folds a content line at 75 octets without splitting a character.
pub fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / 70 * 3);
    let mut used = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        if used + n > 75 {
            out.push_str("\r\n ");
            used = 1;
        }
        out.push(c);
        used += n;
    }
    out
}

/// A parameter value (`CN=`): quoted, without characters a quoted value cannot hold.
fn param(v: &str) -> String {
    format!("\"{}\"", v.chars().filter(|c| *c != '"' && !c.is_control()).collect::<String>())
}

fn utc(t: Instant) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

pub fn calendar(method: Option<&str>, name: Option<&str>, events: &[Event]) -> String {
    let mut lines = vec!["BEGIN:VCALENDAR".to_string(), "VERSION:2.0".into(), "PRODID:-//Ferrum//Bookings//EN".into(), "CALSCALE:GREGORIAN".into()];
    if let Some(m) = method {
        lines.push(format!("METHOD:{m}"));
    }
    if let Some(n) = name {
        lines.push(format!("X-WR-CALNAME:{}", escape(n)));
        lines.push("X-PUBLISHED-TTL:PT1H".into());
        lines.push("REFRESH-INTERVAL;VALUE=DURATION:PT1H".into());
    }
    for e in events {
        lines.push("BEGIN:VEVENT".into());
        lines.push(format!("UID:{}", escape(&e.uid)));
        lines.push(format!("SEQUENCE:{}", e.sequence.max(0)));
        lines.push(format!("DTSTAMP:{}", utc(e.stamp)));
        lines.push(format!("DTSTART:{}", utc(e.start)));
        lines.push(format!("DTEND:{}", utc(e.end)));
        lines.push(format!("SUMMARY:{}", escape(&e.summary)));
        if let Some(d) = &e.description {
            lines.push(format!("DESCRIPTION:{}", escape(d)));
        }
        if let Some(l) = &e.location {
            lines.push(format!("LOCATION:{}", escape(l)));
        }
        lines.push(format!("STATUS:{}", e.status));
        lines.push("TRANSP:OPAQUE".into());
        if let Some((name, email)) = &e.organizer {
            lines.push(format!("ORGANIZER;CN={}:mailto:{email}", param(name)));
        }
        for (name, email) in &e.attendees {
            lines.push(format!("ATTENDEE;CN={};ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;RSVP=FALSE:mailto:{email}", param(name)));
        }
        lines.push("END:VEVENT".into());
    }
    lines.push("END:VCALENDAR".into());
    let mut out = lines.iter().map(|l| fold(l)).collect::<Vec<_>>().join("\r\n");
    out.push_str("\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn t(s: &str) -> Instant {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn invitations_are_well_formed() {
        let e = Event {
            uid: "0192@bookings.ferrum".into(),
            sequence: 3,
            stamp: t("2026-10-29T10:00:00Z"),
            start: t("2026-11-02T04:30:00Z"),
            end: t("2026-11-02T05:30:00Z"),
            summary: "Deep tissue massage, 60 min; with Asha".into(),
            description: Some("Reference BKG-2026-00001\nBring water".into()),
            location: Some("Room A, 2nd floor".into()),
            status: "CONFIRMED",
            organizer: Some(("Asha \"Ash\" Rao".into(), "asha@spa.test".into())),
            attendees: vec![("Guest, Jane".into(), "jane@example.test".into())],
        };
        let ics = calendar(Some("REQUEST"), None, std::slice::from_ref(&e));
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n") && ics.ends_with("END:VCALENDAR\r\n"));
        assert!(ics.contains("\r\nMETHOD:REQUEST\r\n"));
        assert!(ics.contains("\r\nDTSTART:20261102T043000Z\r\nDTEND:20261102T053000Z\r\n"));
        assert!(ics.contains("SUMMARY:Deep tissue massage\\, 60 min\\; with Asha\r\n"));
        assert!(ics.contains("DESCRIPTION:Reference BKG-2026-00001\\nBring water\r\n"));
        assert!(ics.contains("ORGANIZER;CN=\"Asha Ash Rao\":mailto:asha@spa.test\r\n"));
        assert!(ics.contains("SEQUENCE:3\r\n"));
        assert!(ics.replace("\r\n ", "").contains("ATTENDEE;CN=\"Guest, Jane\";ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;RSVP=FALSE:mailto:jane@example.test"));
        assert!(ics.split("\r\n").all(|l| l.len() <= 75), "every physical line fits 75 octets");
        let feed = calendar(None, Some("Asha's bookings"), &[e]);
        assert!(!feed.contains("METHOD:") && feed.contains("X-WR-CALNAME:Asha's bookings\r\n"));
    }

    #[test]
    fn folding_keeps_characters_whole() {
        for src in [format!("DESCRIPTION:{}", "x".repeat(200)), format!("SUMMARY:{}", "Réunion 日本語 ✓ ".repeat(12))] {
            let folded = fold(&src);
            for (i, physical) in folded.split("\r\n").enumerate() {
                assert!(physical.len() <= 75, "line {i} is {} octets", physical.len());
                assert!(i == 0 || physical.starts_with(' '));
            }
            assert_eq!(folded.replace("\r\n ", ""), src);
        }
        assert_eq!(escape("a\r\nb\u{7}c\\"), "a\\nbc\\\\");
    }
}
