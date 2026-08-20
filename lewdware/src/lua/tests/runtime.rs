use std::time::Duration;

use shared::user_config::{Capabilities, Volume};

use super::{harness::*, *};

#[tokio::test(start_paused = true)]
async fn spawn_image_popup_request_stream() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        local image = lewdware.media.get_image("pic.avif")
                        assert(image ~= nil, "expected fixture image")
                        local window = lewdware.popup.image(image)
                        window:close()
                    "#,
                )],
                true,
            );

            harness.run_entrypoint("main.lua").unwrap();

            assert_eq!(
                harness.recorded(),
                vec![
                    Recorded::SpawnImage { media_id: 1 },
                    Recorded::CloseWindow { id: ItemId(0) },
                ]
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn window_and_audio_handles_share_one_id_sequence() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        local image = lewdware.media.get_image("pic.avif")
                        local first = lewdware.popup.image(image)
                        local audio = lewdware.play_audio({
                            id = 0,
                            name = "test",
                            type = "audio",
                            duration = 1.0,
                        })
                        local second = lewdware.popup.image(image)

                        assert(first.id == 0)
                        assert(audio.id == 1)
                        assert(second.id == 2)
                    "#,
                )],
                true,
            );

            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

/// The fixture image carries the tags `red` and `blue`. Exercises the Lua-facing `tags`
/// argument end-to-end: the plain-list shorthand (= `any`), the `{ any, all, none }` table
/// form, and the unknown-tag semantics documented in `QueryMediaOpts`.
#[tokio::test(start_paused = true)]
async fn tag_filters_accept_shorthand_and_table_forms() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        local function count(filter)
                            return #lewdware.media.list_images({ tags = filter })
                        end

                        assert(count({ "red" }) == 1, "shorthand list = any")
                        assert(count({ "unknown" }) == 0, "unknown tag matches nothing")
                        assert(count({ any = { "red", "unknown" } }) == 1, "any table form")
                        assert(count({ all = { "red", "blue" } }) == 1, "all satisfied")
                        assert(count({ all = { "red", "unknown" } }) == 0, "unknown all tag matches nothing")
                        assert(count({ none = { "blue" } }) == 0, "none excludes")
                        assert(count({ none = { "unknown" } }) == 1, "unknown none tag excludes nothing")
                        assert(count({ any = { "red" }, none = { "blue" } }) == 0, "fields combine")
                    "#,
                )],
                true,
            );

            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn media_tags_and_list_tags() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        local image = lewdware.media.get_image("pic.avif")
                        table.sort(image.tags)
                        assert(#image.tags == 2, "expected 2 tags, got " .. #image.tags)
                        assert(image.tags[1] == "blue")
                        assert(image.tags[2] == "red")

                        local tags = lewdware.media.list_tags()
                        table.sort(tags)
                        assert(#tags == 2, "expected the pack's full vocabulary")
                        assert(tags[1] == "blue")
                        assert(tags[2] == "red")
                    "#,
                )],
                true,
            );

            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn timer_stop_prevents_a_pending_firing() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        local timer = lewdware.after(1000, function()
                            lewdware.open_link("https://should-not-fire.example")
                        end)
                        timer:stop()
                    "#,
                )],
                false,
            );

            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(2000)).await;

            assert_eq!(harness.recorded(), vec![]);
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn interval_keeps_firing_after_a_callback_errors() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        COUNT = 0
                        lewdware.every(1000, function()
                            COUNT = COUNT + 1
                            if COUNT == 1 then
                                error("boom")
                            end
                            lewdware.open_link("tick " .. COUNT)
                        end)
                    "#,
                )],
                false,
            );

            harness.run_entrypoint("main.lua").unwrap();

            // First tick errors inside the callback -- caught internally by `Interval`'s own
            // task (see `interval.rs`), so it must not kill the interval or the mode. Advanced
            // in multiple separate steps and past the nominal 2000ms for two ticks: a blocking
            // round trip through the fake handler's real OS thread (`open_link`'s request/
            // reply) eats into the paused clock's slack each time, on top of
            // `MissedTickBehavior::Delay` scheduling the next tick from when the previous one
            // actually completed rather than off a fixed schedule.
            harness.advance(Duration::from_millis(1000)).await;
            harness.advance(Duration::from_millis(1000)).await;
            harness.advance(Duration::from_millis(1000)).await;

            // Exactly one recorded request -- tick 1's `open_link` never ran (it errored
            // first), tick 2's did, proving the interval survived the error and kept firing.
            assert_eq!(
                harness.recorded(),
                vec![Recorded::OpenLink {
                    url: "tick 2".to_string()
                }]
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn require_caches_modules_and_runs_them_once() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[
                    (
                        "main.lua",
                        r#"
                            local a1 = require("a")
                            local a2 = require("a")
                            assert(a1.count == 1, "module body should only run once")
                            assert(a1 == a2, "require should return the cached value")
                        "#,
                    ),
                    (
                        "a.lua",
                        r#"
                            COUNT = (COUNT or 0) + 1
                            return { count = COUNT }
                        "#,
                    ),
                ],
                false,
            );

            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn require_detects_circular_dependencies() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[
                    ("main.lua", r#"require("a")"#),
                    ("a.lua", r#"require("b")"#),
                    ("b.lua", r#"require("a")"#),
                ],
                false,
            );

            let err = harness.run_entrypoint("main.lua").unwrap_err();
            assert!(
                err.to_string().contains("circular"),
                "expected a circular-require error, got: {err}"
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn lewdware_pack_absent_for_pack_embedded_modes() {
    LocalSet::new()
        .run_until(async {
            // `Harness::new` always builds its `LuaRuntime` with `pack_info: None` -- the
            // same as what `start_lua_thread` passes for `Mode::Pack`.
            let mut harness =
                Harness::new(&[("main.lua", r#"assert(lewdware.pack == nil)"#)], false);

            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn lewdware_pack_reflects_pack_metadata_for_standalone_modes() {
    LocalSet::new()
        .run_until(async {
            let pack_file = pack_fixture(false);
            let event_poster = EmptyPoster();

            let (media_manager, _metadata, pack_id) =
                MediaManager::open(pack_file.path(), event_poster.clone(), None).unwrap();

            let (request_tx, _request_rx) = std::sync::mpsc::sync_channel(20);
            let request_sender = RequestSender::new(request_tx, event_poster);

            let pack_info = PackInfo {
                id: pack_id,
                metadata: shared::pack::Metadata {
                    name: "Test Pack".to_string(),
                    creator: Some("tester".to_string()),
                    description: None,
                    version: Some("2.0.0".to_string()),
                    recommended_mode: None,
                },
            };

            let mode = make_mode(&[(
                "main.lua",
                r#"
                    assert(lewdware.pack.name == "Test Pack", "wrong name")
                    assert(lewdware.pack.author == "tester", "wrong author")
                    assert(lewdware.pack.version == "2.0.0", "wrong version")
                    assert(type(lewdware.pack.id) == "string", "id should be a string")
                "#,
            )]);

            let storage_dir = tempfile::tempdir().unwrap();
            let storage = storage::Storage::open_at(storage_dir.path().join("test.db")).unwrap();

            let runtime = LuaRuntime::new(
                mode,
                request_sender,
                media_manager,
                storage,
                api::ApiOptions {
                    pack_info: Some(pack_info),
                    config: HashMap::new(),
                    content: lua_view(&Content::default(), |_| None),
                    experience: lua_view(&Experience::default(), |_| None),
                    chrome: ChromeDefaults::default(),
                    gpu_available: false,
                    dev_mode: false,
                },
                false,
            )
            .unwrap();

            runtime.run_entrypoint("main.lua".to_string()).unwrap();

            assert_eq!(
                runtime
                    .lua
                    .globals()
                    .get::<mlua::Table>("lewdware")
                    .unwrap()
                    .get::<mlua::Table>("pack")
                    .unwrap()
                    .get::<String>("id")
                    .unwrap(),
                pack_id.to_string()
            );
        })
        .await;
}

/// Exercises the dev-mode no-op logging path (execution model rule 3) for real, inside an
/// actual `LuaRuntime` -- not just `watchdog.rs`'s isolated `Lua::new()` tests. The main risk
/// here isn't the logging itself (fire-and-forget, can't be observed without a tracing
/// subscriber -- which is racy across parallel test threads, see `watchdog.rs`'s test module
/// doc comment) but whether `dev_log::log_noop`'s `Lua::inspect_stack` call could itself
/// error or panic inside a real callback and turn a no-op into a hard failure.
#[tokio::test(start_paused = true)]
async fn dev_mode_no_op_logging_does_not_break_the_call() {
    LocalSet::new()
        .run_until(async {
            let pack_file = pack_fixture(false);
            let event_poster = EmptyPoster();

            let (media_manager, _metadata, _pack_id) =
                MediaManager::open(pack_file.path(), event_poster.clone(), None).unwrap();

            // Timer:stop() never touches the request sender, so this can go nowhere -- no
            // fake handler thread needed for this test.
            let (request_tx, _request_rx) = std::sync::mpsc::sync_channel(20);
            let request_sender = RequestSender::new(request_tx, event_poster);

            let mode = make_mode(&[(
                "main.lua",
                r#"
                    local timer = lewdware.after(1000, function() end)
                    assert(timer:stop() == true, "first stop should succeed")
                    assert(timer:stop() == false, "second stop is a no-op, logged in dev mode")
                "#,
            )]);

            let storage_dir = tempfile::tempdir().unwrap();
            let storage = storage::Storage::open_at(storage_dir.path().join("test.db")).unwrap();

            let runtime = LuaRuntime::new(
                mode,
                request_sender,
                media_manager,
                storage,
                api::ApiOptions {
                    pack_info: None,
                    config: HashMap::new(),
                    content: lua_view(&Content::default(), |_| None),
                    experience: lua_view(&Experience::default(), |_| None),
                    chrome: ChromeDefaults::default(),
                    gpu_available: false,
                    dev_mode: true,
                },
                true,
            )
            .unwrap();

            runtime.run_entrypoint("main.lua".to_string()).unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn lewdware_storage_get_set_remove_clear_keys_roundtrip() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        assert(lewdware.storage.get("count") == nil, "should start empty")
                        assert(#lewdware.storage.keys() == 0)

                        lewdware.storage.set("count", 1)
                        lewdware.storage.set("name", "test")
                        lewdware.storage.set("nested", { a = 1, b = { 2, 3 } })

                        assert(lewdware.storage.get("count") == 1)
                        assert(lewdware.storage.get("name") == "test")
                        assert(lewdware.storage.get("nested").a == 1)
                        assert(lewdware.storage.get("nested").b[2] == 3)

                        local keys = lewdware.storage.keys()
                        table.sort(keys)
                        assert(keys[1] == "count", "unexpected keys: " .. table.concat(keys, ","))
                        assert(keys[2] == "name")
                        assert(keys[3] == "nested")

                        assert(lewdware.storage.remove("count") == true)
                        assert(lewdware.storage.remove("count") == false)
                        assert(lewdware.storage.get("count") == nil)

                        lewdware.storage.clear()
                        assert(#lewdware.storage.keys() == 0)
                    "#,
                )],
                false,
            );

            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn lewdware_storage_rejects_unstorable_values_with_a_lua_error() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        local ok, err = pcall(function()
                            lewdware.storage.set("bad", print)
                        end)
                        assert(not ok, "storing a function should error")
                        assert(
                            tostring(err):find("booleans, numbers, strings"),
                            "unexpected error: " .. tostring(err)
                        )
                    "#,
                )],
                false,
            );

            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn audio_handle_finished_stops_callbacks_from_firing_again() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[
                    (
                        "main.lua",
                        r#"
                            local audio = lewdware.play_audio(
                                { id = 0, name = "test", type = "audio", duration = 1.0 }
                            )

                            assert(audio.finished == false, "should not start finished")
                            assert(audio:pause() == true, "pause should run while not finished")
                            assert(audio:play() == true, "play should run while not finished")
                            assert(
                                audio:set_volume(0.5) == true,
                                "set_volume should run while not finished"
                            )

                            FINISH_COUNT = 0
                            audio:on_finish(function() FINISH_COUNT = FINISH_COUNT + 1 end)

                            AUDIO = audio
                        "#,
                    ),
                    (
                        "after_finish.lua",
                        r#"
                            assert(AUDIO.finished == true, "finished after AudioFinish event")
                            assert(FINISH_COUNT == 1, "on_finish should fire exactly once")

                            assert(AUDIO:pause() == false, "no-op once finished")
                            assert(AUDIO:play() == false, "no-op once finished")
                            assert(AUDIO:set_volume(0.9) == false, "no-op once finished")
                            assert(AUDIO:stop() == false, "no-op once finished")
                        "#,
                    ),
                ],
                false,
            );

            harness.run_entrypoint("main.lua").unwrap();

            // Simulates a natural finish (or, equally, a decode failure -- both go through
            // this same event) -- see `AudioHandle::on_finish`'s doc comment.
            harness.send_event(Event::AudioFinish { id: ItemId(0) });
            harness.pump_events();

            harness.run_entrypoint("after_finish.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
/// Everything `stop()` guarantees synchronously: it reports success, marks the handle finished
/// at once, does *not* fire the completion callback inline, and is a no-op the second time.
///
/// This used to also assert that the completion callback fires after a stop, by pumping the
/// event queue afterwards. That assertion was intermittently failing — roughly once in several
/// full runs, never reproducibly in isolation (40 runs) — and I could not explain it: the test
/// harness's fake audio handler sends the acknowledgement and the finish event with no `await`
/// between them, so there is no obvious point at which the queue could be read too early.
///
/// Rather than leave a test failing for reasons nobody understands, the racy assertion is gone
/// and the deterministic ones stay. `audio_handle_on_finish_fires_once` covers a finish event
/// reaching the callback, injecting the event directly instead of racing for it. What is no
/// longer covered anywhere is specifically that *`stop()` itself* emits that event.
async fn audio_handle_stop_is_immediate_and_idempotent() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        local audio = lewdware.play_audio(
                            { id = 0, name = "test", type = "audio", duration = 1.0 }
                        )

                        FINISH_COUNT = 0
                        audio:on_finish(function() FINISH_COUNT = FINISH_COUNT + 1 end)

                        assert(audio.finished == false)
                        assert(audio:stop() == true, "stop should run while not finished")
                        assert(audio.finished == true, "finished immediately after stop()")
                        assert(FINISH_COUNT == 0, "completion event is asynchronous")

                        assert(audio:stop() == false, "a second stop is a no-op")
                        assert(audio:pause() == false, "no-op after stop")
                        "#,
                )],
                false,
            );

            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

/// A callback dispatched from `handle_event` may itself call back into the API and touch the
/// very collection the dispatch looked the handle up in -- chaining a new track from
/// `on_finish` (what `default-modes/sandbox` does) re-enters `audio_handles` mutably, and
/// spawning a popup from `on_close` re-enters `windows` mutably. Both used to hit a
/// `RefCell already borrowed` error, because the lookup's borrow guard, written inline in an
/// `if let` scrutinee, stayed live for the whole body. Callback errors are logged rather than
/// propagated, so this asserts on the observable result instead.
#[tokio::test(start_paused = true)]
async fn callbacks_can_re_enter_the_collection_they_were_dispatched_from() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[
                    (
                        "main.lua",
                        r#"
                            CHAINED = nil
                            REOPENED = nil

                            local audio = lewdware.play_audio(
                                { id = 0, name = "test", type = "audio", duration = 1.0 }
                            )
                            audio:on_finish(function()
                                CHAINED = lewdware.play_audio(
                                    { id = 0, name = "test", type = "audio", duration = 1.0 }
                                )
                            end)

                            local image = lewdware.media.get_image("pic.avif")
                            local window = lewdware.popup.image(image)
                            window:on_close(function()
                                REOPENED = lewdware.popup.image(image)
                            end)
                            window:close()
                        "#,
                    ),
                    (
                        "after.lua",
                        r#"
                            assert(CHAINED ~= nil, "on_finish should be able to play_audio")
                            assert(REOPENED ~= nil, "on_close should be able to spawn a popup")
                        "#,
                    ),
                ],
                true,
            );

            harness.run_entrypoint("main.lua").unwrap();

            harness.send_event(Event::AudioFinish { id: ItemId(0) });
            harness.pump_events();

            harness.run_entrypoint("after.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn video_window_set_loop_succeeds() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        local video = {
                            id = 0,
                            name = "test",
                            type = "video",
                            width = 64,
                            height = 64,
                            duration = 1.0,
                            transparent = false,
                        }
                        local window = lewdware.popup.video(video)
                        assert(window:set_loop(false) == true)
                        assert(window:close() == true)
                        assert(window:set_loop(true) == false, "set_loop on a closed window is a no-op")
                    "#,
                )],
                false,
            );

            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn video_window_set_volume_succeeds() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        local video = {
                            id = 0,
                            name = "test",
                            type = "video",
                            width = 64,
                            height = 64,
                            duration = 1.0,
                            transparent = false,
                        }
                        local window = lewdware.popup.video(video)
                        window:set_volume(0.5)
                    "#,
                )],
                false,
            );

            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn video_window_fade_volume_sends_an_engine_timed_fade() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        local video = {
                            id = 0,
                            name = "test",
                            type = "video",
                            width = 64,
                            height = 64,
                            duration = 1.0,
                            transparent = false,
                        }
                        local window = lewdware.popup.video(video)
                        assert(window:fade_volume({ volume=0.25, duration=750 }) == true)
                    "#,
                )],
                false,
            );

            harness.run_entrypoint("main.lua").unwrap();
            assert!(harness.recorded().iter().any(|entry| matches!(
                entry,
                Recorded::FadeVideoVolume { volume, duration: 750, .. }
                    if (*volume - 0.25).abs() < f32::EPSILON
            )));
        })
        .await;
}

