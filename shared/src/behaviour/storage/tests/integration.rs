use super::*;

#[test]
fn a_full_document_survives_the_round_trip() {
    let mut conn = pack();
    let mut first = stage("a");
    first.content.tags = Some(vec!["kinky".to_string(), "soft".to_string()]);
    first.content.wallpaper = Some(1);
    first.content.audio = Some(3);
    first.on_enter = StageEntry {
        splash: Some(1),
        sound: Some(3),
        popup_burst: Some(4),
        notification: Some("Enter".to_string()),
    };
    first.end = Some(StageEnd {
        duration_seconds: Some(300.0),
        event_count: Some(EventCountCondition {
            event: EventKind::Sound,
            count: 10,
            scope: CountScope::Session,
        }),
        strategy: EndStrategy::All,
    });
    first.movement = Some(Movement {
        minimum_speed: Some(50.0),
        maximum_speed: None,
    });
    first.mitosis = Some(Mitosis {
        chance: Some(0.5),
        count: Some(2),
    });
    first.prompt.timeout_multiplier = 1.5;
    first.prompt.popup_burst = Some(5);
    first.events.popup = Some(EventSchedule {
        interval: Interval::Fixed { seconds: 30.0 },
        initial_delay_seconds: Some(5.0),
        max_concurrent: Some(3),
    });
    first.events.web = Some(EventSchedule {
        interval: Interval::Random {
            minimum_seconds: 10.0,
            maximum_seconds: 20.0,
        },
        initial_delay_seconds: None,
        max_concurrent: None,
    });
    first.events.sound = Some(EventSchedule {
        interval: Interval::Fixed { seconds: 7.5 },
        initial_delay_seconds: Some(1.0),
        max_concurrent: None,
    });

    let behaviour = Behaviour {
        content: crate::behaviour::Content {
            popups: BTreeMap::from([
                (
                    1,
                    PopupMedia {
                        weight: Some(2.5),
                        scale: Some(1.5),
                        region: Some(SpawnRegion {
                            x: 0.5,
                            y: 0.25,
                            width: 0.5,
                            height: 0.5,
                        }),
                        monitor: Some(MonitorPreference::Primary),
                        caption: Some("Look.".to_string()),
                        video_loop: None,
                        video_audio: None,
                        video_volume: None,
                        audio: vec![3],
                    },
                ),
                (
                    2,
                    PopupMedia {
                        video_loop: Some(false),
                        video_audio: Some(true),
                        video_volume: Some(0.6),
                        ..PopupMedia::default()
                    },
                ),
            ]),
            audio: BTreeMap::from([(3, AudioMedia { volume: Some(0.4) })]),
            content_groups: vec![ContentGroup {
                id: "kinky".to_string(),
                label: "Kinky".to_string(),
                description: Some("Kinky-tagged content".to_string()),
                tags: vec!["kinky".to_string(), "soft".to_string()],
                enabled_by_default: false,
            }],
            captions: vec![
                TextItem {
                    text: "Obey.".to_string(),
                    tags: vec!["kinky".to_string()],
                    timeout_seconds: None,
                    summary: None,
                },
                TextItem {
                    text: "Good.".to_string(),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                },
            ],
            prompts: vec![TextItem {
                text: "Type this.".to_string(),
                tags: vec![],
                timeout_seconds: Some(30.0),
                summary: None,
            }],
            notifications: vec![TextItem {
                text: "Ping.".to_string(),
                tags: vec!["soft".to_string()],
                timeout_seconds: None,
                summary: Some("Attention".to_string()),
            }],
            web_links: vec![WebLink {
                url: "https://duckduckgo.com/?q=".to_string(),
                args: vec!["one".to_string(), "two".to_string()],
                tags: vec!["kinky".to_string()],
            }],
            wallpaper: Some(1),
            splash: Some(2),
        },
        experience: Some(Experience {
            timeline: Timeline {
                stages: vec![first, stage("b")],
                transitions: vec![Transition {
                    id: "t1".to_string(),
                    from_stage: "a".to_string(),
                    to_stage: "b".to_string(),
                    duration_seconds: 12.5,
                    easing: Easing::EaseInOut,
                    affected: vec![
                        TransitionCategory::MitosisChance,
                        TransitionCategory::PopupInterval,
                        TransitionCategory::SoundInterval,
                        TransitionCategory::Crossfade,
                    ],
                }],
            },
            label: Some("Corruption".to_string()),
        }),
    };

    store(&mut conn, &behaviour);
    assert_eq!(read(&conn).unwrap(), behaviour);
}
