use std::collections::HashMap;

use crate::config::FilterConfig;
use crate::config::PostFilterConfig;
use crate::error::Error;

pub struct FilterPipelineBuilder {
    data: mustache::Data,
    filters: Vec<String>,
    content_type: String,
}

impl FilterPipelineBuilder {
    pub fn new(data: mustache::Data) -> Self {
        FilterPipelineBuilder {
            data,
            filters: Vec::new(),
            content_type: "video/MP2T".to_string(),
        }
    }

    pub fn build(self) -> (Vec<String>, String) {
        (self.filters, self.content_type)
    }

    pub fn add_pre_filters(
        &mut self,
        pre_filters: &HashMap<String, FilterConfig>,
        names: &[String],
    ) -> Result<usize, Error> {
        for name in names.iter() {
            if pre_filters.contains_key(name) {
                self.add_pre_filter(&pre_filters[name], name)?;
            } else {
                tracing::warn!(pre_filter = name, "No such pre-filter");
            }
        }
        Ok(self.filters.len())
    }

    pub fn add_service_filter(&mut self, config: &FilterConfig) -> Result<usize, Error> {
        self.add_builtin_filter(config, "service-filter")
    }

    pub fn add_decode_filter(&mut self, config: &FilterConfig) -> Result<usize, Error> {
        self.add_builtin_filter(config, "decode-filter")
    }

    pub fn add_program_filter(&mut self, config: &FilterConfig) -> Result<usize, Error> {
        self.add_builtin_filter(config, "program-filter")
    }

    pub fn add_post_filters(
        &mut self,
        post_filters: &HashMap<String, PostFilterConfig>,
        names: &[String],
    ) -> Result<usize, Error> {
        for name in names.iter() {
            if post_filters.contains_key(name) {
                self.add_post_filter(&post_filters[name], name)?;
            } else {
                tracing::warn!(post_filter = name, "No such post-filter");
            }
        }
        Ok(self.filters.len())
    }

    fn add_pre_filter(&mut self, config: &FilterConfig, name: &str) -> Result<usize, Error> {
        if config.command.is_empty() {
            return Ok(self.filters.len());
        }
        let filter = match self.make_filter(&config.command) {
            Ok(filter) => filter,
            Err(err) => {
                tracing::error!(%err, pre_filter = name, "Failed to render pre-filter");
                return Err(err);
            }
        };
        if filter.is_empty() {
            tracing::warn!(pre_filter = name, "Empty pre-filter");
        } else {
            self.filters.push(filter);
        }
        Ok(self.filters.len())
    }

    fn add_builtin_filter(&mut self, config: &FilterConfig, name: &str) -> Result<usize, Error> {
        if config.command.is_empty() {
            return Ok(self.filters.len());
        }
        let filter = match self.make_filter(&config.command) {
            Ok(filter) => filter,
            Err(err) => {
                tracing::error!(%err, filter = name, "Failed to render filter");
                return Err(err);
            }
        };
        if filter.is_empty() {
            tracing::warn!(filter = name, "Empty filter");
        } else {
            self.filters.push(filter);
        }
        Ok(self.filters.len())
    }

    fn add_post_filter(&mut self, config: &PostFilterConfig, name: &str) -> Result<usize, Error> {
        if config.command.is_empty() {
            return Ok(self.filters.len());
        }
        let filter = match self.make_filter(&config.command) {
            Ok(filter) => filter,
            Err(err) => {
                tracing::error!(%err, post_filter = name, "Failed to render post-filter");
                return Err(err);
            }
        };
        if filter.is_empty() {
            tracing::warn!(post_filter = name, "Empty post-filter");
        } else {
            self.filters.push(filter);
            if let Some(content_type) = config.content_type.as_ref() {
                self.content_type.clone_from(content_type);
            }
        }
        Ok(self.filters.len())
    }

    fn make_filter(&self, command: &str) -> Result<String, Error> {
        let template = mustache::compile_str(command)?;
        Ok(template
            .render_data_to_string(&self.data)?
            .trim()
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use test_log::test;

    #[test]
    fn test_make_filter() {
        let channel = channel_gr!("test", "channel");
        let user = tuner_user!(0, web; "user-id");

        let data = mustache::MapBuilder::new()
            .insert_str("channel_name", &channel.name)
            .insert("channel_type", &channel.channel_type)
            .unwrap()
            .insert_str("channel", &channel.channel)
            .insert("user", &user)
            .unwrap()
            .build();

        let builder = FilterPipelineBuilder::new(data);

        assert_matches!(builder.make_filter("{{channel_name}}"), Ok(cmd) => {
            assert_eq!(cmd, "test");
        });
        assert_matches!(builder.make_filter("{{channel_type}}"), Ok(cmd) => {
            assert_eq!(cmd, "GR");
        });
        assert_matches!(builder.make_filter("{{channel}}"), Ok(cmd) => {
            assert_eq!(cmd, "channel");
        });
        assert_matches!(builder.make_filter("{{#user}}{{priority}}{{/user}}"), Ok(cmd) => {
            assert_eq!(cmd, "0");
        });
        // rust-mustache seems to support directly accessing properties.
        assert_matches!(builder.make_filter("{{user.priority}}"), Ok(cmd) => {
            assert_eq!(cmd, "0");
        });
        assert_matches!(builder.make_filter("{{user.info.Web.id}}"), Ok(cmd) => {
            assert_eq!(cmd, "user-id");
        });
        assert_matches!(builder.make_filter("{{^user.info.Job}}not job{{/user.info.Job}}"), Ok(cmd) => {
            assert_eq!(cmd, "not job");
        });
    }
}