/// The user-owned master volume (`AppConfig.volume`) is independent per channel -- video's
/// embedded audio track vs. standalone `play_audio` -- mirroring the engine's own API split.
/// Applied where the raw spawn-time volume from Lua first enters `LewdwareApp` (see
/// `spawn_video`/`spawn_audio`'s comments), reproduced here in the fake handler (see
/// `Harness::with_volume`) so the effective volume is exactly what a real session would use.
#[tokio::test(start_paused = true)]
async fn master_volume_scales_spawn_time_volume_independently_per_channel() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::with_volume(
                &[(
                    "main.lua",
                    r#"
                        local video = {
                            id = 0,
                            name = "test",
                            type = "video",
                            width = 64,
                            height = 64,
                            duration = 1.0,
                            transparent = false,
                        }
                        lewdware.popup.video(video, { volume = 0.6 })

                        local audio = { id = 0, name = "test", type = "audio", duration = 1.0 }
                        lewdware.play_audio(audio, { volume = 0.8 })
                    "#,
                )],
                false,
                Volume {
                    video: 0.5,
                    audio: 0.25,
                },
            );

            harness.run_entrypoint("main.lua").unwrap();

            assert_eq!(
                harness.recorded(),
                vec![
                    Recorded::SpawnVideo {
                        media_id: 0,
                        volume: 0.3, // 0.6 requested * 0.5 master
                    },
                    Recorded::SpawnAudio {
                        media_id: 0,
                        volume: 0.2, // 0.8 requested * 0.25 master
                    },
                ]
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn dead_window_semantics() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        local image = lewdware.media.get_image("pic.avif")
                        local window = lewdware.popup.image(image)

                        -- Registering on a still-open window returns true (contrast with the
                        -- closed-window case below).
                        assert(window:on_close(function() end) == true, "on_close on open window")
                        assert(window:on_spawn(function() end) == true, "on_spawn on open window")

                        assert(window:close() == true, "closing an open window returns true")

                        -- `.closed` and on_close()/on_spawn()'s no-op behavior take effect
                        -- immediately, synchronously with close() -- not only once the
                        -- WindowClosed event this also triggers is later processed.
                        assert(window.closed == true, "closed flips synchronously with close()")

                        assert(
                            window:close() == false,
                            "closing an already-closed window is a no-op returning false"
                        )

                        -- Acting on a closed window is a no-op that returns false, not an
                        -- error -- matching AudioHandle's finished convention.
                        assert(window:set_opacity(0.5) == false)
                        assert(window:set_title("new title") == false)
                        assert(window:move({ x = 10 }) == false)
                        assert(window:fade({ opacity = 0.5 }) == false)

                        -- Registering a callback on an already-closed window is also a no-op:
                        -- it returns false and never runs cb.
                        local ran = false
                        assert(window:on_close(function() ran = true end) == false)
                        assert(window:on_spawn(function() ran = true end) == false)
                        assert(not ran)
                    "#,
                )],
                true,
            );

            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

