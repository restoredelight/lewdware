use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicBool},
};

use crate::{
    audio::AudioPlayer,
    error::{LewdwareError, Result},
    lua::{DialogElement, FadeOpts, MoveOpts, TextStyle},
    media::{ImageData, MediaRequirement, RequirementId, ResolvedMedia},
    video::VideoDecoder,
    window::WindowOpts,
};

pub(super) struct PendingItem {
    pub opts: PendingItemOpts,
    pub requirements: Vec<Requirement>,
}

impl PendingItem {
    pub fn resolve(&mut self, id: RequirementId, media: ResolvedMedia) -> Result<()> {
        let requirement = self
            .requirements
            .iter_mut()
            .find(|requirement| requirement.id == id)
            .ok_or(LewdwareError::Internal("Pending requirement not found"))?;

        let matching_type = matches!(
            (&requirement.state, &media),
            (
                RequirementState::Pending(MediaRequirement::Image { .. }),
                ResolvedMedia::Image(_)
            ) | (
                RequirementState::Pending(MediaRequirement::Video { .. }),
                ResolvedMedia::Video(_)
            ) | (
                RequirementState::Pending(MediaRequirement::Audio { .. }),
                ResolvedMedia::Audio(_)
            )
        );
        if !matching_type {
            return Err(LewdwareError::Internal(
                "Media result did not match its pending requirement",
            ));
        }

        requirement.state = RequirementState::Resolved(media);
        Ok(())
    }

    pub fn is_resolved(&self) -> bool {
        self.requirements
            .iter()
            .all(|requirement| matches!(requirement.state, RequirementState::Resolved(_)))
    }

    pub fn into_resolved(self) -> Result<(PendingItemOpts, ResolvedRequirements)> {
        let mut resolved = HashMap::with_capacity(self.requirements.len());
        for requirement in self.requirements {
            let RequirementState::Resolved(media) = requirement.state else {
                return Err(LewdwareError::Internal("Requirement is still pending"));
            };
            resolved.insert(requirement.id, media);
        }
        Ok((self.opts, ResolvedRequirements(resolved)))
    }
}

pub(super) struct Requirement {
    pub id: RequirementId,
    pub state: RequirementState,
}

pub(super) enum RequirementState {
    Pending(MediaRequirement),
    Resolved(ResolvedMedia),
}

#[allow(clippy::large_enum_variant)]
pub(super) enum PendingItemOpts {
    Image {
        window: PendingWindowOpts,
        image: RequirementId,
    },
    Video {
        window: PendingWindowOpts,
        video: RequirementId,
        loop_video: Arc<AtomicBool>,
        paused: bool,
        volume: f32,
    },
    Dialog {
        window: PendingWindowOpts,
        elements: Vec<DialogElement<RequirementId>>,
    },
    Text {
        window: PendingWindowOpts,
        text: String,
        style: TextStyle,
    },
    Audio {
        audio: RequirementId,
        paused: bool,
        volume: f32,
    },
}

impl PendingItemOpts {
    pub fn is_window(&self) -> bool {
        !matches!(self, Self::Audio { .. })
    }

    pub fn window_mut(&mut self) -> Option<&mut PendingWindowOpts> {
        match self {
            Self::Image { window, .. }
            | Self::Video { window, .. }
            | Self::Dialog { window, .. }
            | Self::Text { window, .. } => Some(window),
            Self::Audio { .. } => None,
        }
    }
}

pub(super) struct PendingWindowOpts {
    pub window: WindowOpts,
    pub opacity: f32,
    pub title: Option<String>,
    pub pending_move: Option<(u64, MoveOpts)>,
    pub pending_fade: Option<(u64, FadeOpts)>,
}

impl PendingWindowOpts {
    pub fn new(window: WindowOpts) -> Self {
        Self {
            opacity: window.popup_opts.opacity,
            title: window.popup_opts.title.clone(),
            pending_move: None,
            pending_fade: None,
            window,
        }
    }
}

