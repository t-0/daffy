use super::*;
fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("daffy-index-{}-{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
fn names(files: &[PathBuf]) -> Vec<String> {
    let mut v: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}
#[test]
fn walk_finds_videos_recursively_and_skips_hidden_entries() {
    let root = temp("walk");
    fs::create_dir_all(root.join("a/b")).unwrap();
    fs::create_dir_all(root.join(".hidden")).unwrap();
    for f in [
        "one.mp4",
        "a/two.MKV",
        "a/b/three.webm",
        "a/notes.txt",
        "a/song.mp3",
        ".hidden/four.mp4",
        "a/.five.mov",
    ] {
        fs::write(root.join(f), b"").unwrap();
    }
    let (mut files, mut unreadable) = (Vec::new(), Vec::new());
    walk(&root, &mut files, &mut unreadable);
    assert_eq!(names(&files), ["one.mp4", "three.webm", "two.MKV"]);
    assert!(unreadable.is_empty(), "{unreadable:?}");
}
#[cfg(unix)]
#[test]
fn walk_follows_symlinks_once_and_survives_loops() {
    use std::os::unix::fs::symlink;
    let root = temp("links");
    fs::create_dir_all(root.join("real")).unwrap();
    fs::write(root.join("real/clip.mp4"), b"").unwrap();
    symlink(root.join("real"), root.join("alias")).unwrap();
    symlink(&root, root.join("real/loop")).unwrap();
    symlink(root.join("missing"), root.join("dangling")).unwrap();
    let (mut files, mut unreadable) = (Vec::new(), Vec::new());
    walk(&root, &mut files, &mut unreadable);
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(names(&files), ["clip.mp4"]);
    assert_eq!(unreadable.len(), 1, "{unreadable:?}");
    assert!(unreadable[0].ends_with("dangling"));
}
#[test]
fn folder_and_collection_come_from_the_path() {
    let root = Path::new("/home/me/Movies");
    assert_eq!(
        locate(root, Path::new("/home/me/Movies/TV/Show/S1/e1.mkv")),
        ("TV/Show/S1".to_string(), "TV".to_string())
    );
    assert_eq!(
        locate(root, Path::new("/home/me/Movies/Music Videos/x.m4v")),
        ("Music Videos".to_string(), "Music Videos".to_string())
    );
    assert_eq!(
        locate(root, Path::new("/home/me/Movies/top.mp4")),
        ("Movies".to_string(), "Movies".to_string())
    );
    // A file outside the root (an odd symlink) still gets something sane.
    let (f, c) = locate(root, Path::new("/elsewhere/clip.mp4"));
    assert_eq!(f, "elsewhere");
    assert_eq!(c, "elsewhere");
}
#[test]
fn bare_video_uses_the_file_name_and_extension() {
    let v = bare_video(Path::new("/x/My Clip.MOV"), 5, 6);
    assert_eq!(v.title, "My Clip");
    assert_eq!(v.filetype, "mov");
    assert!(!v.has_video() && !v.has_audio());
    assert_eq!((v.mtime, v.size), (5, 6));
}
#[test]
fn sort_key_orders_by_collection_folder_then_file_name() {
    let mk = |p: &str| {
        let mut v = bare_video(Path::new(p), 0, 0);
        v.locate(Path::new("/m"));
        v
    };
    let mut vs = [
        mk("/m/TV/show/02.mkv"),
        mk("/m/Films/b.mp4"),
        mk("/m/TV/show/01.mkv"),
        mk("/m/a.mp4"),
    ];
    vs.sort_by_cached_key(|v| v.sort_key());
    let names: Vec<String> = vs.iter().map(|v| v.file_name()).collect();
    // Top-level files group under the root's own name, "m".
    assert_eq!(names, ["b.mp4", "a.mp4", "01.mkv", "02.mkv"]);
}
#[test]
fn index_round_trips_through_json_with_missing_fields() {
    let v = bare_video(Path::new("/x/a.mp4"), 1, 2);
    let json = serde_json::to_string(&Index { videos: vec![v] }).unwrap();
    let idx: Index = serde_json::from_str(&json).unwrap();
    assert_eq!(idx.videos[0].title, "a");
    // Older or hand-edited files may lack the newer fields.
    let idx: Index = serde_json::from_str(
        r#"{"videos":[{"path":"/x/b.mp4","mtime":0,"size":0,"title":"b","year":null,"duration_secs":1.5,"filetype":"mp4"}]}"#,
    )
    .unwrap();
    assert_eq!(idx.videos[0].width, 0);
    assert_eq!(idx.videos[0].acodec, None);
}
