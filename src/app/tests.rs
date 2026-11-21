use super::settings::settings_file;
use super::*;
use crate::video::Protocol;
use std::path::PathBuf;
use std::time::Instant as TInstant;
fn temp_config(tag: &str) -> Config {
    let dir = std::env::temp_dir().join(format!("daffy-test-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).unwrap();
    Config {
        video_home: dir.clone(),
        index_file: dir.join("index.json"),
        video_editor: None,
        video_editor_mode: EditorMode::Gui,
        renderer: None,
        ffmpeg: PathBuf::from("ffmpeg"),
        ffprobe: PathBuf::from("ffprobe"),
    }
}
fn renderer() -> Renderer {
    Renderer {
        protocol: Protocol::Blocks,
        truecolor: true,
        cell: (1, 2),
        cell_known: false,
    }
}
fn video(collection: &str, folder: &str, title: &str, n: u32) -> Video {
    let dir = if folder.is_empty() {
        collection.to_string()
    } else {
        format!("{collection}/{folder}")
    };
    let mut v = Video {
        path: PathBuf::from(format!("/m/{dir}/{n:02} {title}.mp4")),
        mtime: n as u64,
        size: 1,
        title: title.into(),
        folder: String::new(),
        collection: String::new(),
        year: None,
        duration_secs: 180.0 + n as f64,
        width: 1280,
        height: 720,
        fps: 25.0,
        vcodec: "h264".into(),
        acodec: Some("aac".into()),
        channels: 2,
        sample_rate: 48_000,
        filetype: "mp4".into(),
        gain_db: None,
    };
    v.locate(std::path::Path::new("/m"));
    v
}
fn args_of(c: &Command) -> Vec<String> {
    c.get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}
#[test]
fn editor_file_goes_last_unless_placed() {
    let p = std::path::Path::new("/m/a b/x.mp4");
    let w = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
    let c = editor_command(&w("ed --v"), p);
    assert_eq!(c.get_program(), "ed");
    assert_eq!(args_of(&c), ["--v", "/m/a b/x.mp4"]);
    let c = editor_command(&w("ed --in={file} --out={file}.bak last"), p);
    assert_eq!(
        args_of(&c),
        ["--in=/m/a b/x.mp4", "--out=/m/a b/x.mp4.bak", "last"]
    );
    let c = editor_command(&w("ed {file}"), p);
    assert_eq!(args_of(&c), ["/m/a b/x.mp4"]);
}
#[test]
fn measured_gain_survives_reindex_and_feeds_preamp() {
    let mut app = app_with("gain", library());
    app.videos[3].gain_db = Some(-4.5);
    let path = app.videos[3].path.clone();
    app.set_videos(library());
    let t = app.videos.iter().find(|t| t.path == path).unwrap();
    assert_eq!(t.gain_db, Some(-4.5));
    let mut changed = library();
    changed.iter_mut().find(|t| t.path == path).unwrap().size = 2;
    app.set_videos(changed);
    let t = app.videos.iter().find(|t| t.path == path).unwrap();
    assert_eq!(t.gain_db, None, "a changed file needs a new measurement");
    // The gain only reaches the FX chain while normalisation is on and
    // that video is playing.
    app.videos
        .iter_mut()
        .find(|t| t.path == path)
        .unwrap()
        .gain_db = Some(-4.5);
    let idx = app.videos.iter().position(|t| t.path == path).unwrap();
    app.playing = Some(idx);
    app.sync_track_gain();
    assert_eq!(app.fx.track_gain, 0.0);
    assert!(matches!(app.normalize_state(), NormalizeState::Off));
    app.set_normalize(true);
    assert_eq!(app.fx.track_gain, -4.5);
    assert!(app.measuring.is_none());
    assert!(matches!(app.normalize_state(), NormalizeState::Gain(g) if g == -4.5));
    app.playing = Some((idx + 1) % app.videos.len());
    app.sync_track_gain();
    assert_eq!(app.fx.track_gain, 0.0);
    assert!(
        app.measuring.is_some(),
        "an unmeasured video starts a measurement"
    );
    assert!(matches!(app.normalize_state(), NormalizeState::Measuring));
    app.set_normalize(false);
    assert_eq!(app.fx.track_gain, 0.0);
}
#[test]
fn silent_videos_are_not_measured() {
    let mut lib = library();
    lib[0].acodec = None;
    let mut app = app_with("silent", lib);
    let idx = app.videos.iter().position(|v| v.acodec.is_none()).unwrap();
    app.playing = Some(idx);
    app.set_normalize(true);
    assert!(app.measuring.is_none());
}
#[test]
fn reindex_keeps_selection_and_scroll() {
    let mut app = app_with("keep", library());
    app.move_cursor(7);
    app.offsets = [3, 1, 0];
    let before = app.view_videos[app.cursor_video];
    let path = app.videos[before].path.clone();
    app.section = Section::Folders;
    app.move_cursor(2);
    let folder = app.folders[app.view_folders[app.cursor_folder]].name.clone();
    // Rebuild from a copy with one extra video at the front of the order.
    let mut videos = library();
    videos.push(video("AAA First", "Early", "Opener", 1));
    videos.sort_by_cached_key(|t| t.sort_key());
    app.set_videos(videos);
    assert_eq!(app.videos[app.view_videos[app.cursor_video]].path, path);
    assert_eq!(app.folders[app.view_folders[app.cursor_folder]].name, folder);
    assert_eq!(app.offsets, [3, 1, 0]);
    assert_eq!(app.recenter, [false; 3]);
}
fn app_with(tag: &str, mut videos: Vec<Video>) -> App {
    videos.sort_by_cached_key(|t| t.sort_key());
    let mut app = App::build(temp_config(tag), FxParams::default(), None, None, renderer());
    app.set_videos(videos);
    app
}
fn library() -> Vec<Video> {
    let mut v = Vec::new();
    for (co, fo, titles) in [
        ("Films", "", vec!["Alpha", "Beta"]),
        ("TV", "Show A/Season 1", vec!["Pilot", "Second", "Third"]),
        ("TV", "Show A/Season 2", vec!["Return", "Finale"]),
        ("TV", "Show B", vec!["Only"]),
        ("Home", "2007 Lauren", vec!["Beach", "Birthday", "Cake"]),
        ("Home", "2010", vec!["Snow"]),
    ] {
        for (n, t) in titles.iter().enumerate() {
            v.push(video(co, fo, t, n as u32 + 1));
        }
    }
    v
}
fn invariants(app: &App) {
    assert!(app.cursor_video < app.view_videos.len().max(1));
    assert!(app.cursor_folder < app.view_folders.len().max(1));
    assert!(app.cursor_collection < app.view_collections.len().max(1));
    for &i in &app.view_videos {
        assert!(i < app.videos.len());
    }
    if let Some(p) = app.playing {
        assert!(p < app.videos.len());
    }
    let (f, c) = (&app.folders, &app.collections);
    for w in f.windows(2) {
        assert_eq!(w[0].end, w[1].start);
    }
    for w in c.windows(2) {
        assert_eq!(w[0].folder_end, w[1].folder_start);
        assert_eq!(w[0].video_end, w[1].video_start);
    }
}
#[test]
fn library_groups_into_folders_and_collections() {
    let app = app_with("group", library());
    assert_eq!(app.videos.len(), 12);
    assert_eq!(app.folders.len(), 6);
    assert_eq!(app.collections.len(), 3);
    let names: Vec<&str> = app.collections.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Films", "Home", "TV"]);
    let tv = &app.collections[2];
    assert_eq!(tv.folder_end - tv.folder_start, 3);
    assert_eq!(tv.video_end - tv.video_start, 6);
    let folders: Vec<&str> = app.folders[tv.folder_start..tv.folder_end]
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(folders, ["TV/Show A/Season 1", "TV/Show A/Season 2", "TV/Show B"]);
    assert_eq!(app.folders[0].name, "Films");
    invariants(&app);
}
#[test]
fn search_starts_at_top_when_playing_video_is_filtered_out() {
    let mut app = app_with("s1", library());
    let beach = app.videos.iter().position(|t| t.title == "Beach").unwrap();
    app.play_index(beach);
    app.search = "show a".into();
    app.apply_search();
    assert_eq!(app.view_videos.len(), 5);
    assert_eq!(app.cursor_video, 0);
    assert_eq!(app.cursor_folder, 0);
    invariants(&app);
}
#[test]
fn search_follows_playing_video_when_it_matches() {
    let mut app = app_with("s2", library());
    let finale = app.videos.iter().position(|t| t.title == "Finale").unwrap();
    app.play_index(finale);
    app.search = "season".into();
    app.apply_search();
    assert_eq!(app.view_videos.len(), 5);
    assert_eq!(app.view_videos[app.cursor_video], finale);
    let fo = app.folder_of(finale).unwrap();
    assert_eq!(app.view_folders[app.cursor_folder], fo);
    assert!(app.recenter.iter().all(|&r| r));
    invariants(&app);
}
#[test]
fn empty_search_shows_everything_and_matches_are_case_insensitive() {
    let mut app = app_with("s3", library());
    app.search = "LAUREN".into();
    app.apply_search();
    assert_eq!(app.view_videos.len(), 3, "folder names are searched");
    app.search = "03 cake".into();
    app.apply_search();
    assert_eq!(app.view_videos.len(), 1, "file names are searched");
    app.search = "  ".into();
    app.apply_search();
    assert_eq!(app.view_videos.len(), 12);
    assert_eq!(app.view_folders.len(), 6);
    assert_eq!(app.view_collections.len(), 3);
}
#[test]
fn sort_keys_order_as_described() {
    let mut app = app_with("sort", library());
    app.sort_row = 1;
    app.apply_sort();
    assert_eq!(app.sort, SortKey::Title);
    let titles: Vec<&str> = app.view_videos.iter().map(|&i| app.videos[i].title.as_str()).collect();
    let mut sorted = titles.clone();
    sorted.sort_by_key(|t| t.to_lowercase());
    assert_eq!(titles, sorted);
    app.sort_row = 2;
    app.apply_sort();
    let mtimes: Vec<u64> = app.view_videos.iter().map(|&i| app.videos[i].mtime).collect();
    assert!(mtimes.windows(2).all(|w| w[0] >= w[1]), "{mtimes:?}");
    app.sort_row = 3;
    app.apply_sort();
    let lens: Vec<f64> = app.view_videos.iter().map(|&i| app.videos[i].duration_secs).collect();
    assert!(lens.windows(2).all(|w| w[0] >= w[1]), "{lens:?}");
    app.sort_row = 0;
    app.apply_sort();
    assert_eq!(app.view_videos, (0..12).collect::<Vec<_>>());
}
#[test]
fn every_sort_key_is_a_permutation() {
    let mut app = app_with("perm", library());
    for (row, key) in SortKey::ALL.iter().enumerate() {
        app.sort_row = row;
        app.apply_sort();
        assert_eq!(app.sort, *key);
        let mut v = app.view_videos.clone();
        v.sort();
        assert_eq!(v, (0..app.videos.len()).collect::<Vec<_>>());
        invariants(&app);
    }
}
#[test]
fn scroll_offset_keeps_cursor_visible_with_minimal_movement() {
    assert_eq!(App::scroll_offset(0, 3, 100, 10, false), 0);
    assert_eq!(App::scroll_offset(0, 10, 100, 10, false), 1);
    assert_eq!(App::scroll_offset(50, 49, 100, 10, false), 49);
    assert_eq!(App::scroll_offset(50, 70, 100, 10, false), 61);
    assert_eq!(App::scroll_offset(95, 99, 100, 10, false), 95);
    assert_eq!(App::scroll_offset(0, 0, 0, 10, false), 0);
    assert_eq!(App::scroll_offset(7, 7, 8, 10, false), 7);
}
#[test]
fn scroll_offset_centres_off_screen_video_changes() {
    assert_eq!(App::scroll_offset(0, 50, 100, 10, true), 45);
    assert_eq!(App::scroll_offset(0, 5, 100, 10, true), 0);
    assert_eq!(App::scroll_offset(0, 99, 100, 10, true), 90);
    assert_eq!(App::scroll_offset(0, 7, 8, 10, true), 0);
}
#[test]
fn video_changes_request_recentre_but_cursor_moves_do_not() {
    let mut app = app_with("recenter", library());
    app.play_index(9);
    assert_eq!(app.recenter, [true; 3]);
    app.move_cursor(1);
    assert!(!app.recenter[0]);
    assert!(app.recenter[1] && app.recenter[2]);
    app.section = Section::Folders;
    app.move_cursor(-1);
    assert!(!app.recenter[1]);
}
#[test]
fn step_video_wraps_both_ways() {
    let mut app = app_with("step", library());
    app.play_index(11);
    app.step_video(1);
    assert_eq!(app.playing, Some(0));
    app.step_video(-1);
    assert_eq!(app.playing, Some(11));
}
#[test]
fn step_video_without_videos_is_a_noop() {
    let mut app = app_with("empty", Vec::new());
    app.step_video(1);
    assert_eq!(app.playing, None);
    app.activate_cursor();
    assert_eq!(app.playing, None);
    invariants(&app);
}
#[test]
fn shuffle_never_repeats_current_and_previous_walks_history() {
    let mut app = app_with("shuffle", library());
    app.toggle_shuffle();
    assert!(app.shuffle);
    app.play_index(0);
    let mut seen = Vec::new();
    for _ in 0..40 {
        let before = app.playing;
        app.step_video(1);
        assert_ne!(app.playing, before, "shuffle repeated the current video");
        seen.push(app.playing.unwrap());
    }
    let distinct: std::collections::HashSet<_> = seen.iter().collect();
    assert!(distinct.len() > 5, "shuffle hardly varied: {seen:?}");
    let last = *seen.last().unwrap();
    let prev = seen[seen.len() - 2];
    assert_eq!(app.playing, Some(last));
    app.step_video(-1);
    assert_eq!(app.playing, Some(prev));
    app.step_video(-1);
    assert_eq!(app.playing, Some(seen[seen.len() - 3]));
    for _ in 0..50 {
        app.step_video(-1);
    }
    invariants(&app);
}
#[test]
fn shuffle_and_expand_are_remembered_across_restarts() {
    let mut app = app_with("persist", library());
    assert!(!app.shuffle && !app.expanded);
    app.toggle_shuffle();
    app.toggle_expand();
    let saved = load_settings(&app.config);
    assert!(saved.shuffle && saved.expanded);
    app.toggle_shuffle();
    let saved = load_settings(&app.config);
    assert!(!saved.shuffle && saved.expanded);
}
#[test]
fn settings_file_without_extras_still_loads_fx() {
    let config = temp_config("legacy");
    std::fs::write(
        settings_file(&config),
        r#"{"preamp": 3, "loudness": true}"#,
    )
    .unwrap();
    let s = load_settings(&config);
    assert_eq!(s.fx.preamp, 3);
    assert!(s.fx.loudness);
    assert!(!s.shuffle && !s.expanded);
    assert_eq!(s.speed_pct, 0);
    assert_eq!(s.last_video, None);
}
#[test]
fn last_video_is_remembered_and_restored_paused() {
    let config = temp_config("resume");
    let mut app = App::build(config, FxParams::default(), None, None, renderer());
    let mut lib = library();
    lib.sort_by_cached_key(|t| t.sort_key());
    app.set_videos(lib);
    app.play_index(5);
    let path = app.videos[5].path.clone();
    let saved = load_settings(&app.config);
    assert_eq!(saved.last_video.as_ref(), Some(&path));
    // A new app on the same settings: the video is selected and cued paused
    // once the index arrives.
    let config = temp_config("resume");
    let mut app2 = App::build(config, FxParams::default(), None, None, renderer());
    app2.resume_video = load_settings(&app2.config).last_video;
    assert_eq!(app2.playing, None);
    let mut lib = library();
    lib.sort_by_cached_key(|t| t.sort_key());
    app2.set_videos(lib);
    assert_eq!(app2.playing.map(|i| app2.videos[i].path.clone()), Some(path.clone()));
    assert!(app2.start_paused);
    assert_eq!(app2.pending_play, Some(5));
    assert_eq!(app2.view_videos[app2.cursor_video], 5);
    // Saving before the index has loaded keeps the remembered video.
    let config = temp_config("resume");
    let mut app3 = App::build(config, FxParams::default(), None, None, renderer());
    app3.resume_video = Some(path.clone());
    app3.toggle_shuffle();
    assert_eq!(load_settings(&app3.config).last_video, Some(path));
}
#[test]
fn speed_is_remembered_and_clamped_on_restore() {
    let config = temp_config("speed");
    let mut app = App::build(config, FxParams::default(), None, None, renderer());
    app.adjust_speed(7);
    assert_eq!(load_settings(&app.config).speed_pct, 7);
    std::fs::write(settings_file(&app.config), r#"{"speed_pct": 400}"#).unwrap();
    let s = load_settings(&app.config);
    let mut app2 = App::build(temp_config("speed"), FxParams::default(), None, None, renderer());
    app2.set_speed_pct(s.speed_pct);
    assert_eq!(app2.speed_pct, 100);
}
#[test]
fn missing_last_video_is_ignored() {
    let config = temp_config("missing");
    let mut app = App::build(config, FxParams::default(), None, None, renderer());
    app.resume_video = Some(PathBuf::from("/m/gone/00 Nope.mp4"));
    app.set_videos(library());
    assert_eq!(app.playing, None);
    assert_eq!(app.resume_video, None);
    invariants(&app);
}
#[test]
fn shuffle_with_one_visible_video_falls_back_to_linear() {
    let mut app = app_with("one", vec![video("A", "", "Solo", 1)]);
    app.toggle_shuffle();
    app.play_index(0);
    app.step_video(1);
    assert_eq!(app.playing, Some(0));
}
#[test]
fn speed_adjusts_clamps_and_resets() {
    let mut app = app_with("speed2", library());
    app.adjust_speed(10);
    assert_eq!(app.speed_pct, 10);
    assert!((app.speed() - 1.1).abs() < 1e-6);
    app.adjust_speed(-100);
    assert_eq!(app.speed_pct, -50);
    app.adjust_speed(500);
    assert_eq!(app.speed_pct, 100);
    app.reset_speed();
    assert_eq!(app.speed_pct, 0);
}
#[test]
fn eq_rows_follow_mode_and_stereo() {
    let mut app = app_with("eq", library());
    assert_eq!(app.eq_rows().len(), 5);
    app.eq_row = 2;
    app.eq_adjust(1);
    assert_eq!(app.fx.eq.mode, EqMode::Three);
    assert_eq!(app.eq_rows().len(), 8);
    app.eq_row = 3;
    app.eq_adjust(3);
    assert_eq!(app.fx.eq.three[0], 3);
    app.eq_adjust(100);
    assert_eq!(app.fx.eq.three[0], EQ_RANGE_DB);
    app.eq_reset_row();
    assert_eq!(app.fx.eq.three[0], 0);
    let stereo = app.eq_rows().len() - 1;
    app.eq_row = stereo;
    app.eq_adjust(1);
    app.eq_adjust(1);
    assert_eq!(app.fx.stereo, StereoMode::Width);
    assert_eq!(app.eq_rows().last(), Some(&EqRow::Width));
    app.eq_row = app.eq_rows().len() - 1;
    app.eq_adjust(-100);
    assert_eq!(app.fx.width, 0);
}
#[test]
fn unplayable_entries_are_refused_with_a_message() {
    let mut lib = library();
    lib[0].width = 0;
    lib[0].height = 0;
    lib[0].acodec = None;
    let mut app = app_with("bare", lib);
    let idx = app.videos.iter().position(|v| !v.has_video()).unwrap();
    app.play_index(idx);
    app.flush_play();
    assert_eq!(app.playing, None);
    assert!(app.status.contains("no audio or video stream"), "{}", app.status);
}
#[test]
fn renderer_cycles_through_every_option_and_is_remembered() {
    let mut app = app_with("renderer", library());
    let names: Vec<String> = app.renderer_options().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        [
            "auto",
            "kitty",
            "iterm2",
            "blocks",
            "blocks256",
            "quads",
            "quads256",
            "sextants",
            "sextants256",
            "mosaic",
            "mosaic256",
            "ascii",
            "ascii256"
        ],
        "no sixel without a known cell size"
    );
    app.cycle_renderer();
    assert_eq!(app.renderer.protocol, Protocol::Kitty);
    assert!(app.renderer_was.is_some(), "the old picture needs clearing");
    for _ in 0..10 {
        app.cycle_renderer();
    }
    assert_eq!((app.renderer.protocol, app.renderer.truecolor), (Protocol::Ascii, true));
    assert_eq!(load_settings(&app.config).renderer.as_deref(), Some("ascii"));
    app.cycle_renderer();
    app.cycle_renderer();
    assert_eq!(app.renderer, renderer(), "back to the detected one");
    assert_eq!(load_settings(&app.config).renderer, None);
    // A remembered choice is applied on the next start.
    app.choose_renderer("blocks256");
    app.save_settings();
    let mut again = App::build(temp_config("renderer"), FxParams::default(), None, None, renderer());
    if let Some(name) = load_settings(&again.config).renderer.as_deref() {
        again.choose_renderer(name);
    }
    assert!(!again.renderer.truecolor);
}
#[test]
fn move_cursor_clamps_at_ends() {
    let mut app = app_with("clamp", library());
    app.move_cursor(-5);
    assert_eq!(app.cursor_video, 0);
    app.move_cursor(i64::MAX / 2);
    assert_eq!(app.cursor_video, app.view_videos.len() - 1);
    app.section = Section::Folders;
    app.move_cursor(i64::MIN / 2);
    assert_eq!(app.cursor_folder, 0);
    invariants(&app);
}
#[test]
#[ignore]
fn soak_random_operations_keep_invariants() {
    let mut lib = Vec::new();
    for co in 0..40 {
        for fo in 0..6 {
            for n in 1..=12 {
                lib.push(video(
                    &format!("Collection {co:03}"),
                    &format!("Folder {fo}"),
                    &format!("Clip {:03}", (co * 7 + fo * 3 + n) % 97),
                    n,
                ));
            }
        }
    }
    let n_videos = lib.len();
    let mut app = app_with("soak", lib);
    let mut rng = 0x1234_5678_9abc_def1u64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let words = [
        "collection 00",
        "folder 3",
        "clip 01",
        "zzz",
        "",
        "COLLECTION 039",
        "7",
    ];
    let started = TInstant::now();
    let iters = 200_000;
    for _ in 0..iters {
        match next() % 14 {
            0 => app.move_cursor((next() % 41) as i64 - 20),
            1 => app.step_video(1),
            2 => app.step_video(-1),
            3 => app.toggle_shuffle(),
            4 => {
                app.search = words[(next() % words.len() as u64) as usize].into();
                app.apply_search();
            }
            5 => {
                app.sort_row = (next() % SortKey::ALL.len() as u64) as usize;
                app.apply_sort();
            }
            6 => app.play_index((next() % n_videos as u64) as usize),
            7 => app.adjust_speed((next() % 21) as i32 - 10),
            8 => app.reset_speed(),
            9 => {
                app.section = match next() % 3 {
                    0 => Section::Videos,
                    1 => Section::Folders,
                    _ => Section::Collections,
                }
            }
            10 => app.activate_cursor(),
            11 => app.toggle_play(),
            12 => {
                app.eq_row = (next() % app.eq_rows().len() as u64) as usize;
                app.eq_adjust((next() % 11) as i32 - 5);
            }
            _ => app.flush_play(),
        }
        invariants(&app);
    }
    let el = started.elapsed();
    eprintln!(
        "soak: {iters} ops over {n_videos} videos in {:.2}s ({:.0} ops/s)",
        el.as_secs_f64(),
        iters as f64 / el.as_secs_f64()
    );
}
#[test]
#[ignore]
fn perf_search_and_sort_on_large_library() {
    let mut lib = Vec::new();
    for i in 0..20_000u32 {
        lib.push(video(
            &format!("Collection {:04}", i % 1500),
            &format!("Folder {}", i % 4000),
            &format!("Title {}", (i * 2654435761u32.wrapping_mul(1)) % 100_000),
            i % 15 + 1,
        ));
    }
    let mut app = app_with("perf", lib);
    let t = TInstant::now();
    for q in ["collection 07", "title 1", "folder 39", ""] {
        app.search = q.into();
        app.apply_search();
    }
    let search_ms = t.elapsed().as_secs_f64() * 1000.0 / 4.0;
    let t = TInstant::now();
    for row in 0..SortKey::ALL.len() {
        app.sort_row = row;
        app.apply_sort();
    }
    let sort_ms = t.elapsed().as_secs_f64() * 1000.0 / SortKey::ALL.len() as f64;
    eprintln!("perf: 20k videos, search {search_ms:.1} ms, sort+filter {sort_ms:.1} ms");
    assert!(search_ms < 200.0, "search {search_ms} ms");
    assert!(sort_ms < 500.0, "sort {sort_ms} ms");
}
