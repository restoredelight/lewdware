use std::{
    error::Error,
    fmt::Display,
    fs,
    io::{self, ErrorKind},
    mem,
    path::{Path, PathBuf},
};

use crate::{
    config::{MediaOpts, NotificationOpts, OneOrMore, PackOpts},
    target::{Either, Empty, Item, Items, Target},
};
use glob::{Pattern, PatternError};
use merge::Merge;
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy)]
pub enum MediaCategory {
    Popup,
    Wallpaper,
}

#[derive(Debug)]
pub enum ConfigError {
    IoError(io::Error),
    JsonError {
        file: PathBuf,
        string: String,
        error: json5::Error,
    },
    PackOptsError {
        file: PathBuf,
        error: PackOptsError,
    },
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::IoError(error) => {
                error.fmt(f)?;
            }
            ConfigError::JsonError {
                file,
                string,
                error,
            } => match error {
                json5::Error::Message { msg, location } => {
                    if let Some(location) = location
                        && let Some(line) = string.lines().nth(location.line - 1)
                    {
                        writeln!(f, "Error in {}: {}", file.display(), msg)?;
                        writeln!(f, "{}", line)?;
                        write!(f, "{}^", " ".repeat(location.column - 1))?;
                    }
                }
            },
            ConfigError::PackOptsError { file, error } => {
                writeln!(f, "Error in {}", file.display())?;
                error.fmt(f)?;
            }
        }

        Ok(())
    }
}

impl Error for ConfigError {}

impl From<io::Error> for ConfigError {
    fn from(value: io::Error) -> Self {
        ConfigError::IoError(value)
    }
}

/// Will search for `config.json`/`config.json5`
pub fn find_config(root: &Path) -> Result<Config, ConfigError> {
    let config_path = root.join("config.json");
    let config_path_json5 = root.join("config.json5");

    let (path, config): (_, PackOpts) = match fs::read_to_string(&config_path) {
        Ok(content) => (
            Some(&config_path),
            json5::from_str(&content).map_err(|err| ConfigError::JsonError {
                file: config_path.to_path_buf(),
                string: content,
                error: err,
            })?,
        ),
        Err(err) => match err.kind() {
            ErrorKind::NotFound => match fs::read_to_string(&config_path_json5) {
                Ok(content) => (
                    Some(&config_path_json5),
                    json5::from_str(&content).map_err(|err| ConfigError::JsonError {
                        file: config_path_json5.to_path_buf(),
                        string: content,
                        error: err,
                    })?,
                ),
                Err(err) => match err.kind() {
                    ErrorKind::NotFound => (None, Default::default()),
                    _ => {
                        return Err(err.into());
                    }
                },
            },
            _ => return Err(err.into()),
        },
    };

    let valid_tags = if let Some(path) = path {
        check_opts(&config).map_err(|err| ConfigError::PackOptsError {
            file: path.to_path_buf(),
            error: err,
        })?
    } else {
        vec![]
    };

    let mut nested_config = Vec::new();

    for file in WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            let name = e.file_name().to_str();

            path.is_file()
                && (name == Some("metadata.json") || name == Some("metadata.json5"))
                && path.parent() != Some(root)
        })
    {
        let content = fs::read_to_string(file.path())?;

        let config = json5::from_str(&content).map_err(|err| ConfigError::JsonError {
            file: config_path_json5.to_path_buf(),
            string: content,
            error: err,
        })?;

        check_media_opts(&valid_tags, &config).map_err(|err| ConfigError::PackOptsError {
            file: file.path().to_path_buf(),
            error: err,
        })?;

        nested_config.push((file.path().parent().unwrap().to_path_buf(), config));
    }

    Ok(Config::new(config, nested_config))
}

pub struct Config {
    pub root_config: PackOpts,
    nested_config: Vec<(PathBuf, MediaOpts)>,
}

impl Config {
    fn new(root_config: PackOpts, nested_config: Vec<(PathBuf, MediaOpts)>) -> Self {
        Self {
            root_config,
            nested_config,
        }
    }

