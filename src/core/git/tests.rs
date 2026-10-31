use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::cmd::{Git, GitError, classify, spawn};
use super::conflict::{self, Choice, ConflictKind};
use super::locate;
use super::setup;
use super::status::{ChangeKind, parse};
use super::sync::{MAX_FILE, Report};

fn exe() -> PathBuf {
    locate::find(None).expect("git")
}

fn raw(dir: &Path, args: &[&str]) {
    let out = Command::new(exe()).current_dir(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn fresh(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("schrumbi-git-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dunce::canonicalize(&dir).unwrap()
}

fn configure(dir: &Path) {
    raw(dir, &["config", "user.name", "Muster"]);
    raw(dir, &["config", "user.email", "muster@example.invalid"]);
    raw(dir, &["config", "commit.gpgsign", "false"]);
    raw(dir, &["config", "core.hooksPath", "/dev/null"]);
}

/// A bare remote with one commit and two clones standing for two devices.
fn devices(name: &str) -> (Git, Git) {
    let base = fresh(name);
    raw(&base, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
    raw(&base, &["clone", "-q", "remote.git", "a"]);
    let a = base.join("a");
    configure(&a);
    std::fs::write(a.join("n.md"), "eins\nzwei\ndrei\n").unwrap();
    std::fs::write(a.join("b.pdf"), [0u8, 1, 2]).unwrap();
    raw(&a, &["add", "-A"]);
    raw(&a, &["commit", "-q", "-m", "start"]);
    raw(&a, &["push", "-q", "-u", "origin", "HEAD"]);
    raw(&base, &["clone", "-q", "remote.git", "b"]);
    let b = base.join("b");
    configure(&b);
    (Git::new(exe(), &a), Git::new(exe(), &b))
}

fn sync(git: &Git) -> Result<Report, GitError> {
    git.sync(&|files| format!("Stand: {} Dateien", files.len()), &|_| {})
}

fn read(git: &Git, name: &str) -> String {
    std::fs::read_to_string(git.root.join(name)).unwrap()
}

fn log_count(git: &Git) -> usize {
    git.run(&["rev-list", "--count", "HEAD"]).unwrap().trim().parse().unwrap()
}

#[test]
fn changes_travel_between_devices() {
    let (a, b) = devices("travel");
    std::fs::write(a.root.join("neu.md"), "hallo").unwrap();
    let report = sync(&a).unwrap();
    assert_eq!(report.committed, 1);
    assert!(report.pushed);
    let report = sync(&b).unwrap();
    assert_eq!(report.committed, 0);
    assert_eq!(report.pulled, vec![b.root.join("neu.md")]);
    assert_eq!(read(&b, "neu.md"), "hallo");
    assert!(!report.pushed);
}

#[test]
fn nothing_to_do_makes_no_state() {
    let (a, _) = devices("nothing");
    let before = log_count(&a);
    let report = sync(&a).unwrap();
    assert_eq!(report, Report::default());
    assert_eq!(log_count(&a), before);
}

#[test]
fn pulled_files_get_the_time_of_their_state() {
    let (a, b) = devices("times");
    std::fs::write(a.root.join("neu.md"), "hallo").unwrap();
    sync(&a).unwrap();
    let state: u64 = a.run(&["log", "-1", "--format=%ct"]).unwrap().trim().parse().unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    sync(&b).unwrap();
    let time = std::fs::metadata(b.root.join("neu.md")).unwrap().modified().unwrap();
    assert_eq!(time.duration_since(UNIX_EPOCH).unwrap().as_secs(), state);
}

#[test]
fn both_sides_changing_different_files_merge() {
    let (a, b) = devices("merge");
    std::fs::write(a.root.join("a.md"), "a").unwrap();
    sync(&a).unwrap();
    std::fs::write(b.root.join("b.md"), "b").unwrap();
    let report = sync(&b).unwrap();
    assert!(report.pushed);
    assert_eq!(report.pulled, vec![b.root.join("a.md")]);
    assert_eq!(read(&b, "a.md"), "a");
    sync(&a).unwrap();
    assert_eq!(read(&a, "b.md"), "b");
    assert!(a.status().unwrap().changed.is_empty());
}

#[test]
fn conflicts_leave_the_working_tree_alone() {
    let (a, b) = devices("conflict");
    std::fs::write(a.root.join("n.md"), "eins\nA\ndrei\n").unwrap();
    std::fs::write(a.root.join("b.pdf"), [0u8, 9]).unwrap();
    sync(&a).unwrap();
    std::fs::write(b.root.join("n.md"), "eins\nB\ndrei\n").unwrap();
    std::fs::write(b.root.join("b.pdf"), [0u8, 8]).unwrap();
    let Err(GitError::Conflicts(mut files)) = sync(&b) else { panic!("conflict expected") };
    files.sort();
    assert_eq!(files, ["b.pdf", "n.md"]);
    assert_eq!(read(&b, "n.md"), "eins\nB\ndrei\n");
    assert!(!b.git_dir().join("MERGE_HEAD").exists());
    assert!(b.status().unwrap().changed.is_empty());
}

#[test]
fn awkward_names_survive() {
    let (a, b) = devices("names");
    for name in ["Übung ä.md", "-dash.md", "mit  zwei Leerzeichen.md", "ordner mit ß/ö.md", "\"quote\".md"] {
        if name.contains('"') && cfg!(windows) {
            continue;
        }
        let path = a.root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, name).unwrap();
    }
    let report = sync(&a).unwrap();
    assert!(report.committed >= 4);
    sync(&b).unwrap();
    assert_eq!(read(&b, "-dash.md"), "-dash.md");
    assert_eq!(read(&b, "ordner mit ß/ö.md"), "ordner mit ß/ö.md");
    assert!(a.status().unwrap().changed.is_empty());
}

#[test]
fn nested_repositories_stay_out() {
    let (a, b) = devices("nested");
    let inner = a.root.join("P1/Projekt");
    std::fs::create_dir_all(&inner).unwrap();
    raw(&inner, &["init", "-q"]);
    configure(&inner);
    std::fs::write(inner.join("i.txt"), "i").unwrap();
    raw(&inner, &["add", "-A"]);
    raw(&inner, &["commit", "-q", "-m", "i"]);
    std::fs::write(a.root.join("normal.md"), "n").unwrap();
    let report = sync(&a).unwrap();
    assert_eq!(report.nested, ["P1/Projekt"]);
    assert_eq!(report.committed, 1);
    assert!(!a.run(&["ls-files", "-s"]).unwrap().contains("P1/Projekt"));
    sync(&b).unwrap();
    assert!(!b.root.join("P1").exists());
}

#[test]
fn oversized_files_stay_local_and_are_reported() {
    let (a, _) = devices("large");
    let file = std::fs::File::create(a.root.join("gross.zip")).unwrap();
    file.set_len(MAX_FILE + 1).unwrap();
    std::fs::write(a.root.join("klein.md"), "k").unwrap();
    let report = sync(&a).unwrap();
    assert_eq!(report.too_large, ["gross.zip"]);
    assert_eq!(report.committed, 1);
    assert!(!a.run(&["ls-files"]).unwrap().contains("gross.zip"));
}

#[test]
fn a_broken_remote_keeps_the_local_state() {
    let (a, _) = devices("broken");
    raw(&a.root, &["remote", "set-url", "origin", "/gibt/es/nicht.git"]);
    std::fs::write(a.root.join("x.md"), "x").unwrap();
    let before = log_count(&a);
    assert!(sync(&a).is_err());
    assert_eq!(log_count(&a), before + 1);
    assert!(a.status().unwrap().changed.is_empty());
}

#[test]
fn no_remote_saves_and_says_so() {
    let dir = fresh("noremote");
    raw(&dir, &["init", "-q", "-b", "main"]);
    configure(&dir);
    std::fs::write(dir.join("x.md"), "x").unwrap();
    let git = Git::new(exe(), &dir);
    assert_eq!(sync(&git), Err(GitError::NoRemote));
    assert_eq!(log_count(&git), 1);
}

#[test]
fn first_push_sets_the_upstream() {
    let base = fresh("firstpush");
    raw(&base, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
    raw(&base, &["init", "-q", "-b", "main", "a"]);
    let a = base.join("a");
    configure(&a);
    raw(&a, &["remote", "add", "origin", "../remote.git"]);
    std::fs::write(a.join("x.md"), "x").unwrap();
    let git = Git::new(exe(), &a);
    assert!(sync(&git).unwrap().pushed);
    assert_eq!(git.status().unwrap().upstream.as_deref(), Some("origin/main"));
}

#[test]
fn stops_in_states_only_the_user_should_change() {
    let (a, _) = devices("states");
    std::fs::write(a.git_dir().join("index.lock"), "").unwrap();
    assert_eq!(sync(&a), Err(GitError::Locked));
    std::fs::remove_file(a.git_dir().join("index.lock")).unwrap();
    std::fs::write(a.git_dir().join("MERGE_HEAD"), "").unwrap();
    assert_eq!(sync(&a), Err(GitError::ExternalMerge));
    std::fs::remove_file(a.git_dir().join("MERGE_HEAD")).unwrap();
    raw(&a.root, &["checkout", "-q", "--detach"]);
    assert_eq!(sync(&a), Err(GitError::Detached));
}

#[test]
fn dangerous_arguments_are_refused() {
    let (a, _) = devices("refuse");
    for args in [["push", "--force"], ["reset", "--hard"], ["clean", "-fd"], ["stash", "push"], ["rebase", "main"], ["--no-pager", "clean"]] {
        assert!(matches!(a.run(&args), Err(GitError::Other(m)) if m.starts_with("refused")), "{args:?}");
    }
}

#[test]
fn commit_messages_are_never_mistaken_for_commands() {
    let (a, _) = devices("message");
    for (i, text) in ["clean", "--force", "stash", "# Überschrift", "-x", "   "].iter().enumerate() {
        std::fs::write(a.root.join(format!("{i}.md")), "x").unwrap();
        let report = a.sync(&|_| text.to_string(), &|_| {}).unwrap();
        assert!(report.pushed, "{text}");
        let subject = a.run(&["log", "-1", "--format=%s"]).unwrap();
        assert_eq!(subject.trim(), if text.trim().is_empty() { "Stand" } else { text });
    }
}

#[test]
fn a_deleted_upstream_branch_is_pushed_again() {
    let (a, b) = devices("gone");
    raw(&b.root, &["push", "-q", "origin", "HEAD:refs/heads/rest"]);
    raw(&b.root, &["remote", "set-head", "origin", "rest"]);
    raw(&a.root, &["config", "remote.origin.prune", "true"]);
    raw(&a.root.parent().unwrap().join("remote.git"), &["symbolic-ref", "HEAD", "refs/heads/rest"]);
    raw(&a.root, &["push", "-q", "origin", ":main"]);
    std::fs::write(a.root.join("x.md"), "x").unwrap();
    let report = sync(&a).unwrap();
    assert!(report.pushed);
    assert_eq!(a.status().unwrap().upstream.as_deref(), Some("origin/main"));
}

#[cfg(unix)]
#[test]
fn a_push_that_loses_the_race_tries_again() {
    use std::os::unix::fs::PermissionsExt;
    let (a, b) = devices("race");
    let hooks = a.root.parent().unwrap().join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("pre-push");
    std::fs::write(&hook, format!("#!/bin/sh\nunset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE\n[ -f ../once ] && exit 0\ntouch ../once\ncd {} && echo r > rennen.md && {} add -A && {} commit -qm r && {} push -q\n", b.root.display(), exe().display(), exe().display(), exe().display())).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    raw(&a.root, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    std::fs::write(a.root.join("meins.md"), "m").unwrap();
    let report = sync(&a).unwrap();
    assert!(report.pushed);
    assert_eq!(read(&a, "rennen.md").trim(), "r");
    sync(&b).unwrap();
    assert_eq!(read(&b, "meins.md"), "m");
}

#[test]
fn an_unreachable_remote_counts_as_offline_and_loses_nothing() {
    let (a, _) = devices("offline");
    raw(&a.root, &["remote", "set-url", "origin", "ssh://git@127.0.0.1:1/x.git"]);
    std::fs::write(a.root.join("x.md"), "x").unwrap();
    let report = sync(&a).unwrap();
    assert!(report.offline, "{report:?}");
    assert_eq!(report.committed, 1);
    assert!(a.status().unwrap().changed.is_empty());
}

#[test]
fn renames_from_the_other_device_report_both_paths() {
    let (a, b) = devices("rename");
    raw(&a.root, &["mv", "n.md", "umbenannt.md"]);
    sync(&a).unwrap();
    let report = sync(&b).unwrap();
    assert!(report.pulled.contains(&b.root.join("n.md")), "{:?}", report.pulled);
    assert!(report.pulled.contains(&b.root.join("umbenannt.md")));
}

#[test]
fn another_remote_name_works() {
    let (a, _) = devices("remote-name");
    raw(&a.root, &["remote", "rename", "origin", "hda"]);
    raw(&a.root, &["branch", "--unset-upstream"]);
    std::fs::write(a.root.join("x.md"), "x").unwrap();
    assert!(sync(&a).unwrap().pushed);
}

#[test]
fn decomposed_names_as_macos_may_store_them_work() {
    use unicode_normalization::UnicodeNormalization;
    let (a, b) = devices("nfd");
    let name: String = "Übung für Prüfung.md".nfd().collect();
    std::fs::write(a.root.join(&name), "x").unwrap();
    assert_eq!(sync(&a).unwrap().committed, 1);
    assert!(a.status().unwrap().changed.is_empty());
    sync(&b).unwrap();
    assert!(b.status().unwrap().changed.is_empty());
    assert_eq!(std::fs::read_dir(&b.root).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with("Prüfung.md") || e.as_ref().unwrap().file_name().to_string_lossy().contains("fu\u{308}r") || e.as_ref().unwrap().file_name().to_string_lossy().contains("für")).count(), 1);
}

/// Stands in for a second device in manual app tests: writes `SYNC_FILE` with `SYNC_TEXT` in `SYNC_DIR`, then syncs.
#[test]
#[ignore]
fn other_device() {
    let dir = PathBuf::from(std::env::var("SYNC_DIR").unwrap());
    if let (Ok(file), Ok(text)) = (std::env::var("SYNC_FILE"), std::env::var("SYNC_TEXT")) {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    configure(&dir);
    println!("{:?}", sync(&Git::new(exe(), &dir)));
}

/// Measures large repositories: `cargo test git::tests::scale -- --ignored --nocapture`.
#[test]
#[ignore]
fn scale() {
    let counts: Vec<usize> = std::env::var("SCALE").map_or(vec![1_000, 10_000, 100_000], |s| s.split(',').filter_map(|n| n.parse().ok()).collect());
    for count in counts {
        let (a, b) = devices(&format!("scale{count}"));
        for i in 0..count {
            let dir = a.root.join(format!("W{:02}", i % 40)).join(format!("Thema {}", i % 500));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("P1_26_Datei-mit-echtem-Namen-{i}_Muster-M_261031.md")), "x").unwrap();
        }
        let t = std::time::Instant::now();
        let n = a.status().unwrap().changed.len();
        let status = t.elapsed();
        let t = std::time::Instant::now();
        sync(&a).unwrap();
        let first = t.elapsed();
        let t = std::time::Instant::now();
        sync(&a).unwrap();
        let idle = t.elapsed();
        let t = std::time::Instant::now();
        let pulled = sync(&b).unwrap().pulled.len();
        let pull = t.elapsed();
        println!("{count}: changed {n}, status {status:?}, first sync {first:?}, idle sync {idle:?}, pull {pulled} in {pull:?}");
    }
}

fn clash(name: &str) -> (Git, Git) {
    let (a, b) = devices(name);
    std::fs::write(a.root.join("n.md"), "eins\nA\ndrei\n").unwrap();
    std::fs::write(a.root.join("b.pdf"), [0u8, 9]).unwrap();
    sync(&a).unwrap();
    std::fs::write(b.root.join("n.md"), "eins\nB\ndrei\n").unwrap();
    std::fs::write(b.root.join("b.pdf"), [0u8, 8]).unwrap();
    assert!(matches!(sync(&b), Err(GitError::Conflicts(_))));
    (a, b)
}

#[test]
fn conflicts_know_their_kind_and_nothing_changes() {
    let (_, b) = clash("kinds");
    let (files, automatic) = b.conflicts().unwrap();
    assert!(automatic.is_empty());
    let kind = |p: &str| files.iter().find(|f| f.path == p).unwrap().kind;
    assert_eq!(kind("n.md"), ConflictKind::Text);
    assert_eq!(kind("b.pdf"), ConflictKind::Binary);
    assert_eq!(read(&b, "n.md"), "eins\nB\ndrei\n");
    assert!(!b.git_dir().join("MERGE_HEAD").exists());
}

#[test]
fn keeping_both_makes_a_copy_with_the_other_version() {
    let (a, b) = clash("both");
    let choices: HashMap<String, Choice> = [("n.md".to_string(), Choice::Both), ("b.pdf".to_string(), Choice::Both)].into();
    let resolved = b.merge_resolved(&choices, &[], "Konflikte gelöst").unwrap();
    assert_eq!(resolved.copies.len(), 2);
    assert_eq!(read(&b, "n.md"), "eins\nB\ndrei\n");
    let md = resolved.copies.iter().find(|c| c.extension().is_some_and(|e| e == "md")).unwrap();
    assert_eq!(std::fs::read_to_string(md).unwrap(), "eins\nA\ndrei\n");
    let pdf = resolved.copies.iter().find(|c| c.extension().is_some_and(|e| e == "pdf")).unwrap();
    assert_eq!(std::fs::read(pdf).unwrap(), [0u8, 9]);
    assert_eq!(std::fs::read(b.root.join("b.pdf")).unwrap(), [0u8, 8]);
    assert!(sync(&b).unwrap().pushed);
    sync(&a).unwrap();
    assert_eq!(read(&a, "n.md"), "eins\nB\ndrei\n");
    assert!(a.status().unwrap().changed.is_empty());
}

#[test]
fn choosing_a_side_and_taking_it_back_as_a_new_state() {
    let (_, b) = clash("sides");
    let choices: HashMap<String, Choice> = [("n.md".to_string(), Choice::Theirs), ("b.pdf".to_string(), Choice::Mine)].into();
    let resolved = b.merge_resolved(&choices, &[], "gelöst").unwrap();
    assert_eq!(read(&b, "n.md"), "eins\nA\ndrei\n");
    assert_eq!(std::fs::read(b.root.join("b.pdf")).unwrap(), [0u8, 8]);
    sync(&b).unwrap();
    let before = log_count(&b);
    b.undo_resolution(&resolved, "rückgängig").unwrap();
    assert_eq!(read(&b, "n.md"), "eins\nB\ndrei\n");
    assert_eq!(log_count(&b), before + 1);
    assert!(sync(&b).unwrap().pushed);
}

#[test]
fn a_failed_resolution_leaves_everything_as_it_was() {
    let (_, b) = clash("abort");
    let head = b.run(&["rev-parse", "HEAD"]).unwrap();
    assert!(b.merge_resolved(&HashMap::new(), &[], "x").is_err());
    assert!(!b.git_dir().join("MERGE_HEAD").exists());
    assert_eq!(b.run(&["rev-parse", "HEAD"]).unwrap(), head);
    assert_eq!(read(&b, "n.md"), "eins\nB\ndrei\n");
    assert!(b.status().unwrap().changed.is_empty());
}

#[test]
fn deleted_on_one_side_can_be_kept_or_removed() {
    let (a, b) = devices("deleted");
    std::fs::remove_file(a.root.join("n.md")).unwrap();
    sync(&a).unwrap();
    std::fs::write(b.root.join("n.md"), "geändert").unwrap();
    assert!(matches!(sync(&b), Err(GitError::Conflicts(_))));
    let (files, _) = b.conflicts().unwrap();
    assert_eq!(files[0].kind, ConflictKind::DeletedThere);
    let keep: HashMap<String, Choice> = [("n.md".to_string(), Choice::Both)].into();
    b.merge_resolved(&keep, &[], "behalten").unwrap();
    assert_eq!(read(&b, "n.md"), "geändert");
    assert!(sync(&b).unwrap().pushed);
    sync(&a).unwrap();
    std::fs::write(a.root.join("n.md"), "wieder").unwrap();
    sync(&a).unwrap();
    std::fs::remove_file(b.root.join("n.md")).unwrap();
    assert!(matches!(sync(&b), Err(GitError::Conflicts(_))));
    assert_eq!(b.conflicts().unwrap().0[0].kind, ConflictKind::DeletedHere);
    let remove: HashMap<String, Choice> = [("n.md".to_string(), Choice::Mine)].into();
    b.merge_resolved(&remove, &[], "weg").unwrap();
    assert!(!b.root.join("n.md").exists());
}

#[test]
fn the_apps_own_files_merge_without_asking() {
    let (a, b) = devices("own");
    let state = |overrides: &str| format!("{{\"overrides\":{{{overrides}}}}}");
    std::fs::write(a.root.join(".schrumbi-state.json"), state("")).unwrap();
    sync(&a).unwrap();
    sync(&b).unwrap();
    std::fs::write(a.root.join(".schrumbi-state.json"), state("\"e1\":{\"status\":\"done\",\"date\":\"2026-10-01T10:00:00Z\"}")).unwrap();
    sync(&a).unwrap();
    std::fs::write(b.root.join(".schrumbi-state.json"), state("\"e2\":{\"status\":\"done\",\"date\":\"2026-10-01T10:00:00Z\"}")).unwrap();
    let report = sync(&b).unwrap();
    assert!(report.pushed);
    let merged = read(&b, ".schrumbi-state.json");
    assert!(merged.contains("e1") && merged.contains("e2"), "{merged}");
    sync(&a).unwrap();
    assert!(read(&a, ".schrumbi-state.json").contains("e2"));
}

#[test]
fn a_real_config_contradiction_is_asked_about() {
    let (a, b) = devices("config");
    let config = |name: &str| format!("{{\"version\":2,\"name\":\"{name}\",\"semesters\":[],\"modules\":[]}}");
    std::fs::write(a.root.join("schrumbi.json"), config("S")).unwrap();
    sync(&a).unwrap();
    sync(&b).unwrap();
    std::fs::write(a.root.join("schrumbi.json"), config("A")).unwrap();
    sync(&a).unwrap();
    std::fs::write(b.root.join("schrumbi.json"), config("B")).unwrap();
    assert!(matches!(sync(&b), Err(GitError::Conflicts(f)) if f == ["schrumbi.json"]));
}

#[test]
fn stages_parse() {
    let out = "abc\0100644 aaa 1\tn.md\0100644 bbb 2\tn.md\0100644 ccc 3\tn.md\0100644 ddd 2\tweg.md\0\0Auto-merging\0";
    let files = conflict::parse_stages(out);
    assert_eq!(files["n.md"], [Some("aaa".to_string()), Some("bbb".to_string()), Some("ccc".to_string())]);
    assert_eq!(files["weg.md"], [None, Some("ddd".to_string()), None]);
}

#[test]
fn setup_finds_and_cleans_up() {
    let (a, b) = devices("setup");
    std::fs::create_dir_all(a.root.join(".schrumbi-backup")).unwrap();
    std::fs::write(a.root.join(".schrumbi-backup/x-1.json"), "{}").unwrap();
    raw(&a.root, &["add", "-f", ".schrumbi-backup"]);
    raw(&a.root, &["commit", "-q", "-m", "backup"]);
    let inner = a.root.join("Projekt");
    std::fs::create_dir_all(&inner).unwrap();
    raw(&inner, &["init", "-q"]);
    configure(&inner);
    std::fs::write(inner.join("i"), "i").unwrap();
    raw(&inner, &["add", "-A"]);
    raw(&inner, &["commit", "-q", "-m", "i"]);
    let found = setup::inspect(&a);
    assert!(found.has_repo);
    assert!(found.remote.is_some());
    assert!(found.has_identity);
    assert_eq!(found.tracked_backups, 1);
    assert_eq!(found.nested, ["Projekt"]);
    assert_eq!(found.missing_ignores.len(), setup::IGNORE_LINES.len());
    let state = a.clean_up(&found.missing_ignores, &found.nested, true, "Aufräumen").unwrap().unwrap();
    assert!(a.root.join(".schrumbi-backup/x-1.json").exists());
    assert!(!a.run(&["ls-files"]).unwrap().contains("schrumbi-backup"));
    assert!(read(&a, ".gitignore").contains("Projekt/"));
    assert!(a.status().unwrap().changed.is_empty());
    assert!(setup::inspect(&a).missing_ignores.is_empty());
    assert_eq!(a.clean_up(&[], &[], true, "nochmal").unwrap(), None);
    a.undo_clean_up(&state).unwrap();
    assert!(a.run(&["ls-files"]).unwrap().contains("schrumbi-backup"));
    assert!(a.root.join(".schrumbi-backup/x-1.json").exists());
    let _ = b;
}

#[test]
fn setup_recognises_special_places() {
    assert_eq!(setup::cloud_service(Path::new("/Users/x/Library/Mobile Documents/com~apple~CloudDocs/Uni")), Some("iCloud"));
    assert_eq!(setup::cloud_service(Path::new("/Users/x/Dropbox/Uni")), Some("Dropbox"));
    assert_eq!(setup::cloud_service(Path::new("/Users/x/UXD")), None);
    let dir = fresh("plugin");
    assert!(!setup::obsidian_git_enabled(&dir));
    std::fs::create_dir_all(dir.join(".obsidian")).unwrap();
    std::fs::write(dir.join(".obsidian/community-plugins.json"), "[\"obsidian-git\"]").unwrap();
    assert!(setup::obsidian_git_enabled(&dir));
    let nested = dir.join("innen");
    std::fs::create_dir_all(&nested).unwrap();
    raw(&dir, &["init", "-q"]);
    assert!(setup::inspect(&Git::new(exe(), &nested)).outer_root.is_some());
}

#[test]
fn a_new_folder_can_be_set_up_from_nothing() {
    let dir = fresh("init");
    let git = Git::new(exe(), &dir);
    assert!(!setup::inspect(&git).has_repo);
    git.init_repo().unwrap();
    assert!(setup::inspect(&git).has_repo);
    git.set_identity("Muster", "m@example.invalid").unwrap();
    assert_eq!(git.run(&["config", "user.name"]).unwrap().trim(), "Muster");
    git.set_remote("/a").unwrap();
    git.set_remote("/b").unwrap();
    assert_eq!(setup::inspect(&git).remote.as_deref(), Some("/b"));
}

#[test]
fn testing_a_remote_says_whether_it_answers() {
    let (a, _) = devices("test-remote");
    let url = a.root.parent().unwrap().join("remote.git");
    assert_eq!(a.test_remote(url.to_str().unwrap()), Ok(()));
    assert!(a.test_remote("/gibt/es/nicht.git").is_err());
    let empty = a.root.parent().unwrap().join("leer.git");
    raw(a.root.parent().unwrap(), &["init", "-q", "--bare", "leer.git"]);
    assert_eq!(a.test_remote(empty.to_str().unwrap()), Ok(()));
}

#[test]
fn history_lists_versions_across_renames_and_returns_their_bytes() {
    let (a, _) = devices("history");
    std::fs::write(a.root.join("n.md"), "zwei").unwrap();
    sync(&a).unwrap();
    raw(&a.root, &["mv", "n.md", "umbenannt.md"]);
    sync(&a).unwrap();
    std::fs::write(a.root.join("b.pdf"), [9u8, 0, 9]).unwrap();
    sync(&a).unwrap();
    let list = a.versions("umbenannt.md", 0, 100).unwrap();
    assert_eq!(list.len(), 3, "{list:?}");
    assert_eq!(list[0].path, "umbenannt.md");
    assert_eq!(list[2].path, "n.md");
    assert_eq!(a.version_bytes(&list[2]).unwrap(), b"eins\nzwei\ndrei\n");
    assert_eq!(a.version_bytes(&list[1]).unwrap(), b"zwei");
    assert_eq!(a.versions("umbenannt.md", 1, 1).unwrap().len(), 1);
    assert_eq!(a.version_bytes(&a.versions("b.pdf", 1, 1).unwrap()[0]).unwrap(), [0u8, 1, 2]);
    assert!(a.versions("gibt-es-nicht.md", 0, 10).unwrap().is_empty());
}

#[test]
fn errors_are_sorted_by_what_can_be_done() {
    assert_eq!(classify("git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository."), GitError::Auth);
    assert_eq!(classify("ssh: Could not resolve hostname: nodename\nfatal: Could not read from remote repository."), GitError::Offline);
    assert!(matches!(classify("fatal: '/x.git' does not appear to be a git repository\nfatal: Could not read from remote repository."), GitError::Other(_)));
    assert_eq!(classify("fatal: unable to access 'x': Could not resolve host: github.com"), GitError::Offline);
    assert_eq!(classify("! [rejected] main -> main (fetch first)"), GitError::Rejected);
    assert_eq!(classify("fatal: Unable to create '.git/index.lock': File exists."), GitError::Locked);
    assert_eq!(classify("Please tell me who you are."), GitError::NoIdentity);
    assert_eq!(classify("etwas anderes"), GitError::Other("etwas anderes".into()));
}

#[test]
fn versions_are_checked() {
    assert_eq!(locate::parse_version("git version 2.50.1 (Apple Git-155)").as_deref(), Some("2.50.1"));
    assert!(locate::at_least("2.38.0"));
    assert!(locate::at_least("3.0"));
    assert!(!locate::at_least("2.37.9"));
}

#[test]
fn status_parses_every_kind_of_line() {
    let out = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -3\0\
1 .M N... 100644 100644 100644 a a m.md\0\
1 .D N... 100644 100644 000000 a a mit leer zeichen.md\0\
1 A. N... 000000 100644 100644 0 a neu.md\0\
2 R. N... 100644 100644 100644 a a R100 ziel.md\0quelle.md\0\
u UU N... 100644 100644 100644 100644 a b c k.md\0\
1 .M S.M. 160000 160000 160000 a a sub\0\
? frei.md\0? repo/\0";
    let s = parse(out);
    assert_eq!(s.branch.as_deref(), Some("main"));
    assert_eq!((s.ahead, s.behind), (2, 3));
    assert_eq!(s.nested, ["sub", "repo"]);
    let kinds: Vec<(&str, &ChangeKind)> = s.changed.iter().map(|c| (c.path.as_str(), &c.kind)).collect();
    assert_eq!(kinds, [("m.md", &ChangeKind::Modified), ("mit leer zeichen.md", &ChangeKind::Deleted), ("neu.md", &ChangeKind::New), ("ziel.md", &ChangeKind::Renamed { from: "quelle.md".into() }), ("k.md", &ChangeKind::Conflicted), ("frei.md", &ChangeKind::New)]);
    assert_eq!(parse("# branch.head (detached)\0").branch, None);
}

#[test]
fn upload_progress_is_read_from_gits_output() {
    use super::cmd::upload_percent;
    assert_eq!(upload_percent("Writing objects:  45% (9/20)\rWriting objects:  50% (10/20)\r"), Some(50));
    assert_eq!(upload_percent("Writing objects: 100% (20/20), 1 MiB, done.\n"), Some(100));
    assert_eq!(upload_percent("Enumerating objects: 3, done.\n"), None);
}

#[cfg(unix)]
#[test]
fn a_helper_holding_the_pipes_does_not_hang_us() {
    let mut c = Command::new("/bin/sh");
    c.args(["-c", "echo fertig; (sleep 30) & exit 0"]);
    let start = SystemTime::now();
    let out = spawn(c, None, Duration::from_secs(20)).unwrap();
    assert_eq!(out.stdout.trim(), "fertig");
    assert!(start.elapsed().unwrap() < Duration::from_secs(8));
}

#[cfg(unix)]
#[test]
fn hanging_commands_are_killed() {
    let mut c = Command::new("/bin/sleep");
    c.arg("30");
    let start = SystemTime::now();
    assert!(matches!(spawn(c, None, Duration::from_millis(100)), Err(GitError::Timeout)));
    assert!(start.elapsed().unwrap() < Duration::from_secs(5));
}
