//! P4-5 step 1: the command registry. One row per command of the app: name, one-line description,
//! arguments, result schema, what it changes, where it is available and whether it is a long job.
//!
//! The rows are the single description the three front ends share: the Tauri wrappers
//! (`apps/desktop/src-tauri`), the command-line tool and the MCP server (P4-5 stage 2). The
//! tests of `tests/api.rs` fail when a `#[tauri::command]` has no row, when a row has no
//! command, when the argument names of a wrapper differ from the row, and (here) when the
//! test bridge does not run a command of the agent surface. The names and arguments are those
//! of the Tauri wrappers (`snake_case` on the wire); the schemas are JSON Schema, draft 2020-12
//! subset (`type`, `properties`, `required`, `items`, `description`).
//!
//! The registry has no behaviour of its own: calling a command stays with the dispatchers of the
//! modules (`bridge.rs` `invoke`); stage 2 moves them into one `call` function that the bridge,
//! the command-line tool and the MCP server share.

use serde_json::{Value, json};

/// What a command changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Nothing that is kept.
    Read,
    /// The open project's document in memory (dirty until `project_save`).
    Doc,
    /// Files: the project folder (document, `Exports/`, `Analysis/`, peak cache), or other files
    /// the caller names. May change the document too.
    Files,
    /// The app, not a project: settings, kit presets, the playback state.
    App,
}

/// Where a command is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The command-line tool and the MCP server.
    Agent,
    /// Only the running window (menu, splash, transport, output devices, quit).
    Window,
    /// Debug builds only (the self-test).
    Selftest,
}

/// How a command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    /// Answers at once.
    Quick,
    /// Answers at once and keeps working: poll the named status command.
    Poll(&'static str),
    /// Takes a `progress` channel and answers when the work is done (minutes for a big project).
    Progress,
}

/// The JSON type of an argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    Str,
    Int,
    Num,
    Bool,
    /// A list of strings.
    Strs,
    /// An object.
    Obj,
    /// A list of objects.
    Objs,
    /// Any JSON value.
    Any,
    /// A tempo and meter map (P4-8) or null for the default map.
    TempoMap,
}

/// JSON Schema of a tempo and meter map (`TempoMap::to_json`; the ranges are those of
/// `TempoMap::validate`), as text for the result rows.
macro_rules! tempo_map_schema {
    () => {
        r#"{"type":"object","additionalProperties":false,"properties":{
"bpm":{"type":"number","minimum":4,"maximum":1000,"description":"start tempo, default 120"},
"steps":{"type":"array","maxItems":100000,"description":"tempo steps, t strictly increasing","items":{"type":"object","required":["t","bpm"],"additionalProperties":false,"properties":{"t":{"type":"number","exclusiveMinimum":0,"description":"seconds of the audio"},"bpm":{"type":"number","minimum":4,"maximum":1000}}}},
"meter":{"type":"array","minItems":2,"maxItems":2,"prefixItems":[{"type":"integer","minimum":1,"maximum":64,"description":"beats per bar"},{"type":"integer","enum":[1,2,4,8,16,32,64],"description":"note value of a beat"}],"description":"start meter [beats, note value], default [4, 4]"},
"changes":{"type":"array","maxItems":100000,"description":"meter changes, bar strictly increasing","items":{"type":"object","required":["bar","meter"],"additionalProperties":false,"properties":{"bar":{"type":"integer","minimum":2},"meter":{"type":"array","minItems":2,"maxItems":2,"prefixItems":[{"type":"integer","minimum":1,"maximum":64},{"type":"integer","enum":[1,2,4,8,16,32,64]}]}}}},
"bar1_s":{"type":"number","minimum":0,"maximum":1000000,"description":"seconds of bar 1 beat 1, default 0"}}}"#
    };
}

impl Ty {
    fn schema(self) -> Value {
        match self {
            Ty::Str => json!({"type": "string"}),
            Ty::Int => json!({"type": "integer"}),
            Ty::Num => json!({"type": "number"}),
            Ty::Bool => json!({"type": "boolean"}),
            Ty::Strs => json!({"type": "array", "items": {"type": "string"}}),
            Ty::Obj => json!({"type": "object"}),
            Ty::Objs => json!({"type": "array", "items": {"type": "object"}}),
            Ty::Any => json!({}),
            Ty::TempoMap => {
                let mut m: Value = serde_json::from_str(tempo_map_schema!()).expect("the tempo map schema is JSON");
                m["type"] = json!(["object", "null"]);
                m
            }
        }
    }
}

/// One argument.
#[derive(Debug, Clone, Copy)]
pub struct Arg {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
    pub about: &'static str,
}

/// One command.
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub name: &'static str,
    /// One line for `drumspike help` and the MCP tool list.
    pub about: &'static str,
    pub args: &'static [Arg],
    /// The JSON Schema of the result, as text (parsed by [`CommandSpec::result_schema`]).
    pub result: &'static str,
    pub effect: Effect,
    pub surface: Surface,
    pub run: Run,
    /// The answer is bytes (`peaks_tile`, `samples_window`), not JSON.
    pub binary: bool,
}

impl CommandSpec {
    /// True when the command changes the project (its document or its folder).
    #[must_use]
    pub fn writes_project(&self) -> bool {
        matches!(self.effect, Effect::Doc | Effect::Files)
    }

    /// True when the call takes a `progress` channel: the Tauri wrapper has the parameter, the
    /// CLI prints progress lines and the MCP server runs the call as a background job.
    #[must_use]
    pub fn has_progress(&self) -> bool {
        matches!(self.run, Run::Progress) || matches!(self.run, Run::Poll(_)) && self.takes_progress_channel()
    }

    fn takes_progress_channel(&self) -> bool {
        // the commands that start a job and also send its changes on a channel
        matches!(self.name, "analyze_start" | "peaks_build_start")
    }

    /// The JSON Schema of the arguments (an object with the argument names as keys).
    #[must_use]
    pub fn args_schema(&self) -> Value {
        let mut props = serde_json::Map::new();
        for a in self.args {
            let mut s = a.ty.schema();
            s["description"] = json!(a.about);
            props.insert(a.name.into(), s);
        }
        let req: Vec<&str> = self.args.iter().filter(|a| a.required).map(|a| a.name).collect();
        json!({"type": "object", "properties": props, "required": req, "additionalProperties": false})
    }

    /// The JSON Schema of the result.
    ///
    /// # Panics
    ///
    /// Panics (`expect`) when the row's `result` text is not valid JSON; a registry test pins that every row's is.
    #[must_use]
    pub fn result_schema(&self) -> Value {
        serde_json::from_str(self.result).expect("a registry result schema is valid JSON")
    }
}

const fn req(name: &'static str, ty: Ty, about: &'static str) -> Arg {
    Arg {
        name,
        ty,
        required: true,
        about,
    }
}
const fn opt(name: &'static str, ty: Ty, about: &'static str) -> Arg {
    Arg {
        name,
        ty,
        required: false,
        about,
    }
}