    /// Resolves all popups, wallpapers, notifications, etc. that the config files describe. Does
    /// not actually look for any files, just figures out everything that's described in the
    /// config.
    pub fn resolve(&self) -> Resolved {
        Resolved {
            popups: self.get_popups(),
            wallpapers: self.get_wallpapers(),
            notifications: self.get_notifications(),
            links: self.get_links(),
            prompts: self.get_prompts(),
        }
    }

    /// Given a path to an image, video or audio file, figure out its tags, and its category (i.e.
    /// whether the file is intended to be used as a wallpaper).
    pub fn get_tags_and_category(
        &self,
        path: &Path,
        resolved: &Resolved,
    ) -> (Vec<String>, MediaCategory) {
        let mut tags = Vec::new();
        let mut category = MediaCategory::Popup;

        tags.extend(self.resolve_tags(path));

        for popup in &resolved.popups {
            if glob_matches(&popup.primary, path) {
                tags.extend(popup.tags.clone());
            }
        }

        for wallpaper in &resolved.wallpapers {
            if glob_matches(&wallpaper.primary, path) {
                tags.extend(wallpaper.tags.clone());

                category = MediaCategory::Wallpaper;
            }
        }

        (tags, category)
    }

    fn get_popups(&self) -> Vec<ResolvedTarget<String>> {
        self.get_targets(|media_opts| media_opts.popups.as_ref())
    }

    fn get_notifications(&self) -> Vec<ResolvedTarget<String, NotificationOpts>> {
        self.get_targets(|media_opts| media_opts.notifications.as_ref())
    }

    fn get_links(&self) -> Vec<ResolvedTarget<String>> {
        self.get_targets(|media_opts| media_opts.links.as_ref())
    }

    fn get_prompts(&self) -> Vec<ResolvedTarget<String>> {
        self.get_targets(|media_opts| media_opts.prompts.as_ref())
    }

    fn get_wallpapers(&self) -> Vec<ResolvedTarget<String>> {
        self.get_targets(|media_opts| media_opts.wallpaper.as_ref())
    }

    fn get_targets<Primary, PrimaryStruct, Opts, ExtraOpts>(
        &self,
        f: impl Fn(&MediaOpts) -> Option<&Target<Primary, PrimaryStruct, Opts, ExtraOpts>>,
    ) -> Vec<ResolvedTarget<Primary, Opts>>
    where
        Primary: Clone,
        Opts: Default + Merge + Clone,
        ExtraOpts: Clone,
        PrimaryStruct: Into<Primary> + Clone,
    {
        let mut result = Vec::new();

        if let Some(target) = f(&self.root_config.media) {
            let mut targets = self.get_targets_from_target(target);

            if let Some(default_tag) = &self.root_config.default_tag {
                for target in targets.iter_mut() {
                    target.tags.push(default_tag.clone());
                }
            }

            result.extend(targets);
        }

        for (path, config) in &self.nested_config {
            if let Some(target) = f(config) {
                let default_tags = self.resolve_tags(path);

                let mut targets = self.get_targets_from_target(target);

                for target in targets.iter_mut() {
                    target.tags.extend(default_tags.clone());

                    if let Some(default_tag) = &self.root_config.default_tag {
                        target.tags.push(default_tag.clone());
                    }
                }

                result.extend(targets);
            }
        }

        result
    }

    fn get_targets_from_target<Primary, PrimaryStruct, Opts, ExtraOpts>(
        &self,
        target: &Target<Primary, PrimaryStruct, Opts, ExtraOpts>,
    ) -> Vec<ResolvedTarget<Primary, Opts>>
    where
        Primary: Clone,
        Opts: Default + Merge + Clone,
        ExtraOpts: Clone,
        PrimaryStruct: Into<Primary> + Clone,
    {
        match target {
            Either::Left(items) => self.get_targets_from_items(items),
            Either::Right(config) => {
                let mut items = self.get_targets_from_items(&config.items);

                for item in items.iter_mut() {
                    let default = config.default.opts.clone();

                    let opts = mem::replace(&mut item.opts, default);
                    item.opts.merge(opts);

                    item.tags.extend(config.default.tags.clone());
                }

                items
            }
        }
    }