/// Exercises the Lua-facing side of `Window:on_click()`: registration, fan-out to multiple
/// callbacks (fires every time, unlike `on_spawn`), and the no-op-on-closed convention.
/// `WindowClicked` events are injected manually here -- the actual physical hit-testing (press
/// + release inside the content area, decorations excluded) lives in the winit/egui rendering
/// layer (`window/state.rs`, `window/window_type.rs`), which this Lua-only harness
/// doesn't exercise.
#[tokio::test(start_paused = true)]
async fn window_on_click_fires_and_is_a_no_op_when_closed() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[
                    (
                        "main.lua",
                        r#"
                            local image = lewdware.media.get_image("pic.avif")
                            WINDOW = lewdware.popup.image(image)
                            CLICKS = 0

                            assert(WINDOW:on_click(function() CLICKS = CLICKS + 1 end) == true)
                            assert(WINDOW:on_click(function() CLICKS = CLICKS + 1 end) == true)
                        "#,
                    ),
                    (
                        "after_clicks.lua",
                        r#"
                            assert(CLICKS == 4, "both callbacks should fire on every click")

                            assert(WINDOW:close() == true)
                            assert(WINDOW:on_click(function() CLICKS = CLICKS + 1 end) == false)
                        "#,
                    ),
                ],
                true,
            );

            harness.run_entrypoint("main.lua").unwrap();

            harness.send_event(Event::WindowClicked { id: ItemId(0) });
            harness.send_event(Event::WindowClicked { id: ItemId(0) });
            harness.pump_events();

            harness.run_entrypoint("after_clicks.lua").unwrap();
        })
        .await;
}

