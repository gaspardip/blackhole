use blackhole_check::objections::{Date, parse};

fn date(text: &str) -> Date {
    text.parse().unwrap()
}

#[test]
fn calendar() {
    for (a, b, n) in [
        ("1900-02-28", "1900-03-01", 1),
        ("2000-02-28", "2000-03-01", 2),
        ("2024-02-29", "2024-03-01", 1),
        ("2026-11-18", "2026-12-18", 30),
        ("1999-12-31", "2000-01-01", 1),
    ] {
        assert_eq!(date(b).day_number() - date(a).day_number(), n);
    }
    for (days, text) in [
        (0, "1970-01-01"),
        (58, "1970-02-28"),
        (59, "1970-03-01"),
        (364, "1970-12-31"),
        (789, "1972-02-29"),
        (1095, "1972-12-31"),
        (10956, "1999-12-31"),
        (11016, "2000-02-29"),
        (11017, "2000-03-01"),
        (20775, "2026-11-18"),
        (47541, "2100-03-01"),
    ] {
        assert_eq!(Date::from_unix_days(days), Some(date(text)), "{text}");
    }
    for s in [
        "1900-02-29",
        "2026-04-31",
        "2026-00-01",
        "2026-13-01",
        "2026-01-00",
        "2026-1-01",
        "2026-01-01!",
    ] {
        assert!(s.parse::<Date>().is_err());
    }
}

#[test]
fn strict_rows() {
    let row = "core/no-suppression\ta.py\t1\t2026-11-18\t-\treason\n";
    for (from, to) in [
        ("core/no-suppression", "unknown"),
        ("core/no-suppression", "ts/no-test-removal"),
        ("a.py", "../a.py"),
        ("\t1\t", "\t0\t"),
        ("\t1\t", "\t+1\t"),
        ("11-18", "02-30"),
        ("11-18", "12-01"),
        ("\t-\t", "\t2026-11-17\t"),
        ("\t-\t", "\t2026-11-19\t"),
        ("reason", ""),
        ("reason", "a\tb"),
        ("\n", ""),
    ] {
        assert!(
            parse(row.replace(from, to).as_bytes(), date("2026-11-18"))
                .unwrap_err()
                .to_string()
                .contains("objections line 1:")
        );
    }
    assert!(
        parse(row.repeat(2).as_bytes(), date("2026-11-18"))
            .unwrap_err()
            .to_string()
            .contains("objections line 2:")
    );
}

#[test]
fn inclusive_lifetime() {
    for renewal in ["-", "2026-12-18"] {
        let rows = parse(
            format!("core/no-suppression\ta\t1\t2026-11-18\t{renewal}\treason\n").as_bytes(),
            date("2027-01-31"),
        )
        .unwrap();
        let entry = rows.values().next().unwrap();
        let days = if renewal == "-" {
            ["2026-11-18", "2026-12-18", "2026-12-19"]
        } else {
            ["2026-12-18", "2027-01-17", "2027-01-18"]
        };
        assert_eq!(days.map(|s| entry.live(date(s))), [true, true, false]);
    }
}