const SID: Arg = req(
    "sid",
    Ty::Str,
    "Project id from project_open, project_new or project_import",
);
const NAME: Arg = req("name", Ty::Str, "Track name");
const DEST_OPT: Arg = opt(
    "dest",
    Ty::Str,
    "Destination folder: absolute, existing, writable; default Exports/ in the project folder",
);
const COMPENSATE_OPT: Arg = opt(
    "compensate",
    Ty::Bool,
    "P4-7c: put the MIDI notes of a track with a sampler calibration at the aligned attack minus the sampler offset; default true",
);
const EVENTS_OPT: Arg = opt(
    "events",
    Ty::Objs,
    "Hits to use instead of the track's stored hits: [{t, v}]",
);

const OBJ: &str = r#"{"type":"object"}"#;
const SUMMARY: &str = r#"{"type":"object","required":["sid","name","folder","dirty","order","tracks"]}"#;

const fn cmd(
    name: &'static str,
    about: &'static str,
    args: &'static [Arg],
    result: &'static str,
    effect: Effect,
    surface: Surface,
    run: Run,
) -> CommandSpec {
    CommandSpec {
        name,
        about,
        args,
        result,
        effect,
        surface,
        run,
        binary: false,
    }
}

use Effect::{App, Doc, Files, Read};
use Run::{Poll, Progress, Quick};
use Surface::{Agent, Selftest, Window};

