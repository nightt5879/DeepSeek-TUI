//! Search backend selection and the shared async adapter contract.

use std::time::Instant;

use async_trait::async_trait;

use super::contract::{BackendId, BackendSearch, QueryCapabilities, SearchQuery};
use crate::config::SearchProvider;
use crate::tools::spec::{ToolContext, ToolError};

#[async_trait]
pub(crate) trait SearchBackend {
    fn id(&self) -> BackendId;
    fn capabilities(&self) -> QueryCapabilities;
    async fn search(
        &self,
        query: &SearchQuery,
        deadline: Instant,
    ) -> Result<BackendSearch, ToolError>;
}

#[derive(Clone, Copy)]
pub(crate) struct BackendContext<'a> {
    tool_context: &'a ToolContext,
}

pub(crate) enum ConfiguredSearchBackend<'a> {
    Bing(BackendContext<'a>),
    DuckDuckGo(BackendContext<'a>),
    Tavily(BackendContext<'a>),
    Bocha(BackendContext<'a>),
    Metaso(BackendContext<'a>),
    Searxng(BackendContext<'a>),
    Baidu(BackendContext<'a>),
    Volcengine(BackendContext<'a>),
    Sofya(BackendContext<'a>),
}

impl<'a> ConfiguredSearchBackend<'a> {
    #[must_use]
    pub(crate) fn from_context(context: &'a ToolContext) -> Self {
        let backend = BackendContext {
            tool_context: context,
        };
        match context.search_provider {
            SearchProvider::Bing => Self::Bing(backend),
            SearchProvider::DuckDuckGo => Self::DuckDuckGo(backend),
            SearchProvider::Tavily => Self::Tavily(backend),
            SearchProvider::Bocha => Self::Bocha(backend),
            SearchProvider::Metaso => Self::Metaso(backend),
            SearchProvider::Searxng => Self::Searxng(backend),
            SearchProvider::Baidu => Self::Baidu(backend),
            SearchProvider::Volcengine => Self::Volcengine(backend),
            SearchProvider::Sofya => Self::Sofya(backend),
        }
    }

    const fn provider(&self) -> SearchProvider {
        match self {
            Self::Bing(_) => SearchProvider::Bing,
            Self::DuckDuckGo(_) => SearchProvider::DuckDuckGo,
            Self::Tavily(_) => SearchProvider::Tavily,
            Self::Bocha(_) => SearchProvider::Bocha,
            Self::Metaso(_) => SearchProvider::Metaso,
            Self::Searxng(_) => SearchProvider::Searxng,
            Self::Baidu(_) => SearchProvider::Baidu,
            Self::Volcengine(_) => SearchProvider::Volcengine,
            Self::Sofya(_) => SearchProvider::Sofya,
        }
    }

    const fn context(&self) -> &BackendContext<'a> {
        match self {
            Self::Bing(context)
            | Self::DuckDuckGo(context)
            | Self::Tavily(context)
            | Self::Bocha(context)
            | Self::Metaso(context)
            | Self::Searxng(context)
            | Self::Baidu(context)
            | Self::Volcengine(context)
            | Self::Sofya(context) => context,
        }
    }
}

#[async_trait]
impl SearchBackend for ConfiguredSearchBackend<'_> {
    fn id(&self) -> BackendId {
        match self.provider() {
            SearchProvider::Bing => BackendId::Bing,
            SearchProvider::DuckDuckGo => BackendId::DuckDuckGo,
            SearchProvider::Tavily => BackendId::Tavily,
            SearchProvider::Bocha => BackendId::Bocha,
            SearchProvider::Metaso => BackendId::Metaso,
            SearchProvider::Searxng => BackendId::Searxng,
            SearchProvider::Baidu => BackendId::Baidu,
            SearchProvider::Volcengine => BackendId::Volcengine,
            SearchProvider::Sofya => BackendId::Sofya,
        }
    }

    fn capabilities(&self) -> QueryCapabilities {
        // All current adapters enforce result count. Other knobs are either
        // post-filtered by the shared harness or reported as not honored.
        QueryCapabilities::count_only()
    }

    async fn search(
        &self,
        query: &SearchQuery,
        deadline: Instant,
    ) -> Result<BackendSearch, ToolError> {
        crate::tools::web_search::run_backend_search(
            self.provider(),
            query,
            deadline,
            self.context().tool_context,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_configured_provider_maps_to_one_explicit_backend_adapter() {
        let cases = [
            (SearchProvider::Bing, BackendId::Bing),
            (SearchProvider::DuckDuckGo, BackendId::DuckDuckGo),
            (SearchProvider::Tavily, BackendId::Tavily),
            (SearchProvider::Bocha, BackendId::Bocha),
            (SearchProvider::Metaso, BackendId::Metaso),
            (SearchProvider::Searxng, BackendId::Searxng),
            (SearchProvider::Baidu, BackendId::Baidu),
            (SearchProvider::Volcengine, BackendId::Volcengine),
            (SearchProvider::Sofya, BackendId::Sofya),
        ];

        for (provider, expected) in cases {
            let mut context = ToolContext::new(std::path::PathBuf::from("."));
            context.search_provider = provider;
            let backend = ConfiguredSearchBackend::from_context(&context);
            assert_eq!(backend.id(), expected);
            assert_eq!(
                backend.capabilities().max_results,
                super::super::contract::CapabilityState::Supported
            );
        }
    }
}