pub(super) struct ResolvedRequirements(HashMap<RequirementId, ResolvedMedia>);

impl ResolvedRequirements {
    pub fn take_image(&mut self, id: RequirementId) -> Result<ImageData> {
        match self.0.remove(&id) {
            Some(ResolvedMedia::Image(image)) => Ok(image),
            Some(_) => Err(LewdwareError::Internal("Requirement was not an image")),
            None => Err(LewdwareError::Internal("Resolved requirement not found")),
        }
    }

    pub fn take_video(&mut self, id: RequirementId) -> Result<VideoDecoder> {
        match self.0.remove(&id) {
            Some(ResolvedMedia::Video(video)) => Ok(video),
            Some(_) => Err(LewdwareError::Internal("Requirement was not a video")),
            None => Err(LewdwareError::Internal("Resolved requirement not found")),
        }
    }

    pub fn take_audio(&mut self, id: RequirementId) -> Result<AudioPlayer> {
        match self.0.remove(&id) {
            Some(ResolvedMedia::Audio(audio)) => Ok(audio),
            Some(_) => Err(LewdwareError::Internal("Requirement was not audio")),
            None => Err(LewdwareError::Internal("Resolved requirement not found")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn image_requirement(id: u64, media_id: u64) -> Requirement {
        Requirement {
            id: RequirementId(id),
            state: RequirementState::Pending(MediaRequirement::Image {
                media_id,
                width: 1,
                height: 1,
            }),
        }
    }

    fn pending_item(requirements: Vec<Requirement>) -> PendingItem {
        PendingItem {
            opts: PendingItemOpts::Audio {
                paused: false,
                volume: 1.0,
                audio: RequirementId(99),
            },
            requirements,
        }
    }

    #[test]
    fn requirements_resolve_by_id_out_of_order() {
        let first = RequirementId(10);
        let second = RequirementId(20);
        let mut item = pending_item(vec![
            image_requirement(first.0, 100),
            image_requirement(second.0, 200),
        ]);

        item.resolve(
            second,
            ResolvedMedia::Image(ImageData::from_pixel(1, 1, Rgba([2, 0, 0, 255]))),
        )
        .unwrap();
        assert!(!item.is_resolved());

        item.resolve(
            first,
            ResolvedMedia::Image(ImageData::from_pixel(1, 1, Rgba([1, 0, 0, 255]))),
        )
        .unwrap();
        assert!(item.is_resolved());

        let (_, mut resolved) = item.into_resolved().unwrap();
        assert_eq!(resolved.take_image(first).unwrap().get_pixel(0, 0).0[0], 1);
        assert_eq!(resolved.take_image(second).unwrap().get_pixel(0, 0).0[0], 2);
    }

    #[test]
    fn item_without_requirements_is_immediately_resolved() {
        let item = pending_item(Vec::new());
        assert!(item.is_resolved());
        assert!(item.into_resolved().unwrap().1.0.is_empty());
    }

    #[test]
    fn unknown_or_already_resolved_requirement_is_rejected() {
        let id = RequirementId(10);
        let mut item = pending_item(vec![image_requirement(id.0, 100)]);
        let image = || ResolvedMedia::Image(ImageData::from_pixel(1, 1, Rgba([1, 0, 0, 255])));

        assert!(item.resolve(RequirementId(11), image()).is_err());
        item.resolve(id, image()).unwrap();
        assert!(item.resolve(id, image()).is_err());
    }

    #[test]
    fn resolved_media_must_match_the_pending_requirement_type() {
        let id = RequirementId(10);
        let mut item = pending_item(vec![Requirement {
            id,
            state: RequirementState::Pending(MediaRequirement::Video {
                media_id: 100,
                loop_video: Arc::new(AtomicBool::new(false)),
                play_audio: false,
                volume: 1.0,
            }),
        }]);

        let image = ResolvedMedia::Image(ImageData::from_pixel(1, 1, Rgba([1, 0, 0, 255])));
        assert!(item.resolve(id, image).is_err());
        assert!(!item.is_resolved());
    }
}
