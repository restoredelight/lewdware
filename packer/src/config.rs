use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use pack_format::config::{MediaOpts, PackOpts, StringOrObject, StringOrVec};
use walkdir::WalkDir;

pub fn find_config(root: &PathBuf) -> Result<Config> {
    let config_path = root.join("config.json");

    let config: PackOpts = match fs::read_to_string(config_path) {
        Ok(content) => json5::from_str(&content)?,
        Err(err) => match err.kind() {
            ErrorKind::NotFound => Default::default(),
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

            path.is_file()
                && e.file_name().to_str() == Some("metadata.json")
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

    pub fn get_tags(&self, path: &Path) -> Vec<String> {
        let mut tags = Vec::new();

        if let Some(default_tag) = &self.root_config.default_tag {
            tags.push(default_tag.clone());
        }

        tags.extend(self.resolve_tags(Some(path)));

        tags.extend(get_tags_from_config(&self.root_config.media, path));

        for (root, config) in &self.nested_config {
            if let Ok(path) = path.strip_prefix(root) {
                tags.extend(get_tags_from_config(config, path));
            }
        }

        tags
    }

    pub fn notifications(&self) -> Result<Vec<(Notification, Vec<String>)>> {
        let mut result = Vec::new();

        self.get_notifications_from_config(&self.root_config.media, None, &mut result);

        for (path, config) in &self.nested_config {
            self.get_notifications_from_config(config, Some(path), &mut result);
        }

        Ok(result)
    }

    fn get_notifications_from_config(
        &self,
        config: &MediaOpts,
        path: Option<&Path>,
        result: &mut Vec<(Notification, Vec<String>)>,
    ) {
        if let Some(notifications) = &config.notifications {
            let mut default_tags: Vec<String> = self.resolve_tags(path);

            default_tags.extend(notifications.default.tags.iter().cloned());

            for notification in &notifications.items {
                let mut tags = default_tags.to_vec();
                let notification = match notification {
                    StringOrObject::String(body) => Notification {
                        summary: notifications.default.summary.as_ref().cloned(),
                        body: body.clone(),
                    },
                    StringOrObject::Object(notification) => {
                        for tag in &notification.opts.tags {
                            tags.push(tag.clone());
                        }

                        Notification {
                            summary: notification
                                .opts
                                .summary
                                .as_ref()
                                .or(notifications.default.summary.as_ref())
                                .cloned(),
                            body: notification.body.clone(),
                        }
                    }
                };

                result.push((notification, tags));
            }
        }
    }

    pub fn links(&self) -> Result<Vec<(Link, Vec<String>)>> {
        let mut result = Vec::new();

        self.get_links_from_config(&self.root_config.media, None, &mut result);

        for (path, config) in &self.nested_config {
            self.get_links_from_config(config, Some(path), &mut result);
        }

        Ok(result)
    }

    fn get_links_from_config(
        &self,
        config: &MediaOpts,
        path: Option<&Path>,
        result: &mut Vec<(Link, Vec<String>)>,
    ) {
        if let Some(links) = &config.links {
            let mut default_tags: Vec<String> = self.resolve_tags(path);

            default_tags.extend(links.default.tags.iter().cloned());

            for link in &links.items {
                let mut tags = default_tags.to_vec();

                let link = match link {
                    StringOrObject::String(link) => Link { link: link.clone() },
                    StringOrObject::Object(link) => {
                        for tag in &link.opts.tags {
                            tags.push(tag.clone());
                        }

                        Link {
                            link: link.link.clone(),
                        }
                    }
                };

                result.push((link, tags));
            }
        }
    }

    pub fn prompts(&self) -> Result<Vec<(Prompt, Vec<String>)>> {
        let mut result = Vec::new();

        self.get_prompts_from_config(&self.root_config.media, None, &mut result);

        for (path, config) in &self.nested_config {
            self.get_prompts_from_config(config, Some(path), &mut result);
        }

        Ok(result)
    }

    fn get_prompts_from_config(
        &self,
        config: &MediaOpts,
        path: Option<&Path>,
        result: &mut Vec<(Prompt, Vec<String>)>,
    ) {
        if let Some(prompts) = &config.prompts {
            let mut default_tags: Vec<String> = self.resolve_tags(path);

            default_tags.extend(prompts.default.tags.iter().cloned());

            for prompt in &prompts.items {
                let mut tags = default_tags.to_vec();

                let prompt = match prompt {
                    StringOrObject::String(prompt) => Prompt {
                        prompt: prompt.clone(),
                    },
                    StringOrObject::Object(prompt) => {
                        for tag in &prompt.opts.tags {
                            tags.push(tag.clone());
                        }

                        Prompt {
                            prompt: prompt.prompt.clone(),
                        }
                    }
                };

                result.push((prompt, tags));
            }
        }
    }

    fn resolve_tags(&self, path: Option<&Path>) -> Vec<String> {
        let mut tags: Vec<String> = self.root_config.default_tag.iter().cloned().collect();

        if let Some(path) = path {
            for (tag, targets) in &self.root_config.tags {
                match targets {
                    StringOrVec::String(x) => {
                        if path.starts_with(x) {
                            tags.push(tag.clone());
                        }
                    }
                    StringOrVec::Vec(items) => {
                        for item in items {
                            if path.starts_with(item) {
                                tags.push(tag.clone());
                                break;
                            }
                        }
                    }
                }
            }
        }

        tags
    }
}

fn get_tags_from_config(config: &MediaOpts, path: &Path) -> Vec<String> {
    let mut tags = Vec::new();

    if let Some(popups) = &config.popups {
        for tag in &popups.default.tags {
            tags.push(tag.clone());
        }

        for popup in &popups.items {
            if let StringOrObject::Object(popup) = popup
                && path.starts_with(&popup.path)
            {
                for tag in &popup.opts.tags {
                    tags.push(tag.clone());
                }
            }
        }
    }

    tags
}

pub struct Notification {
    pub summary: Option<String>,
    pub body: String,
}

pub struct Link {
    pub link: String,
}

pub struct Prompt {
    pub prompt: String,
}
