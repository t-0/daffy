use super::*;
#[test]
fn split_command_handles_quotes_and_escapes() {
    assert_eq!(split_command("ed --v"), ["ed", "--v"]);
    assert_eq!(
        split_command(r#"open -a "Final Cut Pro" {file}"#),
        ["open", "-a", "Final Cut Pro", "{file}"]
    );
    assert_eq!(split_command("x 'a b' c\\ d"), ["x", "a b", "c d"]);
    assert_eq!(split_command("   "), Vec::<String>::new());
    assert_eq!(split_command("\"\""), [""]);
}
#[test]
fn kv_parser_strips_quotes_and_comments() {
    let map = parse_kv("# c\nvideo.home = \"~/Movies\"\n\nindex.file='x'\nbad line\n");
    assert_eq!(map["video.home"], "~/Movies");
    assert_eq!(map["index.file"], "x");
    assert_eq!(map.len(), 2);
}
#[test]
fn unknown_keys_are_reported_with_a_hint() {
    let mut map = HashMap::new();
    map.insert("video.hom".to_string(), "x".to_string());
    let err = check_keys(&map, Path::new("/c")).unwrap_err().to_string();
    assert!(err.contains("video.hom"), "{err}");
    assert!(err.contains("did you mean `video.home`"), "{err}");
    let mut map = HashMap::new();
    map.insert("video.home".to_string(), "x".to_string());
    map.insert("ffmpeg.dir".to_string(), "x".to_string());
    assert!(check_keys(&map, Path::new("/c")).is_ok());
}
#[test]
fn editor_mode_parses() {
    assert_eq!(EditorMode::parse("gui").unwrap(), EditorMode::Gui);
    assert_eq!(EditorMode::parse(" CLI ").unwrap(), EditorMode::Cli);
    assert_eq!(EditorMode::parse("terminal").unwrap(), EditorMode::Cli);
    assert!(EditorMode::parse("web").is_err());
}
#[test]
fn tool_lookup_prefers_the_configured_dir() {
    let p = find_tool("ffmpeg", Some(Path::new("/x/bin")));
    assert_eq!(p, PathBuf::from("/x/bin/ffmpeg"));
    // Without a dir the result is either a bare name (found on PATH) or an
    // absolute fallback; never empty.
    let p = find_tool("ffmpeg", None);
    assert!(p.file_name().is_some());
}
