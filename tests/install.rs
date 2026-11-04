use std::fs;
use std::path::Path;

use claude_review::install::{
    commands_dir, install_commands, is_ours, remove_legacy_project_commands, stale_commands,
    Outcome, COMMANDS, LEGACY, MARKER,
};
use tempfile::TempDir;

fn commands_home() -> (TempDir, std::path::PathBuf) {
    let home = TempDir::with_prefix("claude-review-home-").expect("temp dir");
    let dir = home.path().join(".claude/commands");
    (home, dir)
}

fn outcome_for(results: &[(std::path::PathBuf, Outcome)], file: &str) -> Outcome {
    results
        .iter()
        .find(|(path, _)| path.ends_with(file))
        .map(|(_, outcome)| *outcome)
        .unwrap_or_else(|| panic!("no result for {file}"))
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn installs_every_command_into_a_fresh_directory() {
    let (_home, dir) = commands_home();

    let results = install_commands(&dir, false).expect("install");

    for command in &COMMANDS {
        assert_eq!(outcome_for(&results, command.file), Outcome::Created);
        assert_eq!(
            fs::read_to_string(dir.join(command.file)).expect("read"),
            command.source
        );
    }
}

#[test]
fn re_running_reports_that_nothing_changed() {
    let (_home, dir) = commands_home();

    install_commands(&dir, false).expect("first install");
    let results = install_commands(&dir, false).expect("second install");

    assert!(results
        .iter()
        .all(|(_, outcome)| *outcome == Outcome::AlreadyCurrent));
}

#[test]
fn replaces_an_older_managed_copy() {
    let (_home, dir) = commands_home();
    fs::create_dir_all(&dir).expect("mkdir");
    fs::write(
        dir.join("review-feedback.md"),
        format!("---\n---\n{MARKER}\nan older version\n"),
    )
    .expect("write");

    let results = install_commands(&dir, false).expect("install");

    assert_eq!(
        outcome_for(&results, "review-feedback.md"),
        Outcome::Updated
    );
    assert_eq!(
        fs::read_to_string(dir.join("review-feedback.md")).expect("read"),
        COMMANDS[0].source
    );
}

#[test]
fn upgrades_the_unedited_command_from_before_the_marker() {
    let (_home, dir) = commands_home();
    fs::create_dir_all(&dir).expect("mkdir");
    let (file, legacy) = LEGACY[0];
    fs::write(dir.join(file), legacy).expect("write");

    let results = install_commands(&dir, false).expect("install");

    assert_eq!(outcome_for(&results, file), Outcome::Updated);
    assert!(fs::read_to_string(dir.join(file))
        .expect("read")
        .contains(MARKER));
}

#[test]
fn leaves_an_edited_copy_alone_unless_forced() {
    let (_home, dir) = commands_home();
    fs::create_dir_all(&dir).expect("mkdir");
    fs::write(dir.join("review-feedback.md"), "my own version\n").expect("write");

    let results = install_commands(&dir, false).expect("install");
    assert_eq!(
        outcome_for(&results, "review-feedback.md"),
        Outcome::Modified
    );
    assert_eq!(
        fs::read_to_string(dir.join("review-feedback.md")).expect("read"),
        "my own version\n"
    );

    let forced = install_commands(&dir, true).expect("forced install");
    assert_eq!(outcome_for(&forced, "review-feedback.md"), Outcome::Updated);
}

#[test]
fn removes_a_managed_command_that_is_no_longer_shipped() {
    let (_home, dir) = commands_home();
    fs::create_dir_all(&dir).expect("mkdir");
    fs::write(dir.join("review-retired.md"), format!("{MARKER}\nold\n")).expect("write");
    fs::write(dir.join("someone-elses.md"), "not ours\n").expect("write");

    let results = install_commands(&dir, false).expect("install");

    assert_eq!(outcome_for(&results, "review-retired.md"), Outcome::Removed);
    assert!(!dir.join("review-retired.md").exists());
    assert!(dir.join("someone-elses.md").exists());
}

#[test]
fn leaves_no_temp_files_behind() {
    let (_home, dir) = commands_home();

    install_commands(&dir, false).expect("install");

    let mut expected: Vec<String> = COMMANDS.iter().map(|c| c.file.to_string()).collect();
    expected.sort();
    assert_eq!(names(&dir), expected);
}

#[test]
fn reports_stale_commands_but_not_edited_ones() {
    let (_home, dir) = commands_home();
    assert_eq!(stale_commands(&dir).len(), COMMANDS.len());

    install_commands(&dir, false).expect("install");
    assert!(stale_commands(&dir).is_empty());

    fs::write(dir.join("review-start.md"), "edited\n").expect("write");
    assert!(stale_commands(&dir).is_empty());

    fs::write(dir.join("review-feedback.md"), LEGACY[0].1).expect("write");
    assert_eq!(stale_commands(&dir), vec!["review-feedback.md"]);
}

#[test]
fn removes_an_untouched_legacy_project_copy() {
    let repo = TempDir::with_prefix("claude-review-repo-").expect("temp dir");
    let dir = repo.path().join(".claude/commands");
    fs::create_dir_all(&dir).expect("mkdir");
    fs::write(dir.join(LEGACY[0].0), LEGACY[0].1).expect("write");

    let notes = remove_legacy_project_commands(repo.path());

    assert_eq!(notes.len(), 1);
    assert!(!dir.join(LEGACY[0].0).exists());
}

#[test]
fn keeps_an_edited_legacy_project_copy_and_says_so() {
    let repo = TempDir::with_prefix("claude-review-repo-").expect("temp dir");
    let dir = repo.path().join(".claude/commands");
    fs::create_dir_all(&dir).expect("mkdir");
    fs::write(dir.join(LEGACY[0].0), "edited\n").expect("write");

    let notes = remove_legacy_project_commands(repo.path());

    assert_eq!(notes.len(), 1);
    assert!(dir.join(LEGACY[0].0).exists());
}

#[test]
fn targets_the_home_directory() {
    let home = dirs::home_dir().expect("home dir");
    assert_eq!(
        commands_dir().expect("dir"),
        home.join(".claude").join("commands")
    );
}

#[test]
fn every_shipped_command_is_marked_and_recognised() {
    for command in &COMMANDS {
        assert!(
            command.source.contains(MARKER),
            "{} lacks the marker",
            command.file
        );
        assert!(is_ours(command.file, command.source));
    }
    assert!(!is_ours("review-feedback.md", "anything else"));
}

#[test]
fn the_commands_use_the_session_cli() {
    let feedback = COMMANDS[0].source;
    assert!(feedback.contains("claude-review sessions --status submitted --json"));
    assert!(feedback.contains("claude-review take"));

    let start = COMMANDS[1].source;
    assert!(start.contains("claude-review register --by claude"));
}
