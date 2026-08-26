use super::{pack, popups_of, stage_ids, timeline_pack, tx_on};
use crate::behaviour::editor::media::edit_popup_entries;
use crate::behaviour::editor::{
    MediaSlots, audio_attributes, media_slots, popup_attributes, remove_stage, set_audio_volume,
    set_experience_enabled, set_media_slot, set_popup_caption, set_popup_scale, set_popup_weight,
    set_stage_audio_random, slot_value,
};
use crate::behaviour::schema::{AudioMedia, Behaviour, Content, MediaSlot};
use crate::behaviour::storage::{self, read_one_stage};

#[test]
fn a_file_with_no_opinion_has_no_entry() {
    let conn = pack();
    assert_eq!(popup_attributes(&conn, &[1, 2, 3]).unwrap(), []);
}

#[test]
fn setting_one_field_leaves_the_others_unset() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(set_popup_scale(tx, &[1], Some(2.0)).unwrap());
    });
    let entry = popups_of(&conn, 1).unwrap();
    assert_eq!(entry.scale, Some(2.0));
    assert_eq!(entry.weight, None);
    assert_eq!(entry.caption, None);
}

/// The distinction the double option exists for: an omitted field is left alone, a null one is
/// cleared. Collapsing them would make "set the scale" silently wipe the caption.
#[test]
fn an_omitted_field_is_left_alone_and_a_null_one_is_cleared() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_popup_scale(tx, &[1], Some(2.0)).unwrap();
        set_popup_caption(tx, &[1], Some("hello")).unwrap();
        // Setting the scale again says nothing about the caption.
        set_popup_scale(tx, &[1], Some(3.0)).unwrap();
    });
    let entry = popups_of(&conn, 1).unwrap();
    assert_eq!(entry.scale, Some(3.0));
    assert_eq!(entry.caption.as_deref(), Some("hello"));

    tx_on(&mut conn, |tx| {
        set_popup_caption(tx, &[1], None).unwrap();
    });
    let entry = popups_of(&conn, 1).unwrap();
    assert_eq!(entry.caption, None);
    assert_eq!(entry.scale, Some(3.0));
}

/// Clearing the last thing an entry said removes the entry, rather than leaving a row that means
/// nothing — the same answer a whole-document write reaches.
#[test]
fn clearing_the_last_field_drops_the_entry() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_popup_scale(tx, &[1], Some(2.0)).unwrap();
        set_popup_scale(tx, &[1], None).unwrap();
    });
    assert_eq!(popups_of(&conn, 1), None);
}

/// The inspector edits a selection, so one command has to reach every file in it.
#[test]
fn a_change_applies_across_a_selection() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(set_popup_weight(tx, &[1, 2], Some(5.0)).unwrap());
    });
    assert_eq!(popups_of(&conn, 1).unwrap().weight, Some(5.0));
    assert_eq!(popups_of(&conn, 2).unwrap().weight, Some(5.0));
}

#[test]
fn an_attribute_change_that_changes_nothing_reports_no_change() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_popup_scale(tx, &[1], Some(2.0)).unwrap();
        assert!(!set_popup_scale(tx, &[1], Some(2.0)).unwrap());
    });
}

/// Clearing a field on a file that never had an entry must not create one just to hold nothing.
#[test]
fn clearing_a_field_on_a_file_with_no_entry_writes_nothing() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(!set_popup_scale(tx, &[1], None).unwrap());
    });
    assert_eq!(popups_of(&conn, 1), None);
}

/// An empty caption says nothing, so it clears the field rather than storing "" — otherwise the
/// entry survives holding a caption the author deleted.
#[test]
fn an_empty_caption_clears_rather_than_stores() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_popup_caption(tx, &[1], Some("hello")).unwrap();
        set_popup_caption(tx, &[1], Some("")).unwrap();
    });
    assert_eq!(popups_of(&conn, 1), None);
}

#[test]
fn explicit_sound_pairings_round_trip() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        edit_popup_entries(tx, &[1], |entry| entry.audio = vec![3]).unwrap();
    });
    assert_eq!(popups_of(&conn, 1).unwrap().audio, [3]);

    // A set has no separate null: emptying the list is how a pairing is removed, and that leaves
    // the entry saying nothing.
    tx_on(&mut conn, |tx| {
        edit_popup_entries(tx, &[1], |entry| entry.audio = vec![]).unwrap();
    });
    assert_eq!(popups_of(&conn, 1), None);
}