/// Every command of the app, in the order of the Tauri handler lists.
pub static REGISTRY: &[CommandSpec] = &[
    // ---- the app
    cmd(
        "app_info",
        "Core version, detector and chain (main is model-free)",
        &[],
        r#"{"type":"object","required":["core_version","detector","chain"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd("about_info", "Data of the About dialog", &[], OBJ, Read, Window, Quick),
    cmd(
        "feedback_send",
        "Post the previewed feedback report to the CBV feedback gateway (#164)",
        &[
            req(
                "payload",
                Ty::Obj,
                "The exact JSON the form previewed: kind, title, description, steps?, meta",
            ),
            opt("url", Ty::Str, "Gateway URL; the default is the CBV feedback gateway"),
        ],
        OBJ,
        App,
        Window,
        Quick,
    ),
    cmd(
        "about_third_party",
        "Text of the third-party licence list",
        &[],
        r#"{"type":"string"}"#,
        Read,
        Window,
        Quick,
    ),
    cmd(
        "open_link",
        "Open one of the fixed About addresses by id (website, source, cbv)",
        &[req("id", Ty::Str, "The fixed id of the address")],
        "{}",
        App,
        Window,
        Quick,
    ),
    cmd(
        "startup_mark",
        "Startup timing probe of the window",
        &[opt("info", Ty::Str, "Probe text")],
        "{}",
        Read,
        Window,
        Quick,
    ),
    // ---- the transport (needs the window and an audio device)
    cmd(
        "transport_play",
        "Start playback of a project",
        &[
            SID,
            opt("position", Ty::Num, "Start in seconds"),
            opt("events", Ty::Any, "Hits per track for the click"),
            opt("speed", Ty::Num, "1.0 or 0.5"),
            opt("start", Ty::Num, "Audition: window start in seconds"),
            opt(
                "end",
                Ty::Num,
                "Audition: window end in seconds (with shot, the one-shot's length)",
            ),
            opt("loop", Ty::Bool, "Audition: play the window again until stop"),
            opt(
                "tracks",
                Ty::Strs,
                "Audition: track names that are audible for this play only (overrides solo)",
            ),
            opt(
                "shot",
                Ty::Str,
                "P4-40e: track name whose layer one-shot plays once at the window start (the Layer view's Play); the track is soloed when tracks is empty",
            ),
        ],
        OBJ,
        App,
        Window,
        Progress,
    ),
    cmd(
        "transport_stop",
        "Stop playback",
        &[opt("close", Ty::Bool, "Close the output stream too")],
        OBJ,
        App,
        Window,
        Quick,
    ),
    cmd(
        "transport_seek",
        "Move the playhead",
        &[req("position", Ty::Num, "Seconds")],
        OBJ,
        App,
        Window,
        Quick,
    ),
    cmd(
        "transport_mix",
        "Set mute and solo",
        &[
            opt("mute", Ty::Any, "Track names to mute"),
            opt("solo", Ty::Any, "Track names to solo"),
        ],
        OBJ,
        App,
        Window,
        Quick,
    ),
    cmd(
        "transport_clicks",
        "Set the click: on or off, volume, pitches",
        &[
            opt("enabled", Ty::Bool, "Click on"),
            opt("volume", Ty::Num, "0 to 1"),
            opt("pitches", Ty::Any, "Click pitch per role"),
        ],
        OBJ,
        App,
        Window,
        Quick,
    ),
    cmd("transport_status", "Playback state", &[], OBJ, Read, Window, Quick),
    cmd(
        "track_set_role",
        "Set the role of a track (Inspector 0.9.1); a change re-analyzes it (a stored re-pick); `events` restores the hits (the undo)",
        &[
            SID,
            NAME,
            req("role", Ty::Str, "kick, snare, tom, hat, cymbal or other"),
            opt(
                "note",
                Ty::Int,
                "The MIDI note to store with it, 0 to 127; without it the note stays",
            ),
            opt(
                "source",
                Ty::Str,
                "Where the role comes from: user (default), guess, audio; the undo also takes scheme, none, preset",
            ),
            opt(
                "role_match",
                Ty::Str,
                "The matched words of the name (the undo restores them)",
            ),
            opt(
                "events",
                Ty::Any,
                "The hits to restore (the undo state of a previous call); nothing re-analyzes then",
            ),
        ],
        r#"{"type":"object","required":["name","role","note","role_source","reanalyzed","previous"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_set_mix",
        "Set a track's volume and pan (stored in the project)",
        &[
            SID,
            NAME,
            opt("volume_db", Ty::Num, "Volume in dB"),
            opt("pan", Ty::Num, "-1 left to 1 right"),
        ],
        r#"{"type":"object","required":["volume_db","pan","applied"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "transport_master",
        "Set the master volume in dB",
        &[req("volume_db", Ty::Num, "dB")],
        OBJ,
        App,
        Window,
        Quick,
    ),
    cmd(
        "outputs_list",
        "List the audio output devices",
        &[],
        OBJ,
        Read,
        Window,
        Quick,
    ),
    cmd(
        "output_select",
        "Choose the audio output device",
        &[opt("device", Ty::Str, "Device name; none for the default")],
        OBJ,
        App,
        Window,
        Quick,
    ),
    // ---- projects
    cmd(
        "project_open",
        "Open a project folder or .drumspike file",
        &[
            req("path", Ty::Str, "Project folder or document path"),
            opt(
                "read_only",
                Ty::Bool,
                "Open without the project lock; commands that change the project are refused",
            ),
        ],
        SUMMARY,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "project_get",
        "Summary of an open project: tracks, order, settings, dirty flag",
        &[SID],
        SUMMARY,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "project_list",
        "List the open projects",
        &[opt("shape", Ty::Str, "sessions (default) or ids")],
        r#"{"type":"array","description":"The open projects","items":{"type":"object"}}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "tracks_get",
        "The tracks of a project with role, note, params, hit counts and follows",
        &[SID],
        r#"{"type":"object","required":["sid","name","order","tracks","dirty"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "events_get",
        "The hits of a track: [{t seconds, v velocity, ...}]",
        &[SID, NAME],
        r#"{"type":"object","required":["sr","n_samples","duration","events"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "events_set",
        "Replace all hits of a track (use events_edit for single hits)",
        &[
            SID,
            NAME,
            req("events", Ty::Objs, "The new hits: [{t, v, added?, orig?}]"),
        ],
        r#"{"type":"object","required":["saved"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "peaks_info",
        "Waveform summary of a track: rate, length, levels",
        &[SID, NAME],
        r#"{"type":"object","required":["sr","n_samples","levels"]}"#,
        Read,
        Window,
        Quick,
    ),
    cmd(
        "peaks_tile",
        "Waveform peaks, bins from to of one level (bytes: u32 level, start, n, then f32 min, max, rms)",
        &[
            SID,
            NAME,
            req("level", Ty::Int, "Block size, a key of peaks_info levels"),
            req("from", Ty::Int, "First bin"),
            req("to", Ty::Int, "End bin"),
        ],
        "{}",
        Read,
        Window,
        Quick,
    ),
    cmd(
        "samples_window",
        "Audio samples of a track (bytes: u32 start, u32 rate, then f32)",
        &[
            SID,
            NAME,
            req("start", Ty::Int, "First sample"),
            req("end", Ty::Int, "End sample, at most 262144 after start"),
        ],
        "{}",
        Read,
        Window,
        Quick,
    ),
    // ---- analysis and export
    cmd(
        "analyze_start",
        "Detect the hits of some or all tracks; poll analyze_status",
        &[
            SID,
            opt(
                "tracks",
                Ty::Strs,
                "Track names or patterns (exact, glob `t*`, range `t1-t4`); default all; a pattern that matches nothing is an error",
            ),
            opt(
                "params",
                Ty::Obj,
                "Params per track name: {track: {sensitivity | thresholds, min_gap_ms, refine, shift_samples, vel_lo_db, vel_hi_db}}",
            ),
            opt(
                "detector",
                Ty::Str,
                "The detector of this analysis (#388): `builtin` or a missing flag selects the built-in DSP detector, otherwise a plugin id (per-track params.detector wins)",
            ),
            opt(
                "keep_edited",
                Ty::Bool,
                "Keep the hits added or moved by hand; they win within 20 ms (default true)",
            ),
        ],
        r#"{"type":"object","required":["started"]}"#,
        Files,
        Agent,
        Poll("analyze_status"),
    ),
    cmd(
        "analyze_status",
        "State of the analysis job: idle, queued, running, done",
        &[SID],
        r#"{"type":"object","required":["state","tracks"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "export_midi",
        "Write Exports/<name>_spike.mid (note and hits default to the track's)",
        &[
            SID,
            NAME,
            opt("note", Ty::Int, "MIDI note"),
            EVENTS_OPT,
            opt("ppq", Ty::Int, "Ticks per quarter, default 24000"),
            opt("tempo", Ty::Num, "BPM, default 120"),
            DEST_OPT,
            COMPENSATE_OPT,
        ],
        r#"{"type":"object","required":["path","size","n_events"]}"#,
        Files,
        Agent,
        Quick,
    ),
    cmd(
        "export_spikes",
        "Write Exports/<name>_spike.wav",
        &[SID, NAME, EVENTS_OPT, DEST_OPT],
        r#"{"type":"object","required":["path","n_events","sr"]}"#,
        Files,
        Agent,
        Quick,
    ),
    cmd(
        "export_all",
        "Write MIDI and spike files into Exports/ or dest; midi_mode single: one multi-track .mid",
        &[
            SID,
            DEST_OPT,
            opt(
                "midi_mode",
                Ty::Str,
                "per_track (default: <track>_spike.mid per track) or single (one type 1 file <project>_drums.mid, one MIDI track per drum track, the project's tempo map)",
            ),
            opt(
                "tracks",
                Ty::Strs,
                "Track names or patterns (exact, glob `t*`, range `t1-t4`); default all; a pattern that matches nothing is an error",
            ),
            opt(
                "roles",
                Ty::Strs,
                "Export by role instead of (or on top of) tracks: the tracks whose role is one of these (case-insensitive); a role that no track has is an error",
            ),
            opt(
                "formats",
                Ty::Strs,
                "midi, spikes and/or audio (audio: <track>_audio_spike.wav, the track's own audio cut to the body of each hit (about 1.5 cycles of the low tone), #378); default midi and spikes",
            ),
            COMPENSATE_OPT,
        ],
        r#"{"type":"object","required":["files","midi","spikes","errors"]}"#,
        Files,
        Agent,
        Progress,
    ),
    cmd(
        "velocities_recompute",
        "Hits with velocities from the track's audio and its floor and ceiling",
        &[SID, NAME, EVENTS_OPT],
        r#"{"type":"object","required":["events"]}"#,
        Read,
        Agent,
        Quick,
    ),
    // ---- lifecycle
    cmd(
        "project_new",
        "Create a project folder and open it",
        &[
            opt("name", Ty::Str, "Project name"),
            opt("dir", Ty::Str, "Parent folder"),
            opt("audio_mode", Ty::Str, "referenced (default) or copied"),
        ],
        SUMMARY,
        Files,
        Agent,
        Quick,
    ),
    cmd(
        "project_save",
        "Write the project document; 409 when the file changed on disk since it was read, unless force",
        &[
            SID,
            opt("force", Ty::Bool, "Save anyway: write over a file that changed on disk"),
        ],
        r#"{"type":"object","required":["saved","dirty"]}"#,
        Files,
        Agent,
        Quick,
    ),
    cmd(
        "project_clean_check",
        "Clear the dirty flag when the open document equals the saved one (after an undo back to the saved state)",
        &[SID],
        r#"{"type":"object","required":["dirty"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "project_save_as",
        "Save into a new folder; with copy_audio the audio is copied",
        &[
            SID,
            opt("name", Ty::Str, "New project name"),
            opt("dir", Ty::Str, "Parent folder"),
            opt("copy_audio", Ty::Bool, "Copy the audio files"),
        ],
        r#"{"type":"object","description":"The project summary (sid, name, folder, document, dirty, order, tracks); with copy_audio {job} and the summary is the result of the finished job","properties":{"sid":{"type":"string"},"job":{"type":"object"}}}"#,
        Files,
        Agent,
        Progress,
    ),
    cmd(
        "project_close",
        "Close a project; 409 while it is dirty unless force",
        &[SID, opt("force", Ty::Bool, "Discard unsaved changes")],
        r#"{"type":"object","required":["closed","folder"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "project_rename",
        "Rename the project",
        &[SID, req("name", Ty::Str, "New name")],
        r#"{"type":"object","required":["name"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "tracks_reorder",
        "Set the display order of the tracks",
        &[SID, req("order", Ty::Strs, "All track names in the new order")],
        r#"{"type":"object","required":["order"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_set_note",
        "Set the MIDI note of a track",
        &[SID, NAME, req("note", Ty::Int, "0 to 127")],
        r#"{"type":"object","required":["note"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_set_follow",
        "Make a track take the hits of another track, or its own detection",
        &[
            SID,
            NAME,
            opt("source", Ty::Str, "Track to follow; none for own detection"),
        ],
        r#"{"type":"object","required":["follows","n_events"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_rename",
        "Rename a track",
        &[SID, NAME, req("new_name", Ty::Str, "New track name")],
        r#"{"type":"object","required":["name"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "app_settings_get",
        "The app settings",
        &[],
        r#"{"type":"object","description":"The app settings: key to value"}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "app_settings_set",
        "Write one app setting",
        &[
            req("key", Ty::Str, "Setting key, [a-z_]"),
            req("value", Ty::Any, "Value"),
        ],
        r#"{"type":"object","required":["key","value"]}"#,
        App,
        Agent,
        Quick,
    ),
    cmd(
        "project_recent",
        "The recent projects",
        &[],
        r#"{"type":"array","description":"The recent projects, newest first","items":{"type":"object"}}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "project_recent_clear",
        "Empty the recent projects",
        &[],
        r#"{"type":"object","description":"{cleared: the number of entries the list had}"}"#,
        App,
        Agent,
        Quick,
    ),
    cmd(
        "peaks_build_start",
        "Build the waveform caches; poll peaks_build_status",
        &[SID, opt("tracks", Ty::Strs, "Track names; default all")],
        r#"{"type":"object","required":["started"]}"#,
        Files,
        Agent,
        Poll("peaks_build_status"),
    ),
    cmd(
        "peaks_build_status",
        "State of the waveform cache build",
        &[SID],
        r#"{"type":"object","required":["state","tracks"]}"#,
        Read,
        Agent,
        Quick,
    ),
    // ---- window only
    cmd(
        "shell_sync",
        "The page tells the menu what is open",
        &[
            req("has_project", Ty::Bool, "A project is open"),
            opt("state", Ty::Any, "Menu state"),
        ],
        "{}",
        App,
        Window,
        Quick,
    ),
    cmd(
        "open_paths_take",
        "Paths the Finder or a drop queued",
        &[],
        OBJ,
        Read,
        Window,
        Quick,
    ),
    cmd("quit_app", "Quit the app", &[], "{}", App, Window, Quick),
    cmd(
        "app_ready",
        "The page is ready (ends the splash)",
        &[],
        OBJ,
        App,
        Window,
        Quick,
    ),
    cmd(
        "splash_painted",
        "The splash page painted its first frame (the 5 s minimum counts from it)",
        &[],
        OBJ,
        App,
        Window,
        Quick,
    ),
    cmd(
        "splash_dismiss",
        "A click or any key on the splash dismisses it early",
        &[],
        OBJ,
        App,
        Window,
        Quick,
    ),
    cmd(
        "export_existing",
        "The files that export_all with the same options would replace; checks dest",
        &[
            SID,
            DEST_OPT,
            opt("midi_mode", Ty::Str, "per_track (default) or single"),
            opt("tracks", Ty::Strs, "Track names; default all"),
            opt(
                "formats",
                Ty::Strs,
                "midi, spikes and/or audio; default midi and spikes",
            ),
            COMPENSATE_OPT,
        ],
        r#"{"type":"object","required":["count","files","dest"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "export_dest_set",
        "Keep a folder as the export folder of the project (a document field); none goes back to Exports/",
        &[SID, opt("dest", Ty::Str, "Absolute folder; none or empty clears")],
        r#"{"type":"object","required":["export_dest"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "export_markers_set",
        "Keep the Export dialog's Markers-in-MIDI checkbox for the project (a document field); the exported MIDI writes marker events",
        &[
            SID,
            req("on", Ty::Bool, "true: markers in the exported MIDI; false: off"),
        ],
        r#"{"type":"object","required":["export_markers"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "export_last_get",
        "The export shape of the project (what the Export dialog shows again and Export again repeats); none before the first export",
        &[SID],
        r#"{"type":"object","required":["export_last"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "export_last_set",
        "Keep the export shape of the project (a document field): the dialog's choices recorded at once and what an export ran with",
        &[
            SID,
            req("formats", Ty::Strs, "midi, spikes and/or audio"),
            opt("midi_mode", Ty::Str, "per_track (default) or single"),
            COMPENSATE_OPT,
            opt("scope", Ty::Str, "all (default) or selected"),
            opt("track", Ty::Str, "The track of scope selected"),
            DEST_OPT,
        ],
        r#"{"type":"object","required":["export_last"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "export_drag_prepare",
        "Pre-render the files of the Export dialog's drag tiles into Exports/.drag (hidden; a drag moves them out with export_drag_take)",
        &[
            SID,
            opt("midi_mode", Ty::Str, "per_track (default) or single"),
            opt("tracks", Ty::Strs, "Track names; default all"),
            opt(
                "formats",
                Ty::Strs,
                "midi, spikes and/or audio; default midi and spikes",
            ),
            COMPENSATE_OPT,
            opt("clear", Ty::Bool, "true: remove the pre-rendered drag files"),
        ],
        r#"{"type":"object","required":["dir","midi","spikes","errors","followers"]}"#,
        Files,
        Window,
        Quick,
    ),
    cmd(
        "export_drag_take",
        "A drag starts: move the staged drag files into Exports/ under their export names",
        &[SID, req("paths", Ty::Strs, "The files of export_drag_prepare")],
        r#"{"type":"object","required":["files"]}"#,
        Files,
        Window,
        Quick,
    ),
    cmd(
        "reveal_path",
        "Show a file or folder in the Finder",
        &[req("path", Ty::Str, "Absolute path")],
        "{}",
        App,
        Window,
        Quick,
    ),
    // ---- import, tracks, presets
    cmd(
        "legacy_list",
        "List the sessions of the old server in a folder",
        &[req("folder", Ty::Str, "Folder")],
        r#"{"type":"array","description":"Sessions of the old server","items":{"type":"object"}}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "project_import",
        "Open an old session folder as a new project",
        &[req("session_dir", Ty::Str, "Session folder")],
        SUMMARY,
        Files,
        Agent,
        Quick,
    ),
    cmd(
        "tracks_scan",
        "Guess role and note for audio files or folders (headers only; a file without a name role gets the audio's suggestion)",
        &[
            req("paths", Ty::Strs, "Files or folders"),
            opt("preset", Ty::Str, "Kit preset name"),
        ],
        r#"{"type":"object","required":["files","rejected","guessed"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "tracks_add",
        "Add one track per audio file",
        &[
            SID,
            req(
                "paths",
                Ty::Strs,
                "Files or folders; a folder with an `Audio Files` subfolder and no audio of its own is a Pro Tools session folder (the `.ptx` stays ignored)",
            ),
            opt(
                "tracks",
                Ty::Strs,
                "Filter on the scanned files by name pattern (exact, glob `t*`, range `t1-t4`); a pattern that matches no scanned file is an error",
            ),
            opt("roles", Ty::Any, "Role per file (in the order of the filtered files)"),
            opt("notes", Ty::Any, "MIDI note per file"),
            opt("copy", Ty::Bool, "Copy the audio into the project"),
        ],
        r#"{"type":"object","required":["added","tracks","rejected"]}"#,
        Files,
        Agent,
        Progress,
    ),
    cmd(
        "tracks_remove",
        "Remove tracks: hits, Learn, layer, Analysis folder; a follower becomes a normal track",
        &[
            SID,
            req("names", Ty::Strs, "Track names"),
            opt(
                "snapshot",
                Ty::Bool,
                "Also answer the removed tracks as JSON (the input of tracks_restore)",
            ),
        ],
        r#"{"type":"object","required":["removed","unfollowed","order","tracks"]}"#,
        Files,
        Agent,
        Quick,
    ),
    cmd(
        "tracks_restore",
        "Put removed tracks back (the undo of tracks_remove) from its snapshot",
        &[SID, req("tracks", Ty::Any, "The snapshot of tracks_remove")],
        r#"{"type":"object","required":["restored","order"]}"#,
        Files,
        Window,
        Quick,
    ),
    cmd(
        "kit_presets_list",
        "List the kit presets",
        &[],
        r#"{"type":"array","description":"The kit presets","items":{"type":"object"}}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "kit_preset_save",
        "Save a kit preset (name rules for roles)",
        &[req("name", Ty::Str, "Preset name"), req("rules", Ty::Any, "The rules")],
        r#"{"type":"object","required":["name","rules","path"]}"#,
        App,
        Agent,
        Quick,
    ),
    cmd(
        "kit_preset_load",
        "Read a kit preset",
        &[req("name", Ty::Str, "Preset name")],
        r#"{"type":"object","required":["name","rules"]}"#,
        Read,
        Agent,
        Quick,
    ),
    // ---- jobs, relink
    cmd(
        "project_consolidate",
        "Copy the referenced audio into the project folder",
        &[SID],
        r#"{"type":"object","required":["job"]}"#,
        Files,
        Agent,
        Progress,
    ),
    cmd(
        "job_status",
        "State of the copy or relink job of a project",
        &[SID],
        r#"{"type":"object","required":["state"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "job_cancel",
        "Cancel the copy or relink job of a project",
        &[SID],
        r#"{"type":"object","required":["sid","cancelling"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "project_relink",
        "Point an offline track at a file",
        &[
            SID,
            req("track", Ty::Str, "Track name"),
            req("path", Ty::Str, "Audio file"),
            opt("force", Ty::Bool, "Accept a file that does not match"),
        ],
        r#"{"type":"object","required":["track","status","source","matched","followers"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "relink_search",
        "Find the audio of offline tracks under a folder and relink them",
        &[SID, req("folder", Ty::Str, "Folder to search")],
        r#"{"type":"object","required":["offline","relinked","found","ambiguous","missing","followers"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    // ---- P4-3: histograms and detection params
    cmd(
        "track_histograms",
        "Candidate hits by sensitivity, peak levels, the largest gap",
        &[SID, NAME, EVENTS_OPT],
        r#"{"type":"object","required":["loudness","detection"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "track_repick",
        "Pick the hits again from the kept activations at other params (no model run)",
        &[
            SID,
            NAME,
            opt(
                "params",
                Ty::Obj,
                "Params: sensitivity or thresholds, min_gap_ms, refine",
            ),
            opt("store", Ty::Bool, "Store the result in the document"),
            opt(
                "keep_edited",
                Ty::Bool,
                "With store: keep the hits added or moved by hand (default true)",
            ),
        ],
        r#"{"type":"object","required":["events","n_events","weak_signal"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_velocity_set",
        "Set the velocity floor, ceiling and cut; stores the new velocities (apply)",
        &[
            SID,
            NAME,
            req("lo_db", Ty::Num, "Floor in dBFS (velocity 20)"),
            req("hi_db", Ty::Num, "Ceiling in dBFS (velocity 127)"),
            opt("cut_db", Ty::Num, "Loudness cut in dBFS; omit for none"),
            EVENTS_OPT,
            opt(
                "apply",
                Ty::Bool,
                "Also write the new velocities and cut marks to the stored hits (default true)",
            ),
        ],
        r#"{"type":"object","required":["events"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_params_set",
        "Store the detection params of a track (a sensitivity gives the thresholds)",
        &[SID, NAME, req("params", Ty::Obj, "Params object")],
        r#"{"type":"object","required":["params"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    // ---- P4-5: edits that lived in the page
    cmd(
        "events_edit",
        "Add, delete, move, set the velocity of or restore single hits",
        &[
            SID,
            NAME,
            req(
                "ops",
                Ty::Objs,
                "[{op: add, t, v?, added?: false, orig?, snap?: true} | {op: delete, index} | {op: move, index, t} | {op: velocity, index, v} | {op: restore, index}]; index counts in events_get before the call; t within the track (0 to its duration); restore takes the tom bleed mark off a hit; snap puts an added hit on the attack within 10 ms of t with the velocity of the audio (the answer's snaps list says where); move, velocity and restore may carry was {..} (the field presence the hit had before the edit: bool flags, orig number or null) for the page's byte-identical undo",
            ),
        ],
        r#"{"type":"object","required":["saved","added","deleted","moved","velocity","restored"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_shift_set",
        "Shift all hits of a track by samples (the Hitpoint shift); stored for the next analysis",
        &[
            SID,
            NAME,
            req(
                "shift_samples",
                Ty::Int,
                "Total shift, -100000 to 100000; positive is later",
            ),
        ],
        r#"{"type":"object","required":["shift_samples","delta","moved"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    // ---- P4-6a: Learn marks
    cmd(
        "track_learn_mark",
        "Mark examples of a track for Learn: a range or a hit, kind this or not (a follower marks its source)",
        &[
            SID,
            NAME,
            req(
                "kind",
                Ty::Str,
                "this (a good hit) or not (bleed, a ghost note to drop)",
            ),
            opt("t0", Ty::Num, "Range start in seconds (with t1)"),
            opt("t1", Ty::Num, "Range end in seconds"),
            opt("hit", Ty::Num, "The time of one hit or candidate, instead of a range"),
        ],
        r#"{"type":"object","required":["marked","learn","track","routed","counts"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_learn_remove",
        "Remove a Learn mark, or with at one example of it",
        &[
            SID,
            NAME,
            req("id", Ty::Str, "Mark id, for example m1"),
            opt("at", Ty::Num, "One example time of that mark, in seconds"),
        ],
        r#"{"type":"object","required":["removed","learn","track","routed"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_learn_forget_at",
        "Remove the Learn examples at one hit or inside a range (Forget This; no example there changes nothing)",
        &[
            SID,
            NAME,
            opt("t0", Ty::Num, "Range start in seconds (with t1)"),
            opt("t1", Ty::Num, "Range end in seconds"),
            opt("hit", Ty::Num, "The time of one hit, instead of a range"),
        ],
        r#"{"type":"object","required":["removed","learn","track","routed"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_learn_flip",
        "Change a Learn mark from this to not or back",
        &[SID, NAME, req("id", Ty::Str, "Mark id")],
        r#"{"type":"object","required":["kind","learn","track","routed"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_learn_forget",
        "Remove every Learn mark of a track (the settings stay)",
        &[SID, NAME],
        r#"{"type":"object","required":["forgotten","examples","learn","track","routed"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_learn_set",
        "Store the whole learn object of a track (the undo of a Learn step)",
        &[
            SID,
            NAME,
            req(
                "learn",
                Ty::Any,
                "The learn object {on, threshold, auto, method, marks: [{id, kind, t0, t1, at}]}",
            ),
            opt(
                "events",
                Ty::Any,
                "The hits of the track to store with it (the undo of a Learn step); without it the filter marks the hits again",
            ),
        ],
        r#"{"type":"object","required":["learn","track","routed"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_learn_scores",
        "Score the detection candidates of a track against its Learn examples (writes only the Analysis cache)",
        &[SID, NAME],
        r#"{"type":"object","required":["state","track","routed"]}"#,
        Read,
        Agent,
        Quick,
    ),
    // ---- P4-8: the tempo and meter map
    cmd(
        "tempo_map_get",
        "The tempo and meter map of the project (the default map when none is set)",
        &[SID],
        concat!(
            r#"{"type":"object","required":["map","default"],"properties":{"default":{"type":"boolean"},"map":"#,
            tempo_map_schema!(),
            "}}"
        ),
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "tempo_map_set",
        "Replace the whole tempo and meter map (one step; the answer has the previous map for undo)",
        &[
            SID,
            req(
                "map",
                Ty::TempoMap,
                "The whole map; every key is optional, {} or null is 120 BPM, 4/4, bar 1 at 0 s",
            ),
        ],
        concat!(
            r#"{"type":"object","required":["map","previous","changed"],"properties":{"changed":{"type":"boolean"},"map":"#,
            tempo_map_schema!(),
            r#","previous":{"description":"the map before, for undo; null when the stored value was not a valid map","anyOf":[{"type":"null"},"#,
            tempo_map_schema!(),
            "]}}}"
        ),
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "tempo_map_detect",
        "Suggest a tempo map (tempo, bar 1, meter, steps) from the kick and snare hits; read only, accept with tempo_map_set",
        &[
            SID,
            opt(
                "tracks",
                Ty::Strs,
                "Tracks to use, any role (default: every kick and snare track that follows no other)",
            ),
            opt(
                "min_bars",
                Ty::Int,
                "Bars a new tempo must hold to be a step, 1 to 64, default 4",
            ),
            opt(
                "min_step_pct",
                Ty::Num,
                "Smallest tempo step in percent, 0.1 to 50, default 2",
            ),
        ],
        concat!(
            r#"{"type":"object","required":["suggestion","start","bar1","meter","steps","fit","excursions","tracks","params","warnings","changed"],"properties":{
"suggestion":"#,
            tempo_map_schema!(),
            r#",
"start":{"type":"object","required":["bpm","error_bar","confidence","range_edge","alternatives"],"properties":{"bpm":{"type":"number","description":"the fitted tempo; the suggestion holds a whole BPM (0.5, 0.1) when it lies within error_bar"},"error_bar":{"type":"number","description":"BPM"},"confidence":{"type":"number","minimum":0,"maximum":1},"range_edge":{"type":"boolean","description":"the tempo lies within 3 % of an end of the search range"},"alternatives":{"type":"array","items":{"type":"object","properties":{"bpm":{"type":"number"},"score":{"type":"number","description":"relative to the chosen tempo; without the gate for a gated one"},"comb":{"type":"number","description":"comb score relative to the best peak"},"gated":{"type":"boolean","description":"comb under 60 % of the best: could not be chosen"}}}}}},
"bar1":{"type":"object","required":["t","confidence","backbeat","downbeat","alternative"],"properties":{"t":{"type":"number","description":"seconds"},"confidence":{"type":"number","minimum":0,"maximum":1,"description":"backbeat x downbeat"},"backbeat":{"type":"number","minimum":0,"maximum":1,"description":"how clearly the snare picks 2 and 4 against 1 and 3"},"downbeat":{"type":"number","minimum":0,"maximum":1,"description":"how clearly kicks, crash hits, fill endings and the first kick pick 1 against 3"},"alternative":{"type":"number","description":"seconds: the other candidate, two beats later in 4/4"}}},
"meter":{"type":"object","properties":{"value":{"type":"array","items":{"type":"integer"},"minItems":2,"maxItems":2},"confidence":{"type":"number"},"feel":{"type":["string","null"],"description":"two-beat: 2/4 or 6/8 in dotted quarters fit as well"},"r2":{"type":"number"},"r3":{"type":"number"},"r4":{"type":"number"},"rule":{"type":"string"}}},
"steps":{"type":"array","items":{"type":"object","properties":{"t":{"type":"number"},"bpm":{"type":"number"},"bar":{"type":"integer"},"bars":{"type":"integer"},"confidence":{"type":"number","minimum":0,"maximum":1}}}},
"fit":{"type":"object","properties":{"on_beat":{"type":"number","description":"share of hits within 1/8 beat of the grid"},"jitter_ms":{"type":"number"},"hits":{"type":"integer"},"beats":{"type":"integer"},"late":{"type":"integer","description":"hits after the end of the tracks, left out"},"isolated":{"type":"integer","description":"hits with no other hit within 6 s, left out"}}},
"excursions":{"type":"array","description":"tempo changes shorter than min_bars that the map has no step for and that pull the fitted tempo off; ask the user","items":{"type":"object","properties":{"t":{"type":"number"},"bars":{"type":"integer"},"bpm":{"type":"number"},"map_bpm":{"type":"number"}}}},
"tracks":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"role":{"type":"string"},"hits":{"type":"integer"}}}},
"params":{"type":"object","properties":{"min_bars":{"type":"integer"},"min_step_pct":{"type":"number"}}},
"warnings":{"type":"array","items":{"type":"string"}},
"changed":{"type":"boolean","description":"always false"}}}"#
        ),
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "tempo_map_refit_bar1",
        "The beat phase of `bpm` that the kick and snare hits support, as the new bar 1; commit it with tempo_map_set (one undo step)",
        &[SID, req("bpm", Ty::Num, "The new start tempo in BPM, at most 1000")],
        r#"{"type":"object","required":["bar1_s","support","hits","previous_bar1_s","bpm","changed"],"properties":{"bar1_s":{"type":"number","description":"the new time of bar 1 in seconds: the supported beat phase, at the occurrence nearest to the current bar 1 (within a quarter beat)"},"support":{"type":"number","description":"the kernel weight of the kick and snare hits on the phase"},"hits":{"type":"integer","description":"the kick and snare hits the phase comes from"},"previous_bar1_s":{"type":"number","description":"the bar 1 before, in seconds"},"bpm":{"type":"number"},"changed":{"type":"boolean","description":"always false"}}}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "tempo_map_import_midi",
        "Replace the map with the tempo and time-signature events of a MIDI file (a DAW export)",
        &[SID, req("path", Ty::Str, "MIDI file")],
        concat!(
            r#"{"type":"object","required":["map","previous","changed","ppq","tempo_events","meter_events","snapped"],"properties":{"changed":{"type":"boolean"},"ppq":{"type":"integer"},"tempo_events":{"type":"integer"},"meter_events":{"type":"integer"},"snapped":{"type":"integer"},"map":"#,
            tempo_map_schema!(),
            r#","previous":{"description":"the map before; null when the stored value was not a valid map","anyOf":[{"type":"null"},"#,
            tempo_map_schema!(),
            "]}}}"
        ),
        Doc,
        Agent,
        Quick,
    ),
    // ---- P4-40g: review markers (the list rides as the `markers` pass-through key of the document)
    cmd(
        "markers_get",
        "The review markers of the project: [{t seconds, note?}]",
        &[SID],
        r#"{"type":"object","required":["markers","n"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "markers_set",
        "Replace the whole marker list (an empty list clears it); the answer carries the `previous` list, so one more call undoes it",
        &[
            SID,
            req(
                "markers",
                Ty::Objs,
                "The markers: [{t seconds, note?}] (stored sorted by t; an entry may carry more keys, they are kept)",
            ),
        ],
        r#"{"type":"object","required":["markers","previous","changed","n"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    // ---- #167: per-track lane heights (the map rides as the `row_heights` pass-through key of the document)
    cmd(
        "row_heights_get",
        "The per-track lane-height factors: {track: factor}; the row's wave lane is the global wave height times the factor",
        &[SID],
        r#"{"type":"object","required":["heights","n"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "row_heights_set",
        "Replace the whole per-track lane-height map (an empty map clears it); each factor divides the lane by the global height",
        &[SID, req("heights", Ty::Obj, "The map: {track: positive factor}")],
        r#"{"type":"object","required":["heights","changed","n"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    // ---- #150: the review queue (the confidence of each hit; computed, never stored)
    cmd(
        "track_confidence",
        "The confidence of each hit (0..1), doubtful under 0.5, with the reasons: [{c, doubt, why}] in the order of the hits",
        &[SID, NAME, EVENTS_OPT],
        r#"{"type":"object","required":["confidence","source","doubt_below"]}"#,
        Read,
        Agent,
        Quick,
    ),
    // ---- P4-7a and P4-7b: the one-shot layer of a track
    cmd(
        "track_layer_set",
        "Set, change or remove the one-shot layer of a track (a WAV or AIFF file played and rendered at every hit)",
        &[
            SID,
            NAME,
            opt(
                "path",
                Ty::Any,
                "One-shot file (WAV or AIFF); null or an empty text removes the layer; omit to keep the file",
            ),
            opt("level_db", Ty::Num, "Level in dB, -60 to +12"),
            opt(
                "polarity",
                Ty::Str,
                "normal, invert or auto (the majority of the per-hit alignment)",
            ),
            opt("on", Ty::Bool, "Heard in the playback"),
            opt(
                "curve",
                Ty::Obj,
                "Velocity curve: {kind: db, range_db} (default 24 dB), {kind: linear} or {kind: fixed}",
            ),
            opt(
                "align",
                Ty::Bool,
                "Use the per-hit alignment to the close mic in the playback and the render",
            ),
            opt(
                "onset",
                Ty::Num,
                "P4-40e: the onset of the one-shot in its own samples, overriding the measured one for the render, the playback and track_layer_align; null resets to the measured onset",
            ),
        ],
        r#"{"type":"object","required":["layer","status","path","file","onset_ms"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_layer_get",
        "The one-shot layer of a track, the state of its file and its onset",
        &[SID, NAME],
        r#"{"type":"object","required":["layer","status","path","file","onset_ms"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "track_layer_align",
        "Align the layer's one-shot to the close mic at every hit: lag (samples), match 0 to 1, polarity by majority",
        &[SID, NAME],
        r#"{"type":"object","required":["n","sr","polarity","sign","as_is","inverted","flips","median_match","uncertain","hits"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "track_layer_wave",
        "Audio samples of a track's layer one-shot (bytes: u32 start, u32 rate, then f32); P4-40e, the Layer view's waveform",
        &[
            SID,
            NAME,
            opt("start", Ty::Int, "First sample (0)"),
            opt("end", Ty::Int, "End sample (the file end)"),
        ],
        "{}",
        Read,
        Window,
        Quick,
    ),
    cmd(
        "export_layer",
        "Write Exports/<name>_layer.wav (or `path`): the one-shot at every hit, the source's rate, length and bext time reference",
        &[
            SID,
            NAME,
            opt(
                "path",
                Ty::Str,
                "The file to write (absolute; not a source or a one-shot in use); default Exports/<name>_layer.wav",
            ),
        ],
        r#"{"type":"object","required":["path","size","n_events","sr","n_samples","channels","format","time_reference","onset","aligned","sign","outside"]}"#,
        Files,
        Agent,
        Quick,
    ),
    cmd(
        "sampler_calibration_midi",
        "Write <track>_calibration.mid: one note on bar 2; import it at the session start, bounce the sampler, load the bounce",
        &[
            SID,
            NAME,
            DEST_OPT,
            opt(
                "path",
                Ty::Str,
                "The file to write (absolute); replaces dest and the file name",
            ),
        ],
        r#"{"type":"object","required":["path","size","note","t","tick","text"]}"#,
        Files,
        Agent,
        Quick,
    ),
    cmd(
        "track_sampler_get",
        "The sampler calibration of a track: offset of the transient after the note, polarity of the bounce",
        &[SID, NAME],
        r#"{"type":"object","required":["sampler","offset_ms","polarity","file","layer"]}"#,
        Read,
        Agent,
        Quick,
    ),
    cmd(
        "track_sampler_set",
        "Load the calibration bounce: measure the sampler offset and polarity; MIDI exports then place notes minus the offset",
        &[
            SID,
            NAME,
            opt(
                "path",
                Ty::Str,
                "The bounce (WAV or AIFF, from the session start); empty or none removes the calibration",
            ),
            opt(
                "one_shot",
                Ty::Bool,
                "Also set the bounce as the track's layer one-shot",
            ),
        ],
        r#"{"type":"object","required":["sampler","offset_ms","polarity","file","layer"]}"#,
        Doc,
        Agent,
        Quick,
    ),
    cmd(
        "track_sampler_wave",
        "Audio samples of a track's sampler bounce (bytes: u32 start, u32 rate, then f32); the Layer view's bounce trace",
        &[
            SID,
            NAME,
            opt("start", Ty::Int, "First sample (0)"),
            opt("end", Ty::Int, "End sample (the file end)"),
        ],
        "{}",
        Read,
        Window,
        Quick,
    ),
    // ---- debug builds
    cmd(
        "selftest_clock",
        "Self-test clock",
        &[],
        r#"{"type":"number"}"#,
        Read,
        Selftest,
        Quick,
    ),
    cmd(
        "selftest_memory",
        "Self-test memory probe",
        &[],
        OBJ,
        Read,
        Selftest,
        Quick,
    ),
    cmd(
        "selftest_report",
        "Self-test report",
        &[
            req("report", Ty::Any, "The report"),
            opt("via_quit", Ty::Bool, "Report came on quit"),
        ],
        "{}",
        App,
        Selftest,
        Quick,
    ),
    cmd("splash_report", "Splash timing report", &[], OBJ, Read, Selftest, Quick),
    cmd(
        "selftest_menu",
        "Send a menu event",
        &[req("id", Ty::Str, "Menu item id")],
        OBJ,
        App,
        Selftest,
        Quick,
    ),
    cmd(
        "selftest_menu_report",
        "Menu event report",
        &[],
        OBJ,
        Read,
        Selftest,
        Quick,
    ),
    cmd(
        "selftest_ns_menu",
        "The main menu as macOS has it: NSMenu key equivalents and modifier masks",
        &[],
        OBJ,
        Read,
        Selftest,
        Quick,
    ),
    cmd(
        "selftest_menu_key",
        "A menu_key through the real native menu path (NSMenu performKeyEquivalent)",
        &[req("key", Ty::Str, "The key combination, as the keys driver spells it")],
        OBJ,
        App,
        Selftest,
        Quick,
    ),
    cmd(
        "selftest_menu_log",
        "The menu ids that reached dispatch so far",
        &[],
        OBJ,
        Read,
        Selftest,
        Quick,
    ),
    cmd(
        "selftest_menu_probe",
        "A synthetic keyDown with explicit characters and flags, to probe which variant the NSMenu matches",
        &[
            req("chars", Ty::Str, "The characters of the event"),
            req("flags", Ty::Int, "The NSEvent modifier flags"),
        ],
        OBJ,
        App,
        Selftest,
        Quick,
    ),
    cmd(
        "selftest_open_paths",
        "Queue paths as the Finder does",
        &[req("paths", Ty::Strs, "Paths")],
        "{}",
        App,
        Selftest,
        Quick,
    ),
    cmd(
        "selftest_close_window",
        "Close the window",
        &[],
        "{}",
        App,
        Selftest,
        Quick,
    ),
];

/// The row of a command.
#[must_use]
pub fn find(name: &str) -> Option<&'static CommandSpec> {
    REGISTRY.iter().find(|c| c.name == name)
}

