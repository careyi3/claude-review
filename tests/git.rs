use claude_review::git::{parse_name_status, parse_porcelain_v2, StatusEntry};

fn entry(
    path: &str,
    orig_path: Option<&str>,
    xy: &str,
    untracked: bool,
    unmerged: bool,
    similarity: Option<u32>,
) -> StatusEntry {
    StatusEntry {
        path: path.to_string(),
        orig_path: orig_path.map(str::to_string),
        xy: xy.to_string(),
        sub: "N...".to_string(),
        untracked,
        unmerged,
        similarity,
    }
}

#[test]
fn parses_an_ordinary_changed_entry() {
    let raw = "1 .M N... 100644 100644 100644 aaa bbb src/foo.ts\0";

    assert_eq!(
        parse_porcelain_v2(raw),
        vec![entry("src/foo.ts", None, ".M", false, false, None)]
    );
}

#[test]
fn reads_the_original_path_of_a_rename_from_the_following_field() {
    let raw = "2 R. N... 100644 100644 100644 aaa bbb R096 new/name.ts\0old/name.ts\0";
    let entries = parse_porcelain_v2(raw);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, "new/name.ts");
    assert_eq!(entries[0].orig_path.as_deref(), Some("old/name.ts"));
    assert_eq!(entries[0].similarity, Some(96));
}

#[test]
fn keeps_spaces_in_paths() {
    let raw = "1 .M N... 100644 100644 100644 aaa bbb dir with spaces/a b.txt\0? un tracked.txt\0";

    let paths: Vec<String> = parse_porcelain_v2(raw)
        .into_iter()
        .map(|e| e.path)
        .collect();

    assert_eq!(paths, vec!["dir with spaces/a b.txt", "un tracked.txt"]);
}

#[test]
fn flags_unmerged_entries_and_skips_headers_and_ignored_files() {
    let raw = "# branch.oid abc\0u UU N... 100644 100644 100644 100644 a b c conflict.txt\0! ignored.log\0";
    let entries = parse_porcelain_v2(raw);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, "conflict.txt");
    assert!(entries[0].unmerged);
    assert_eq!(entries[0].xy, "UU");
}

#[test]
fn marks_untracked_entries() {
    let entries = parse_porcelain_v2("? fresh.txt\0");

    assert_eq!(entries.len(), 1);
    assert!(entries[0].untracked);
    assert_eq!(entries[0].xy, "??");
}

#[test]
fn returns_nothing_for_a_clean_tree() {
    assert!(parse_porcelain_v2("").is_empty());
}

#[test]
fn parses_name_status_entries_including_renames() {
    let raw = "M\0src/a.ts\0R087\0old.ts\0new.ts\0A\0added.ts\0D\0gone.ts\0";

    assert_eq!(
        parse_name_status(raw),
        vec![
            entry("src/a.ts", None, "M", false, false, None),
            entry("new.ts", Some("old.ts"), "R087", false, false, Some(87)),
            entry("added.ts", None, "A", false, false, None),
            entry("gone.ts", None, "D", false, false, None),
        ]
    );
}

#[test]
fn marks_unmerged_name_status_entries() {
    assert!(parse_name_status("U\0conflict.ts\0")[0].unmerged);
}
