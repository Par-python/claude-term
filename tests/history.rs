use ct::history::{ScanCache, parse_session, scan_projects};
use std::fs;
use std::path::{Path, PathBuf};

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    fs::write(&p, body).unwrap();
    p
}

#[test]
fn title_from_latest_custom_title() {
    let d = tempfile::tempdir().unwrap();
    let body = concat!(
        "{\"type\":\"summary\",\"cwd\":\"/Users/me/proj\"}\n",
        "{\"type\":\"custom-title\",\"customTitle\":\"Old\",\"sessionId\":\"abc\"}\n",
        "{\"type\":\"custom-title\",\"customTitle\":\"New\",\"sessionId\":\"abc\"}\n",
    );
    let p = write(d.path(), "abc-123.jsonl", body);
    let m = parse_session(&p).unwrap();
    assert_eq!(m.title, "New");
    assert_eq!(m.cwd, PathBuf::from("/Users/me/proj"));
    assert_eq!(m.id, "abc-123");
}

#[test]
fn title_falls_back_to_first_user_message() {
    let d = tempfile::tempdir().unwrap();
    let long = "fix the login bug please ".repeat(10);
    let body = format!(
        "{{\"type\":\"user\",\"cwd\":\"/Users/me/proj\",\"message\":{{\"role\":\"user\",\"content\":\"<command-name>x</command-name>\"}}}}\n{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{long}\"}}}}\n"
    );
    let p = write(d.path(), "s1.jsonl", &body);
    let m = parse_session(&p).unwrap();
    assert!(m.title.starts_with("fix the login bug"));
    assert!(m.title.chars().count() <= 60);

    let body2 = "{\"type\":\"user\",\"cwd\":\"/a\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hello from array\"}]}}\n";
    let p2 = write(d.path(), "s2.jsonl", body2);
    assert_eq!(parse_session(&p2).unwrap().title, "hello from array");
}

#[test]
fn skips_bad_lines() {
    let d = tempfile::tempdir().unwrap();
    let body = "not json at all\n{\"type\":\"custom-title\",\"customTitle\":\"T\",\"cwd\":\"/a/b\"}\n{\"type\":\"user\",\"mess";
    let p = write(d.path(), "s.jsonl", body);
    let m = parse_session(&p).unwrap();
    assert_eq!(m.title, "T");
    assert_eq!(m.cwd, PathBuf::from("/a/b"));
}

#[test]
fn no_cwd_returns_none() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "s.jsonl", "{\"type\":\"custom-title\",\"customTitle\":\"T\"}\n");
    assert!(parse_session(&p).is_none());
}

#[test]
fn empty_file_returns_none() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "s.jsonl", "");
    assert!(parse_session(&p).is_none());
}

#[test]
fn scan_skips_temp_dirs() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("-Users-me-proj");
    write(&a, "good.jsonl", "{\"cwd\":\"/Users/me/proj\",\"customTitle\":\"G\",\"type\":\"custom-title\"}\n");
    write(&a, "notes.txt", "{\"cwd\":\"/Users/me/proj\"}\n");
    write(&a.join("memory"), "m.jsonl", "{\"cwd\":\"/Users/me/proj\"}\n");
    let t = d.path().join("-private-tmp-x");
    write(&t, "tmp.jsonl", "{\"cwd\":\"/private/tmp/x\"}\n");
    write(&t, "tmp2.jsonl", "{\"cwd\":\"/tmp/y\"}\n");
    write(&t, "tmp3.jsonl", "{\"cwd\":\"/var/folders/z\"}\n");
    let got = scan_projects(d.path());
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, "good");
}

#[test]
fn array_content_skips_leading_tag_block() {
    let d = tempfile::tempdir().unwrap();
    let body = "{\"type\":\"user\",\"cwd\":\"/a\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"<system-reminder>x</system-reminder>\"},{\"type\":\"text\",\"text\":\"the real prompt\"}]}}\n";
    let p = write(d.path(), "s.jsonl", body);
    assert_eq!(parse_session(&p).unwrap().title, "the real prompt");
}

#[test]
fn custom_title_is_normalized() {
    let d = tempfile::tempdir().unwrap();
    let body = "{\"type\":\"custom-title\",\"cwd\":\"/a\",\"customTitle\":\"  my\\n  title  \"}\n";
    let p = write(d.path(), "s.jsonl", body);
    assert_eq!(parse_session(&p).unwrap().title, "my title");
}

#[test]
fn blank_custom_title_falls_back() {
    let d = tempfile::tempdir().unwrap();
    let body = concat!(
        "{\"type\":\"user\",\"cwd\":\"/a\",\"message\":{\"role\":\"user\",\"content\":\"hello there\"}}\n",
        "{\"type\":\"custom-title\",\"customTitle\":\"   \"}\n",
    );
    let p = write(d.path(), "s.jsonl", body);
    assert_eq!(parse_session(&p).unwrap().title, "hello there");
}

