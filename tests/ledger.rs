use blackhole_check::{
    engine::registry,
    ledger::{Counts, excess, moved, parse, write},
};

const HEADER: &str = "blackhole-ledger 1\n";
const RULE: &str = "core/no-suppression";

#[test]
fn round_trip() {
    for rows in ["", "1\tx\n", "3\tdir/api.rs\n12\té.py\n"] {
        let mut bytes = HEADER.to_owned();
        for row in rows.lines() {
            use std::fmt::Write;
            writeln!(bytes, "{RULE}\t{row}").unwrap();
        }
        assert_eq!(
            write(&parse(bytes.as_bytes(), registry()).unwrap()).unwrap(),
            bytes
        );
    }
}

#[test]
fn malformed() {
    let row = format!("{RULE}\t1\tx\n");
    let mut cases = vec![
        (String::new(), 1),
        ("other\n".into(), 1),
        (HEADER.trim_end().into(), 1),
        (format!("{HEADER}{row}{row}"), 3),
        (format!("{HEADER}{row}{RULE}\t1\ta\n"), 3),
        (format!("{HEADER}ts/no-test-removal\t1\tx\n"), 2),
        (format!("{HEADER}unknown\t1\tx\n"), 2),
    ];
    for suffix in [
        "",
        "\n",
        "\t1\tx",
        "\t0\tx\n",
        "\t01\tx\n",
        "\t+1\tx\n",
        "\t-1\tx\n",
        "\t1a\tx\n",
        "\t1\t\n",
        "\t1\t/x\n",
        "\t1\ta/../x\n",
        "\t1\ta\\x\n",
        "\t1\ta\rb\n",
        "\t1\ta\tb\n",
        "\t1\ta\0.py\n",
        "\t1\t./a.py\n",
        "\t1\tdir//a.py\n",
        "\t1\tdir/./a.py\n",
        "\t1\tdir/\n",
    ] {
        cases.push((format!("{HEADER}{RULE}{suffix}"), 2));
    }
    cases.push((format!("{HEADER}\n"), 2));
    for (bytes, line) in cases {
        assert!(
            parse(bytes.as_bytes(), registry())
                .unwrap_err()
                .to_string()
                .contains(&format!("line {line}:")),
            "{bytes:?}"
        );
    }
    let bytes = [HEADER.as_bytes(), row.as_bytes(), b"\xff\n"].concat();
    assert!(
        parse(&bytes, registry())
            .unwrap_err()
            .to_string()
            .contains("line 3:")
    );
}

fn entries(rows: &[(&str, usize)]) -> Counts {
    rows.iter()
        .map(|&(path, n)| ((RULE.to_owned(), path.to_owned()), n))
        .collect()
}

#[test]
fn exact_counts() {
    for (base, actual, over, stale) in [
        (1, 1, false, false),
        (0, 1, true, false),
        (1, 2, true, false),
        (2, 1, false, true),
        (1, 0, false, true),
    ] {
        let rows = |n| {
            if n == 0 {
                Counts::new()
            } else {
                entries(&[("a", n)])
            }
        };
        let allowed = rows(base);
        let actual_counts = rows(actual);
        assert_eq!(!excess(&actual_counts, &allowed).is_empty(), over);
        assert_eq!(!excess(&allowed, &actual_counts).is_empty(), stale);
    }
}

#[test]
fn rename_allowances() {
    let base = entries(&[("a", 2), ("b", 1)]);
    for (renames, path, n, grows) in [
        (vec![], "a", 1, false),
        (vec![], "c", 1, true),
        (vec![("a", "c")], "c", 2, false),
        (vec![("a", "c"), ("b", "c")], "c", 3, false),
        (vec![("a", "c")], "c", 3, true),
        (vec![("missing", "c")], "c", 1, true),
        (vec![("a", "c")], "a", 1, true),
    ] {
        let head = entries(&[(path, n)]);
        assert_eq!(
            excess(&head, &moved(&base, &renames)),
            if grows { head } else { Counts::new() }
        );
    }
}

#[test]
fn unrepresentable_rows() {
    let injected = format!("a\n{RULE}\t5\tz");
    for path in ["a\tb", "a\nb", injected.as_str()] {
        assert!(write(&entries(&[(path, 1)])).is_err(), "{path:?}");
    }
    assert!(write(&entries(&[("a", 0)])).is_err());
    let unknown = Counts::from([(("unknown".to_owned(), "a".to_owned()), 1)]);
    assert!(write(&unknown).is_err());
}
