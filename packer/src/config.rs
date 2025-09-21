use std::{
    fs,
    io::ErrorKind,
    mem,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use glob::Pattern;
use merge::Merge;
use pack_format::{
    config::{MediaOpts, NotificationOpts, OneOrMore, PackOpts},
    target::{Either, Empty, Item, Items, Target},
};
use walkdir::WalkDir;

use crate::db::MediaCategory;

pub fn find_config(root: &PathBuf) -> Result<Config> {
    let config_path = root.join("config.json");
    let config_path_json5 = root.join("config.json5");

    let config: PackOpts = match fs::read_to_string(config_path) {
        Ok(content) => json5::from_str(&content)?,
        Err(err) => match err.kind() {
            ErrorKind::NotFound => match fs::read_to_string(config_path_json5) {
                Ok(content) => json5::from_str(&content)?,
                Err(err) => match err.kind() {
                    ErrorKind::NotFound => Default::default(),
                    _ => {
                        bail!(err)
                    }
                },
            },
            _ => {
                bail!(err)
            }
        },
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
                && path.parent() != Some(root.as_path())
        })
    {
        let content = fs::read_to_string(file.path())?;

        nested_config.push((
            file.path().parent().unwrap().to_path_buf(),
            json5::from_str(&content)?,
        ));
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

    pub fn resolve(&self) -> Result<Resolved> {
        Ok(Resolved {
            popups: self.get_popups()?,
            wallpapers: self.get_wallpapers()?,
            notifications: self.get_notifications()?,
            links: self.get_links()?,
            prompts: self.get_prompts()?,
        })
    }

    pub fn get_tags_and_category(
        &self,
        path: &Path,
        resolved: &Resolved,
    ) -> Result<(Vec<String>, MediaCategory)> {
        let mut tags = Vec::new();
        let mut category = MediaCategory::Popup;

        tags.extend(self.resolve_tags(path)?);

        for popup in &resolved.popups {
            if glob_matches(&popup.primary, path)? {
                tags.extend(popup.tags.clone());
            }
        }

        for wallpaper in &resolved.wallpapers {
            if glob_matches(&wallpaper.primary, path)? {
                tags.extend(wallpaper.tags.clone());

                category = MediaCategory::Wallpaper;
            }
        }

        Ok((tags, category))
    }

    pub fn get_popups(&self) -> Result<Vec<ResolvedTarget<String>>> {
        self.get_targets(|media_opts| media_opts.popups.as_ref())
    }

    pub fn get_notifications(&self) -> Result<Vec<ResolvedTarget<String, NotificationOpts>>> {
        self.get_targets(|media_opts| media_opts.notifications.as_ref())
    }

    pub fn get_links(&self) -> Result<Vec<ResolvedTarget<String>>> {
        self.get_targets(|media_opts| media_opts.links.as_ref())
    }

    pub fn get_prompts(&self) -> Result<Vec<ResolvedTarget<String>>> {
        self.get_targets(|media_opts| media_opts.prompts.as_ref())
    }

    pub fn get_wallpapers(&self) -> Result<Vec<ResolvedTarget<String>>> {
        self.get_targets(|media_opts| media_opts.wallpaper.as_ref())
    }

    fn get_targets<Primary, PrimaryStruct, Opts, ExtraOpts>(
        &self,
        f: impl Fn(&MediaOpts) -> Option<&Target<Primary, PrimaryStruct, Opts, ExtraOpts>>,
    ) -> Result<Vec<ResolvedTarget<Primary, Opts>>>
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
                let default_tags = self.resolve_tags(path)?;

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

        Ok(result)
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

    fn resolve_tags(&self, path: &Path) -> Result<Vec<String>> {
        let mut tags: Vec<String> = self.root_config.default_tag.iter().cloned().collect();

        for (tag, targets) in &self.root_config.tags {
            match targets {
                OneOrMore::One(x) => {
                    if glob_matches(x, &path)? {
                        tags.push(tag.clone());
                    }
                }
                OneOrMore::More(items) => {
                    for item in items {
                        if glob_matches(item, &path)? {
                            tags.push(tag.clone());
                            break;
                        }
                    }
                }
            }
        }

        Ok(tags)
    }
}

pub fn glob_matches(glob: &str, path: &Path) -> Result<bool> {
    Ok(Pattern::new(glob)?.matches_path(path))
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