fn session_line(cwd: &str) -> String {
    format!("{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":{{\"content\":\"hi\"}}}}\n")
}

#[test]
fn rescan_of_unchanged_dir_parses_nothing() {
    let d = tempfile::tempdir().unwrap();
    write(&d.path().join("p"), "a.jsonl", &session_line("/work/a"));
    write(&d.path().join("p"), "b.jsonl", &session_line("/work/b"));
    let mut c = ScanCache::default();
    assert_eq!(c.scan(d.path()).len(), 2);
    assert_eq!(c.parse_count, 2);
    assert_eq!(c.scan(d.path()).len(), 2);
    assert_eq!(c.parse_count, 2);
}

#[test]
fn appending_reparses_only_that_file() {
    use std::io::Write;
    let d = tempfile::tempdir().unwrap();
    let a = write(&d.path().join("p"), "a.jsonl", &session_line("/work/a"));
    write(&d.path().join("p"), "b.jsonl", &session_line("/work/b"));
    let mut c = ScanCache::default();
    c.scan(d.path());
    let mut f = fs::OpenOptions::new().append(true).open(&a).unwrap();
    writeln!(f, "{{\"type\":\"custom-title\",\"customTitle\":\"Renamed\"}}").unwrap();
    drop(f);
    let out = c.scan(d.path());
    assert_eq!(c.parse_count, 3);
    assert_eq!(out.iter().find(|m| m.id == "a").unwrap().title, "Renamed");
}

#[test]
fn deleted_file_disappears_from_cache_scan() {
    let d = tempfile::tempdir().unwrap();
    let a = write(&d.path().join("p"), "a.jsonl", &session_line("/work/a"));
    write(&d.path().join("p"), "b.jsonl", &session_line("/work/b"));
    let mut c = ScanCache::default();
    assert_eq!(c.scan(d.path()).len(), 2);
    fs::remove_file(a).unwrap();
    let out = c.scan(d.path());
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].id, "b");
}

fn append(p: &Path, text: &str) {
    use std::io::Write;
    let mut f = fs::OpenOptions::new().append(true).open(p).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn root_with(body: &str) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/target")).unwrap();
    let p = write(&d.path().join("proj"), "s1.jsonl", body);
    (d, p)
}

#[test]
fn appended_lines_are_the_only_bytes_read() {
    let body = "{\"type\":\"user\",\"cwd\":\"/Users/me/proj\",\"message\":{\"content\":\"hello\"}}\n";
    let (d, p) = root_with(body);
    let mut c = ScanCache::default();
    let m = c.scan(d.path());
    assert_eq!(m[0].title, "hello");
    assert_eq!(c.bytes_read, body.len() as u64);
    let add = "{\"type\":\"custom-title\",\"customTitle\":\"Renamed\"}\n";
    append(&p, add);
    let m = c.scan(d.path());
    assert_eq!(m[0].title, "Renamed");
    assert_eq!(m[0].cwd, PathBuf::from("/Users/me/proj"));
    assert_eq!(c.bytes_read, (body.len() + add.len()) as u64, "only the new bytes should be read");
    // unchanged file: nothing is read
    c.scan(d.path());
    assert_eq!(c.bytes_read, (body.len() + add.len()) as u64);
}

#[test]
fn a_half_written_last_line_is_finished_on_the_next_scan() {
    let body = "{\"type\":\"user\",\"cwd\":\"/a\",\"message\":{\"content\":\"hello\"}}\n";
    let (d, p) = root_with(body);
    let mut c = ScanCache::default();
    c.scan(d.path());
    append(&p, "{\"type\":\"custom-title\",\"custom");
    let m = c.scan(d.path());
    assert_eq!(m[0].title, "hello");
    append(&p, "Title\":\"Done\"}\n");
    let before = c.bytes_read;
    let m = c.scan(d.path());
    assert_eq!(m[0].title, "Done");
    // only the unfinished line was read again, not the whole file
    assert!(c.bytes_read - before < 60, "{}", c.bytes_read - before);
}

#[test]
fn a_shrunk_or_rewritten_file_is_parsed_from_scratch() {
    let body = "{\"type\":\"user\",\"cwd\":\"/a\",\"message\":{\"content\":\"hello there\"}}\n{\"type\":\"custom-title\",\"customTitle\":\"One\"}\n";
    let (d, p) = root_with(body);
    let mut c = ScanCache::default();
    assert_eq!(c.scan(d.path())[0].title, "One");
    // truncated and replaced with something shorter
    let short = "{\"type\":\"user\",\"cwd\":\"/b\",\"message\":{\"content\":\"hi\"}}\n";
    fs::write(&p, short).unwrap();
    let before = c.bytes_read;
    let m = c.scan(d.path());
    assert_eq!(m[0].title, "hi");
    assert_eq!(m[0].cwd, PathBuf::from("/b"));
    assert_eq!(c.bytes_read - before, short.len() as u64);
    // same length, newer mtime: also a full re-parse
    let same = short.replace("hi", "yo");
    fs::write(&p, &same).unwrap();
    let f = fs::File::options().write(true).open(&p).unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
    let before = c.bytes_read;
    assert_eq!(c.scan(d.path())[0].title, "yo");
    assert_eq!(c.bytes_read - before, same.len() as u64);
}