/// The commands of the command-line tool and the MCP server.
pub fn agent_commands() -> impl Iterator<Item = &'static CommandSpec> {
    REGISTRY.iter().filter(|c| c.surface == Surface::Agent)
}

/// The MCP tool list entry of a command: `{name, description, inputSchema}`.
#[must_use]
pub fn tool_entry(c: &CommandSpec) -> Value {
    json!({"name": c.name, "description": c.about, "inputSchema": c.args_schema()})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique_and_every_schema_is_valid_json() {
        let mut seen = HashSet::new();
        for c in REGISTRY {
            assert!(seen.insert(c.name), "{} twice", c.name);
            assert!(!c.about.is_empty() && c.about.len() < 130, "{}: one short line", c.name);
            let _ = c.result_schema();
            let a = c.args_schema();
            assert_eq!(a["type"], "object");
            for r in a["required"].as_array().unwrap() {
                assert!(a["properties"][r.as_str().unwrap()].is_object());
            }
            let mut names = HashSet::new();
            assert!(
                c.args.iter().all(|x| names.insert(x.name)),
                "{}: an argument twice",
                c.name
            );
        }
    }

    #[test]
    fn every_result_schema_of_the_agent_surface_says_something() {
        for c in agent_commands() {
            let r = c.result_schema();
            let info = ["required", "properties", "items", "description"]
                .iter()
                .any(|k| r.get(k).is_some());
            assert!(info, "{}: the result schema is bare: {r}", c.name);
            assert!(r["type"] == "object" || r["type"] == "array", "{}", c.name);
        }
        assert!(!find("project_list").unwrap().result_schema()["items"].is_null());
    }

    #[test]
    fn a_command_that_polls_names_a_status_command_of_the_registry() {
        for c in REGISTRY {
            if let Run::Poll(status) = c.run {
                let s = find(status).unwrap_or_else(|| panic!("{}: no status command {status}", c.name));
                assert_eq!(s.effect, Effect::Read);
            }
        }
        assert!(find("analyze_start").unwrap().has_progress());
        assert!(!find("project_open").unwrap().has_progress());
    }

    #[test]
    fn the_flags_follow_the_effect_and_the_surface() {
        assert!(find("events_edit").unwrap().writes_project());
        assert!(find("export_all").unwrap().writes_project());
        assert!(!find("project_get").unwrap().writes_project());
        assert!(!find("app_settings_set").unwrap().writes_project());
        assert!(agent_commands().all(|c| c.surface == Surface::Agent));
        assert!(find("selftest_clock").is_some_and(|c| c.surface == Surface::Selftest));
        let t = tool_entry(find("track_velocity_set").unwrap());
        assert_eq!(t["inputSchema"]["required"], json!(["sid", "name", "lo_db", "hi_db"]));
        assert!(find("no_such_command").is_none());
    }

    /// A command of the agent surface is run by the test bridge's dispatcher (the one `call` of
    /// stage 2 replaces): with no arguments it fails for a missing argument, never as unknown.
    #[test]
    fn the_bridge_runs_every_command_of_the_agent_surface() {
        let root = std::env::temp_dir().join(format!("ds_api_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let svc = crate::Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        svc.set_home(root.join("home"));
        svc.set_projects_dir(root.join("projects"));
        let b = crate::bridge::Bridge::new(svc, &root);
        for c in agent_commands() {
            if let Err(e) = b.invoke(c.name, &json!({})) {
                assert!(!e.message.starts_with("unknown command"), "{}: {}", c.name, e.message);
            }
        }
        assert!(
            b.invoke("no_such_command", &json!({}))
                .unwrap_err()
                .message
                .starts_with("unknown command")
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