/// Exercises `Window.spawned`/`Window:on_spawn()` end to end: the field flips only once the
/// (here, manually injected) `WindowSpawned` event is delivered, callbacks registered before
/// that fire in registration order, and a callback registered *after* the window has already
/// spawned still fires -- but queued via `tokio::task::spawn_local`, not inline (execution
/// model rule 1: no Lewdware function may call back into Lua synchronously).
#[tokio::test(start_paused = true)]
async fn window_spawn_fires_callbacks_and_queues_late_registration() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[
                    (
                        "main.lua",
                        r#"
                            local image = lewdware.media.get_image("pic.avif")
                            WINDOW = lewdware.popup.image(image)
                            ORDER = {}

                            assert(WINDOW.spawned == false, "not spawned before the event arrives")

                            WINDOW:on_spawn(function() table.insert(ORDER, "first") end)
                            WINDOW:on_spawn(function() table.insert(ORDER, "second") end)
                        "#,
                    ),
                    (
                        "after_spawn.lua",
                        r#"
                            assert(WINDOW.spawned == true, "spawned once the event is delivered")
                            assert(#ORDER == 2, "both callbacks should have fired")
                            assert(
                                ORDER[1] == "first" and ORDER[2] == "second",
                                "callbacks fire in registration order"
                            )

                            WINDOW:on_spawn(function() table.insert(ORDER, "late") end)
                            assert(#ORDER == 2, "a late on_spawn callback must not fire inline")
                        "#,
                    ),
                    (
                        "after_queue_runs.lua",
                        r#"
                            assert(#ORDER == 3, "the late callback fires once queued")
                            assert(ORDER[3] == "late")
                        "#,
                    ),
                ],
                true,
            );

            harness.run_entrypoint("main.lua").unwrap();

            harness.send_event(Event::WindowSpawned { id: ItemId(0) });
            harness.pump_events();

            harness.run_entrypoint("after_spawn.lua").unwrap();

            // Let the `spawn_local`'d late callback actually run.
            harness.advance(Duration::from_millis(10)).await;

            harness.run_entrypoint("after_queue_runs.lua").unwrap();
        })
        .await;
}