    fn get_targets_from_items<Primary, PrimaryStruct, Opts>(
        &self,
        items: &Items<Primary, PrimaryStruct, Opts>,
    ) -> Vec<ResolvedTarget<Primary, Opts>>
    where
        Opts: Default + Merge + Clone,
        Primary: Clone,
        PrimaryStruct: Into<Primary> + Clone,
    {
        match items {
            Items::Single(item) => vec![self.get_target_from_item(item)],
            Items::Multiple(items) => items
                .iter()
                .map(|item| self.get_target_from_item(item))
                .collect(),
        }
    }

    fn get_target_from_item<Primary, PrimaryStruct, Opts>(
        &self,
        item: &Item<Primary, PrimaryStruct, Opts>,
    ) -> ResolvedTarget<Primary, Opts>
    where
        Opts: Default + Merge + Clone,
        Primary: Clone,
        PrimaryStruct: Into<Primary> + Clone,
    {
        match item {
            Either::Left(primary) => ResolvedTarget {
                primary: primary.clone(),
                opts: Opts::default(),
                tags: Vec::new(),
            },
            Either::Right(full) => ResolvedTarget {
                primary: full.primary.clone().into(),
                opts: full.opts.clone(),
                tags: full.tags.clone(),
            },
        }
    }

    fn resolve_tags(&self, path: &Path) -> Vec<String> {
        let mut tags: Vec<String> = self.root_config.default_tag.iter().cloned().collect();

        for (tag, targets) in &self.root_config.tags {
            match targets {
                OneOrMore::One(x) => {
                    if glob_matches(x, path) {
                        tags.push(tag.clone());
                    }
                }
                OneOrMore::More(items) => {
                    for item in items {
                        if glob_matches(item, path) {
                            tags.push(tag.clone());
                            break;
                        }
                    }
                }
            }
        }

        tags
    }
}

#[derive(Debug)]
pub enum PackOptsError {
    TagError {
        location: String,
        tag: String,
        valid_tags: Vec<String>,
    },
    GlobError {
        location: String,
        glob: String,
        error: PatternError,
    },
}

impl Display for PackOptsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackOptsError::TagError {
                location,
                tag,
                valid_tags,
            } => {
                writeln!(f, "In `{}`, invalid tag {}", location, tag)?;

                write!(
                    f,
                    "Available tags are {}",
                    valid_tags
                        .iter()
                        .map(|x| format!("\"{}\"", x))
                        .collect::<Vec<_>>()
                        .join(", ")
                )?;
            }
            PackOptsError::GlobError {
                location,
                glob,
                error,
            } => {
                writeln!(f, "In `{}`, invalid glob:", location)?;
                writeln!(f, "{}", glob)?;
                write!(f, "{}^ {}", " ".repeat(error.pos), error.msg)?;
            }
        }

        Ok(())
    }
}

impl Error for PackOptsError {}

fn check_opts(opts: &PackOpts) -> Result<Vec<&str>, PackOptsError> {
    let valid_tags: Vec<&str> = opts.tags.keys().map(|x| x.as_str()).collect();

    if let Some(transition) = &opts.metadata.transition {
        for item in &transition.items {
            if let Some(tags) = &item.tags {
                for tag in tags {
                    check_tag("transition", tag, &valid_tags)?;
                }
            }
        }
    }

    check_media_opts(&valid_tags, &opts.media)?;

    for (tag, target) in &opts.tags {
        let location = format!("tags.{}", tag);
        match target {
            OneOrMore::One(item) => check_glob(&location, item)?,
            OneOrMore::More(items) => {
                for item in items {
                    check_glob(&location, item)?;
                }
            }
        }
    }

    if let Some(ignore) = &opts.ignore {
        match ignore {
            OneOrMore::One(item) => check_glob("ignore", item)?,
            OneOrMore::More(items) => {
                for item in items {
                    check_glob("ignore", item)?;
                }
            }
        }
    }

    Ok(valid_tags)
}

