# daffy

A terminal video player. Point it at a folder of videos and it indexes them (metadata via ffprobe, playback via ffmpeg) and lets you browse by video, folder, or collection. The picture is drawn in the terminal as well as the terminal allows: real images in Kitty, iTerm2, WezTerm, Ghostty, VS Code and sixel-capable terminals, half-block cells everywhere else. Audio goes through the same equaliser and stereo chain as the music player daffy was forked from.

## Setup

    brew install ffmpeg        # or: apt install ffmpeg
    cargo install --path .
    daffy

`cargo run` builds without optimisation; it still plays, but `cargo install` or `cargo run --release` is what to judge smoothness by.

Runs on macOS, Linux and Windows. `ffmpeg` and `ffprobe` must be installed; daffy looks on the PATH and then in the usual Homebrew and `/usr/local` locations, or where `ffmpeg.dir` in the config points. Audio output goes through the platform's native API (CoreAudio, ALSA, WASAPI). On Linux the build needs the ALSA headers and pkg-config, e.g. `sudo apt install libasound2-dev pkg-config`. On macOS, if your library lives somewhere protected (`~/Movies`, `~/Desktop`), give your terminal app access under System Settings → Privacy & Security → Files and Folders.

On first run `~/.daffy` is created with:

    video.home = ~/Movies

Set `video.home` to the folder holding your videos; every video file at or under it is indexed. The index is cached in `~/.daffy-index.json` (override with `index.file = ...`) and refreshed on each start, probing only files whose size or modification time changed. `DAFFY_CONFIG=/path/to/file` points at a different config file. `DAFFY_AUDIO_BUFFER=4096` asks the output device for a fixed period of that many frames instead of leaving the choice to the audio backend. `DAFFY_KEYLOG=1` writes every key event to `~/.daffy.log`, which is the thing to look at if daffy seems to press keys by itself: terminal replies that arrive as keystrokes are filtered out, and the log shows what came in.

Supported containers: mp4, m4v, mov, mkv, webm, avi, mpg/mpeg, ts, m2ts, mts, wmv, flv, 3gp, ogv, vob, divx — anything ffmpeg can decode. A file without a picture (an audio-only mp4, say) plays as audio only.

### The picture

    video.renderer = auto

How frames are drawn is detected on start by asking the terminal (its cell size in pixels, whether it answers a Kitty graphics query, and whether it lists sixel among its device attributes) and looking at `TERM_PROGRAM` and friends. In order of preference:

| Renderer | Where | How |
| --- | --- | --- |
| `kitty` | kitty, Ghostty, Konsole, WezTerm with Kitty graphics enabled | Kitty graphics protocol, zlib-compressed RGB, up to 1280 px wide at 24 fps |
| `iterm2` | iTerm2, VS Code, WezTerm, mintty, Tabby, Rio | iTerm2 inline images (JPEG), up to 960 px wide at 24 fps |
| `sixel` | xterm (with sixel), foot, mlterm, Windows Terminal | sixel, 252-colour ordered-dithered palette, up to 800 px wide at 15 fps |
| `blocks` | everything else, and inside tmux or screen | one `▀` per cell, two pixels high, 24-bit colour (or `blocks256` for 256-colour terminals such as Terminal.app) |
| `ascii` | by choice | one character per cell, picked by brightness from a 70-step ramp, in the pixel's colour (`ascii256` for 256 colours) |

The header shows which one is in use. Press `r` to cycle through all of them while something plays (the detected one, then kitty, iterm2, sixel, blocks, blocks256, ascii, ascii256); the choice is remembered, and cycling back to the detected one forgets it. `daffy --detect` prints what the terminal answered and what was chosen, which is the place to start if the picture is not what you expected. Set `video.renderer` to force one from the config; `sixel` needs a terminal that reports its cell size. Inside tmux the graphics protocols need passthrough configured, so daffy uses blocks there.

Frames are decoded at the size they will be shown, encoded on a separate thread, and a frame is skipped whenever the terminal is still taking the previous one, so a slow terminal lowers the frame rate rather than stalling playback. Press `x` to give the picture the whole window; the decoder is restarted at the new size (and after a resize) once the size has settled.

### Video editor

    video.editor = /Applications/HandBrake.app
    video.editor.mode = gui

`video.editor` is the command that `E` runs on the selected video. Write `{file}` where the path should go (it may appear more than once, and inside a longer argument such as `--in={file}`); if the command has no `{file}`, the path is appended as the last argument. Quote arguments that contain spaces, e.g. `"Final Cut Pro.app"`. On macOS a `.app` bundle is launched through `open -a`. `video.editor.mode` says how the editor behaves:

- `gui` (default): the editor is a desktop app. daffy starts it and carries on playing.
- `cli`: the editor runs in this terminal. daffy pauses playback, hands the screen over, waits for the editor to exit, then redraws, resumes playback and refreshes the index so changed lengths or titles show up. Your selection and scroll position are kept.

## Keys

