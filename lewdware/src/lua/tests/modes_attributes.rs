use std::time::Duration;

use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

/// Captures the options the mode hands `popup.image`, so a test can assert what a per-item
/// attribute actually produced. The recorded `SpawnImage` carries only the media id, and
/// these are options rather than requests -- the engine resolves them into a size the fake
/// handler never sees.
const POPUP_OPTS_CAPTURE_WRAP: &str = r#"
    CAPTURED_SCALE = -1
    CAPTURED_REGION = ""
    CAPTURED_MONITOR = -1
    local real = lewdware.popup.image
    lewdware.popup.image = function(item, opts)
        CAPTURED_SCALE = opts.scale or -1
        CAPTURED_REGION = opts.region and string.format(
            "%g,%g,%g,%g",
            opts.region.x, opts.region.y, opts.region.width, opts.region.height
        ) or ""
        CAPTURED_MONITOR = opts.monitor and opts.monitor.id or -1
        return real(item, opts)
    end
"#;

/// Per-item popup attributes reach the spawn call, and only where the author set them.
#[tokio::test(start_paused = true)]
async fn per_item_popup_attributes_are_applied_to_the_popup() {
    LocalSet::new()
        .run_until(async {
            let media: &[(&str, &[&str])] = &[("popup.avif", &[])];
            let mut content = Content::default();
            content.popups.insert(
                1,
                shared::behaviour::PopupMedia {
                    scale: Some(2.5),
                    // A zero-size region at the far corner: the placement an anchor used to
                    // name, now expressed as the degenerate case of the rectangle.
                    region: Some(shared::behaviour::SpawnRegion {
                        x: 1.0,
                        y: 1.0,
                        width: 0.0,
                        height: 0.0,
                    }),
                    caption: Some("Pinned.".to_string()),
                    ..Default::default()
                },
            );

            let owned = wrapped_default_mode_sources(POPUP_OPTS_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);
            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture_with_data(media, None),
                content,
                base_default_mode_config(),
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1100)).await;

            assert_eq!(harness.eval_number("CAPTURED_SCALE"), 2.5);
            // The region reaches the engine as-is rather than being turned into coordinates
            // here: only the engine knows the size it settled on after its own caps, and
            // "inside this rectangle" is a statement about the whole window.
            assert_eq!(harness.eval_string("CAPTURED_REGION"), "1,1,0,0");
            // Said nothing about a monitor, so the mode names none and the engine picks.
            assert_eq!(harness.eval_number("CAPTURED_MONITOR"), -1.0);
            // The caption goes through `set_title`, which the harness records directly --
            // the window is userdata, so a Lua wrapper cannot intercept the method.
            assert!(
                harness.recorded().iter().any(|recorded| matches!(
                    recorded,
                    Recorded::SetTitle { title: Some(title), .. } if title == "Pinned."
                )),
                "a caption pinned to the file wins over the tag-matched pool, got {:?}",
                harness.recorded(),
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn per_item_weights_control_the_default_modes_popup_draw() {
    LocalSet::new()
        .run_until(async {
            let media: &[(&str, &[&str])] = &[("ordinary.avif", &[]), ("hero.avif", &[])];
            let mut content = Content::default();
            content.popups.insert(
                1,
                shared::behaviour::PopupMedia {
                    weight: Some(1e-300),
                    ..Default::default()
                },
            );
            content.popups.insert(
                2,
                shared::behaviour::PopupMedia {
                    weight: Some(1e300),
                    ..Default::default()
                },
            );

            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture_with_data(media, None),
                content,
                base_default_mode_config(),
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1100)).await;

            assert!(
                harness
                    .recorded()
                    .iter()
                    .any(|recorded| matches!(recorded, Recorded::SpawnImage { media_id: 2 }))
            );
            assert!(
                !harness
                    .recorded()
                    .iter()
                    .any(|recorded| matches!(recorded, Recorded::SpawnImage { media_id: 1 }))
            );
        })
        .await;
}