fn check_media_opts(valid_tags: &[&str], opts: &MediaOpts) -> Result<(), PackOptsError> {
    if let Some(popups) = &opts.popups {
        check_target(
            "popups",
            valid_tags,
            popups,
            Some(Box::new(|glob: String| check_glob("popups", &glob))),
        )?;
    }

    if let Some(notifications) = &opts.notifications {
        check_target("notifications", valid_tags, notifications, None)?;
    }

    if let Some(links) = &opts.links {
        check_target("links", valid_tags, links, None)?;
    }

    if let Some(prompts) = &opts.prompts {
        check_target("prompts", valid_tags, prompts, None)?;
    }

    if let Some(wallpaper) = &opts.wallpaper {
        check_target(
            "wallpaper",
            valid_tags,
            wallpaper,
            Some(Box::new(|glob: String| check_glob("wallpaper", &glob))),
        )?;
    }

    Ok(())
}

type OptionalCheck<X> = Option<Box<dyn Fn(X) -> Result<(), PackOptsError>>>;

fn check_target<Primary, PrimaryStruct, Opts, ExtraOpts>(
    location: &str,
    valid_tags: &[&str],
    target: &Target<Primary, PrimaryStruct, Opts, ExtraOpts>,
    check_primary: OptionalCheck<Primary>,
) -> Result<(), PackOptsError>
where
    Primary: Clone + From<PrimaryStruct>,
    PrimaryStruct: Clone,
    Opts: Clone + Default + Merge,
    ExtraOpts: Clone,
{
    match target {
        Either::Left(items) => check_items(location, valid_tags, items, check_primary)?,
        Either::Right(config) => {
            for tag in &config.default.tags {
                check_tag(location, tag, valid_tags)?;
            }
            check_items(location, valid_tags, &config.items, check_primary)?
        }
    }

    Ok(())
}

fn check_items<Primary, PrimaryStruct, Opts>(
    location: &str,
    valid_tags: &[&str],
    items: &Items<Primary, PrimaryStruct, Opts>,
    check_primary: OptionalCheck<Primary>,
) -> Result<(), PackOptsError>
where
    Primary: From<PrimaryStruct>,
    PrimaryStruct: Clone,
    Opts: Default,
{
    match items {
        Items::Single(item) => match item {
            Either::Left(_) => {}
            Either::Right(item) => {
                for tag in &item.tags {
                    check_tag(location, tag, valid_tags)?;
                }
                if let Some(check_primary) = check_primary {
                    check_primary(item.primary.clone().into())?;
                }
            }
        },
        Items::Multiple(items) => {
            for item in items {
                match item {
                    Either::Left(_) => {}
                    Either::Right(item) => {
                        for tag in &item.tags {
                            check_tag(location, tag, valid_tags)?;
                        }
                        if let Some(check_primary) = &check_primary {
                            check_primary(item.primary.clone().into())?;
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn check_tag(location: &str, tag: &str, valid_tags: &[&str]) -> Result<(), PackOptsError> {
    if !valid_tags.contains(&tag) {
        return Err(PackOptsError::TagError {
            location: location.to_string(),
            tag: tag.to_string(),
            valid_tags: valid_tags.iter().map(|x| x.to_string()).collect(),
        });
    }

    Ok(())
}

fn check_glob(location: &str, glob: &str) -> Result<(), PackOptsError> {
    if let Err(err) = Pattern::new(glob) {
        return Err(PackOptsError::GlobError {
            location: location.to_string(),
            glob: glob.to_string(),
            error: err,
        });
    }

    Ok(())
}

pub fn glob_matches(glob: &str, path: &Path) -> bool {
    Pattern::new(glob).expect("Invalid glob").matches_path(path)
}

pub struct Resolved {
    pub popups: Vec<ResolvedTarget<String>>,
    pub wallpapers: Vec<ResolvedTarget<String>>,
    pub notifications: Vec<ResolvedTarget<String, NotificationOpts>>,
    pub links: Vec<ResolvedTarget<String>>,
    pub prompts: Vec<ResolvedTarget<String>>,
}

pub struct ResolvedTarget<Primary, Opts = Empty> {
    pub primary: Primary,
    pub opts: Opts,
    pub tags: Vec<String>,
}