| Key | Action |
| --- | --- |
| `v` / `d` / `c` | Show the video / folder / collection list. A folder is the directory a file is in (relative to `video.home`); a collection is the top-level directory under `video.home` |
| `/` | Search: type text matched against title, folder, collection and file name, then `Enter`. An empty search shows everything again; `Esc` cancels |
| `o` | Sort the video list by folder (collection, folder, file name), title, newest, or length (`f`/`t`/`n`/`l` are shortcuts inside the popup). Next/previous follow the chosen order |
| `↑` / `↓` (or `k` / `j`) | Move the selection; `PgUp` / `PgDn` move a page, `Home` / `End` jump to the top / bottom |
| `Enter` | Play the selected video, or the first video of the selected folder / collection |
| `space` | Play / pause (plays the selection if nothing is loaded) |
| `n` / `p` | Next / previous video |
| `z` | Toggle shuffle: next and auto-advance pick a random video from the current list, previous walks back through what was played; remembered across restarts |
| `←` / `→` (or `,` / `.`) | Rewind / fast-forward 10 seconds |
| `[` / `]` | Rewind / fast-forward 1 minute |
| `x` | Expand: the picture fills the window, with only the progress bar under it; `x` again brings the list back. Remembered across restarts |
| `r` | Cycle the renderer: detected, kitty, iterm2, sixel, blocks, blocks256, ascii, ascii256 (see The picture above). Remembered across restarts |
| `=` (or `+`) / `-` | Playback speed +1% / −1% (range 50% to 200%) |
| `0` | Reset speed to 100% |
| `e` | Equaliser & stereo dialog: preamp, auto-normalise, off, 3-band or 7-band EQ, loudness, mono / stereo width |
| `l` | Toggle loudness (bass and treble lift for quiet listening) |
| `f` | Reveal the current file in Finder / Explorer / your file manager |
| `E` | Open the selected video in your editor (see `video.editor` above; the key is only shown once one is configured) |
| `i` | Refresh the index (new, changed and removed files); `I` rebuilds it from scratch, probing every file again |
| `q` | Quit |

Speed changes resample the audio, so pitch shifts with speed; the picture follows the audio clock. Seeking restarts the decoders at the new position, so it takes a moment on large files.

`daffy --version` prints the version and exits. If a file will not play, `daffy --probe <file>` prints what ffprobe reports, decodes the first two seconds of picture the way playback does, and measures the audio level.

## How playback works

Two ffmpeg processes per video: one decodes the audio to 48 kHz stereo floats, which stream through the FX chain into the audio device; the other decodes the picture, scaled by ffmpeg to the exact size the screen needs, at the file's frame rate capped per renderer. The audio position is the clock: the frame whose time has come is shown, older ones are dropped. Files without audio run on a wall clock scaled by the playback speed. Playback starts once a quarter of a second of audio is buffered, so ffmpeg's start-up does not play as silence and put the clock ahead of the sound. Anything the decoders or the audio stack write to stderr lands in `~/.daffy.log`.

## Equaliser and loudness

Press `e` for the equaliser. The first row is the preamp: a gain from −24 dB to +12 dB applied before everything else. `←`/`→` move it 1 dB, `PgUp`/`PgDn` 3 dB, `0` returns it to 0 dB. A boost is soft-limited so it cannot clip.

The Normalise row below it levels videos automatically. When it is on, the first time a video plays daffy decodes its audio in the background and measures its loudness (RMS over 400 ms blocks, ignoring near-silent blocks), then adds whatever gain brings it to −18 dBFS, between −18 dB and +12 dB, to the preamp. The measurement takes a while on a long film, fades in when it arrives, and is stored in the index so later plays start at the right level; a file that changes is measured again. The Preamp row shows the video's gain next to your own setting, and `daffy --probe <file>` prints it too.

The 3-band mode has a low shelf at 200 Hz, a mid peak at 1 kHz and a high shelf at 4 kHz; the 7-band mode is a graphic EQ at 60, 150, 400, 1k, 2.5k, 6k and 15k Hz. Each band ranges ±12 dB (`←`/`→` for 1 dB, `PgUp`/`PgDn` for 3 dB, `0` to flatten). The last row toggles loudness, which `l` also toggles from the main screen. A soft limiter after the EQ keeps boosts from clipping.

The Stereo row offers Normal, Mono, and Width. Width uses mid/side processing: the side signal (L−R) is scaled from 0% to 200%, where 100% leaves the image untouched and higher values push it wider (`←`/`→` for 5%, `PgUp`/`PgDn` for 15%). These settings, shuffle, speed, expand and the last video are saved in `~/.daffy-settings.json`.

## Testing

`cargo test` runs the unit tests: library grouping, search and sort behaviour, shuffle, speed, settings, the EQ/stereo chain, ffprobe output parsing, frame sizing, the PCM queue that feeds the audio device, the terminal capability parser and renderer choice, and the Kitty, iTerm2 (JPEG), sixel and 256-colour encoders (the sixel output is decoded back pixel by pixel). None of them touch an audio device or need ffmpeg.

Longer soak and performance tests are marked `#[ignore]` and are meant for a release build:

    cargo test --release -- --ignored --nocapture

They push 200k random operations through the app state against a 3k-video library, stream an hour of audio through the FX chain with parameters randomised twice a second, and time search and sort over 20k videos.

## Versions

### v1.0.0

- initial version