#[test]
fn audio_attributes_follow_the_same_rules() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(set_audio_volume(tx, &[3], Some(0.5)).unwrap());
    });
    assert_eq!(
        audio_attributes(&conn, &[3]).unwrap(),
        [(3, AudioMedia { volume: Some(0.5) })]
    );

    tx_on(&mut conn, |tx| {
        assert!(set_audio_volume(tx, &[3], None).unwrap());
    });
    assert_eq!(audio_attributes(&conn, &[3]).unwrap(), []);
}

#[test]
fn the_pack_media_slots_round_trip() {
    let mut conn = pack();
    let behaviour = Behaviour {
        content: Content {
            wallpaper: Some(1),
            splash: Some(2),
            ..Behaviour::new().content
        },
        ..Behaviour::new()
    };
    tx_on(&mut conn, |tx| {
        storage::write(tx, &behaviour).unwrap();
    });
    assert_eq!(
        media_slots(&conn).unwrap(),
        MediaSlots {
            wallpaper: Some(1),
            splash: Some(2)
        }
    );
}

// ── Slots ────────────────────────────────────────────────────────────────────

#[test]
fn a_pack_slot_reads_back_what_was_written() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(set_media_slot(tx, &MediaSlot::Wallpaper, Some(1)).unwrap());
    });
    assert_eq!(slot_value(&conn, &MediaSlot::Wallpaper).unwrap(), Some(1));
    assert_eq!(slot_value(&conn, &MediaSlot::Splash).unwrap(), None);
    assert_eq!(media_slots(&conn).unwrap().wallpaper, Some(1));
}

#[test]
fn emptying_a_slot_clears_it() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_media_slot(tx, &MediaSlot::Wallpaper, Some(1)).unwrap();
        assert!(set_media_slot(tx, &MediaSlot::Wallpaper, None).unwrap());
    });
    assert_eq!(slot_value(&conn, &MediaSlot::Wallpaper).unwrap(), None);
}

#[test]
fn setting_a_slot_to_what_it_holds_reports_no_change() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_media_slot(tx, &MediaSlot::Wallpaper, Some(1)).unwrap();
        assert!(!set_media_slot(tx, &MediaSlot::Wallpaper, Some(1)).unwrap());
    });
}

#[test]
fn a_stages_optional_rows_are_created_when_a_slot_first_fills_one() {
    let mut conn = timeline_pack(&["a", "b"]);
    let slot = MediaSlot::StageEntrySplash {
        stage: "a".to_string(),
    };
    assert_eq!(slot_value(&conn, &slot).unwrap(), None);
    tx_on(&mut conn, |tx| {
        assert!(set_media_slot(tx, &slot, Some(1)).unwrap());
    });
    assert_eq!(slot_value(&conn, &slot).unwrap(), Some(1));

    let prompt = MediaSlot::StagePromptSound {
        stage: "a".to_string(),
    };
    tx_on(&mut conn, |tx| {
        assert!(set_media_slot(tx, &prompt, Some(3)).unwrap());
    });
    assert_eq!(slot_value(&conn, &prompt).unwrap(), Some(3));
}

/// Naming a track and "pick one at random on entry" are alternatives, so choosing a track has to
/// switch the random flag off or the stage claims both.
#[test]
fn naming_a_stage_track_switches_off_random_selection() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        set_stage_audio_random(tx, "a", true).unwrap();
        set_media_slot(
            tx,
            &MediaSlot::StageAudio {
                stage: "a".to_string(),
            },
            Some(3),
        )
        .unwrap();
    });
    let stage = read_one_stage(&conn, "a").unwrap().unwrap();
    assert_eq!(stage.content.audio, Some(3));
    assert!(!stage.content.audio_random);
}

#[test]
fn a_slot_on_a_removed_stage_is_refused() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        remove_stage(tx, "b").unwrap();
    });
    let tx = conn.transaction().unwrap();
    assert!(
        set_media_slot(
            &tx,
            &MediaSlot::StageWallpaper {
                stage: "b".to_string()
            },
            Some(1)
        )
        .is_err()
    );
}

/// The hazard this section removes: a slot written while the timeline is switched off used to go
/// through a whole-document round trip, and a document read in that state has no stages in it.
#[test]
fn filling_a_slot_while_the_timeline_is_off_keeps_the_stages() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        set_experience_enabled(tx, false).unwrap();
        set_media_slot(tx, &MediaSlot::Wallpaper, Some(1)).unwrap();
    });
    assert_eq!(stage_ids(&conn), ["a", "b"]);
    assert_eq!(slot_value(&conn, &MediaSlot::Wallpaper).unwrap(), Some(1));
}