/// A file asking for the primary monitor gets it named on the spawn call. The mode has to
/// resolve this itself -- `PopupOpts.monitor` is a `Monitor`, not a preference -- which is
/// also what lets it fall back silently when the user has switched that screen off.
#[tokio::test(start_paused = true)]
async fn a_file_can_ask_for_the_primary_monitor() {
    LocalSet::new()
        .run_until(async {
            let media: &[(&str, &[&str])] = &[("popup.avif", &[])];
            let mut content = Content::default();
            content.popups.insert(
                1,
                shared::behaviour::PopupMedia {
                    monitor: Some(shared::behaviour::MonitorPreference::Primary),
                    ..Default::default()
                },
            );

            let owned = wrapped_default_mode_sources(POPUP_OPTS_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);
            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture_with_data(media, None),
                content,
                base_default_mode_config(),
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1100)).await;

            assert_eq!(harness.eval_number("CAPTURED_MONITOR"), 0.0);
            // And it did not also acquire a region: the two are independent fields, and a
            // monitor preference says nothing about where on that monitor the popup goes.
            assert_eq!(harness.eval_string("CAPTURED_REGION"), "");
        })
        .await;
}

/// The other half: a file the author said nothing about is spawned exactly as before. An
/// attribute defaulted here rather than left absent would silently restyle every pack.
#[tokio::test(start_paused = true)]
async fn a_file_with_no_attributes_spawns_unchanged() {
    LocalSet::new()
        .run_until(async {
            let media: &[(&str, &[&str])] = &[("popup.avif", &[])];
            let owned = wrapped_default_mode_sources(POPUP_OPTS_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);
            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture_with_data(media, None),
                Content::default(),
                base_default_mode_config(),
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1100)).await;

            assert_eq!(harness.eval_number("CAPTURED_SCALE"), -1.0, "no scale set");
            assert_eq!(
                harness.eval_string("CAPTURED_REGION"),
                "",
                "placement left to the engine, not narrowed to a full-screen region"
            );
            assert_eq!(
                harness.eval_number("CAPTURED_MONITOR"),
                -1.0,
                "monitor left to the engine"
            );
            assert!(
                !harness
                    .recorded()
                    .iter()
                    .any(|recorded| matches!(recorded, Recorded::SetTitle { .. })),
                "and no caption, since the pack has neither a pinned one nor a pool",
            );
        })
        .await;
}

/// An explicit pairing is the author naming the sound for this popup, so it replaces tag
/// matching rather than being pooled with it -- otherwise naming one sound would leave the
/// popup mostly still playing the others.
#[tokio::test(start_paused = true)]
async fn an_explicit_pairing_replaces_tag_matching() {
    LocalSet::new()
        .run_until(async {
            let media: &[(&str, &[&str])] = &[
                ("popup.avif", &[]),
                ("tagged.ogg", &[shared::tags::POPUP_AUDIO_TAG]),
                ("paired.ogg", &[shared::tags::POPUP_AUDIO_TAG]),
            ];
            let mut content = Content::default();
            content.popups.insert(
                1,
                shared::behaviour::PopupMedia {
                    audio: vec![3],
                    ..Default::default()
                },
            );
            // Half volume on the paired file, to check the author's level composes with the
            // user's rather than replacing it.
            content
                .audio
                .insert(3, shared::behaviour::AudioMedia { volume: Some(0.5) });

            let mut config = base_default_mode_config();
            config.insert(
                "popup_audio_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            config.insert("popup_volume".to_string(), OptionValue::Number(50.0));

            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture_with_data(media, None),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1100)).await;
            harness.send_event(Event::WindowSpawned { id: ItemId(0) });
            harness.pump_events();

            let recorded = harness.recorded();
            let sounds: Vec<&Recorded> = recorded
                .iter()
                .filter(|recorded| matches!(recorded, Recorded::SpawnAudio { .. }))
                .collect();
            assert_eq!(sounds.len(), 1);
            match sounds[0] {
                Recorded::SpawnAudio { media_id, volume } => {
                    assert_eq!(*media_id, 3, "the paired sound, never the tag-matched one");
                    assert!(
                        (*volume - 0.25).abs() < f32::EPSILON,
                        "50% user x 0.5 author, got {volume}",
                    );
                }
                other => panic!("expected a sound, got {other:?}"),
            }
        })
        .await;
}