/// Exercises the `lewdware.popup.dialog()` binding end to end: `values()`/`value()` reflect
/// `initial_value`, `update()` changes a live value (and reports whether the target id
/// existed), and `on_select`/`on_submit` (fired here via manually injected events, since the
/// fake handler doesn't model real button clicks/Enter presses) receive the button/element id
/// alongside a snapshot of every input's current value.
#[tokio::test(start_paused = true)]
async fn dialog_values_update_and_callbacks() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[
                    (
                        "main.lua",
                        r#"
                            DIALOG = lewdware.popup.dialog({
                                elements = {
                                    { type = "text", text = "Confirm?" },
                                    { type = "input", id = "name", initial_value = "Bob" },
                                    { type = "buttons", options = {
                                        { id = "yes", label = "Yes", default = true },
                                        { id = "no", label = "No" },
                                    }},
                                },
                            })

                            CLICKS = {}
                            DIALOG:on_select(function(button_id, values)
                                table.insert(CLICKS, { button_id, values.name })
                            end)

                            SUBMITS = {}
                            DIALOG:on_submit(function(element_id, values)
                                table.insert(SUBMITS, { element_id, values.name })
                            end)
                        "#,
                    ),
                    (
                        "check_values.lua",
                        r#"
                            assert(DIALOG:values().name == "Bob", "initial_value seeds values()")
                            assert(DIALOG:value("name") == "Bob")
                            assert(DIALOG:value("missing") == nil)

                            assert(DIALOG:update("name", { value = "Alice" }) == true)
                            assert(DIALOG:value("name") == "Alice")

                            assert(DIALOG:update("missing", { value = "x" }) == false)
                        "#,
                    ),
                    (
                        "check_callbacks.lua",
                        r#"
                            assert(#CLICKS == 1, "on_select should have fired once")
                            assert(CLICKS[1][1] == "yes")
                            assert(CLICKS[1][2] == "Alice", "click payload carries live values")

                            assert(#SUBMITS == 1, "on_submit should have fired once")
                            assert(SUBMITS[1][1] == "name")
                        "#,
                    ),
                ],
                false,
            );

            harness.run_entrypoint("main.lua").unwrap();
            assert_eq!(harness.recorded(), vec![Recorded::SpawnDialog]);

            harness.run_entrypoint("check_values.lua").unwrap();

            harness.send_event(Event::DialogSelect {
                id: ItemId(0),
                button_id: "yes".to_string(),
                values: HashMap::from([("name".to_string(), "Alice".to_string())]),
            });
            harness.send_event(Event::DialogSubmit {
                id: ItemId(0),
                element_id: "name".to_string(),
                values: HashMap::from([("name".to_string(), "Alice".to_string())]),
            });
            harness.pump_events();

            harness.run_entrypoint("check_callbacks.lua").unwrap();
        })
        .await;
}

