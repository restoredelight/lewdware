mod db;
pub mod editor;
mod resolver;
mod schema;

pub use resolver::{
    CONTENT_GROUP_KEY_PREFIX, EffectiveSchema, effective_config, effective_options,
};
pub use schema::{
    AudioMedia, Behaviour, Caption, Content, ContentGroup, ContentSelection, CountScope, Easing,
    EndStrategy, EventCountCondition, EventKind, EventSchedule, Events, Interval, MediaSlot,
    Mitosis, MonitorPreference, Movement, Notification, PopupMedia, Prompt, SpawnRegion, Stage,
    StageEnd, StageEntry, StagePrompt, Timeline, Transition, TransitionCategory, ValidationIssue,
    ValidationSeverity, WebLink,
};