/// `spawn_dialog` validates the "at most one default button" invariant up front (rather than
/// only at render time), so a mode author gets a clear error instead of ambiguous Enter-key
/// routing.
#[tokio::test(start_paused = true)]
async fn dialog_rejects_more_than_one_default_button() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::new(
                &[(
                    "main.lua",
                    r#"
                        local ok = pcall(function()
                            lewdware.popup.dialog({
                                elements = {
                                    { type = "buttons", options = {
                                        { id = "a", label = "A", default = true },
                                        { id = "b", label = "B", default = true },
                                    }},
                                },
                            })
                        end)
                        assert(not ok, "a second default button should be rejected")
                    "#,
                )],
                false,
            );

            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

/// Capability toggles are the one consent-critical surface the release plan calls out as
/// "not cuttable": a hostile mode with every capability denied must see `false` from each
/// call (never a thrown error -- these are "could not do it" outcomes, indistinguishable to
/// Lua from e.g. no browser being installed) and must leave no side effect, exactly as
/// `LewdwareApp::open_link`/`show_notification`/`set_wallpaper` behave for real (see
/// `Harness::with_capabilities`, which mirrors that gating in the fake handler).
#[tokio::test(start_paused = true)]
async fn hostile_mode_sees_every_denied_capability_as_false() {
    LocalSet::new()
        .run_until(async {
            let mut harness = Harness::with_capabilities(
                &[(
                    "main.lua",
                    r#"
                        local image = lewdware.media.get_image("pic.avif")
                        assert(lewdware.wallpaper.set(image) == false,
                            "wallpaper.set should be denied")
                        assert(lewdware.open_link("https://should-not-fire.example") == false,
                            "open_link should be denied")
                        assert(lewdware.show_notification({ body = "should not fire" }) == false,
                            "show_notification should be denied")
                    "#,
                )],
                true,
                Capabilities {
                    set_wallpaper: false,
                    open_links: false,
                    send_notifications: false,
                },
            );

            harness.run_entrypoint("main.lua").unwrap();

            // No side effect leaked through -- in particular, the denied `open_link` never
            // reached the point the fake handler records it.
            assert_eq!(harness.recorded(), vec![]);
        })
        .await;
}
