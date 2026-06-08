//! [`DevSupportRuntime`] — the dev-support agent's per-turn engine wiring.
//!
//! This is the "wow": a [`KnowledgeBase`](smooth_operator_core::KnowledgeBase)
//! built from the ingested repo, plus the two tools that make the agent
//! genuinely useful over a codebase —
//!
//! 1. **`knowledge_search`** ([`KnowledgeSearchTool`]) — RAG over the **indexed
//!    snapshot** the `ingest` step pulled (prose + code + issues), and
//! 2. **`github_search`** ([`GithubSearchTool`]) — a **live** GitHub code/issue
//!    search for anything that landed after the last ingest,
//!
//! wired onto a real smooth-operator [`Agent`] against the live `llm.smoo.ai`
//! gateway. It mirrors the reference
//! [`KnowledgeChatRuntime`](smooth_operator::runtime::KnowledgeChatRuntime)
//! (knowledge auto-injection + the `knowledge_search` tool + cross-turn memory),
//! and adds the second tool — which the reference runtime does not expose — so
//! this example shows the full dev-team picture.
//!
//! ## Test seam
//!
//! Both external surfaces are injectable so the whole turn runs offline:
//! - [`DevSupportRuntime::with_llm_provider`] swaps the live LLM for a scripted
//!   [`MockLlmClient`](smooth_operator_core::llm_provider::MockLlmClient).
//! - [`DevSupportRuntime::with_github_backend`] swaps the live GitHub search for
//!   a stub [`GithubSearchBackend`].

use std::sync::{Arc, Mutex};

use anyhow::Result;
use smooth_operator_core::llm_provider::LlmProvider;
use smooth_operator_core::{Agent, AgentConfig, AgentEvent, LlmConfig, ToolRegistry};

use smooth_operator::tools::{
    GithubAuth as ToolGithubAuth, GithubSearchBackend, GithubSearchTool, KnowledgeSearchTool,
};
use smooth_operator::StorageAdapter;

use crate::config::{DevSupportConfig, ToolName};

/// Default cap on the agent loop's iterations per turn (LLM → tools → LLM → …).
const DEFAULT_MAX_ITERATIONS: u32 = 8;

/// The outcome of one dev-support turn: the final reply + every engine event.
#[derive(Debug, Clone)]
pub struct TurnOutcome {
    /// The agent's final natural-language answer.
    pub reply: String,
    /// Every [`AgentEvent`] the engine emitted, in order — inspect for
    /// `ToolCallStart` / `ToolCallComplete` to see which tools ran.
    pub events: Vec<AgentEvent>,
}

impl TurnOutcome {
    /// `true` if the agent invoked a tool named `tool_name` during the turn.
    #[must_use]
    pub fn invoked_tool(&self, tool_name: &str) -> bool {
        self.events.iter().any(|e| {
            matches!(
                e,
                AgentEvent::ToolCallStart { tool_name: name, .. } if name == tool_name
            )
        })
    }

    /// The completed result text of the first call to `tool_name`, if any.
    #[must_use]
    pub fn tool_result(&self, tool_name: &str) -> Option<&str> {
        self.events.iter().find_map(|e| match e {
            AgentEvent::ToolCallComplete {
                tool_name: name,
                result,
                ..
            } if name == tool_name => Some(result.as_str()),
            _ => None,
        })
    }

    /// The distinct tool names that completed during the turn, in first-seen
    /// order — for the chat REPL's "tools used" footer.
    #[must_use]
    pub fn tools_used(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for e in &self.events {
            if let AgentEvent::ToolCallComplete { tool_name, .. } = e {
                if !seen.contains(tool_name) {
                    seen.push(tool_name.clone());
                }
            }
        }
        seen
    }
}

/// A knowledge-grounded dev-support runtime wired with `knowledge_search` +
/// `github_search` over the ingested repo.
pub struct DevSupportRuntime {
    storage: Arc<dyn StorageAdapter>,
    llm: LlmConfig,
    owner: String,
    repo: String,
    system_prompt: String,
    enabled_tools: Vec<ToolName>,
    /// Auth for the live `github_search` backend (built per turn). Independent
    /// of the ingestion auth; both come from the same config.
    github_auth: ToolGithubAuth,
    max_iterations: u32,
    /// Test seam: a scripted LLM provider replacing the live client.
    llm_provider: Option<Arc<dyn LlmProvider>>,
    /// Test seam: a stub GitHub search backend replacing the live API.
    github_backend: Option<Arc<dyn GithubSearchBackend>>,
}

impl DevSupportRuntime {
    /// Build a runtime from the parsed config, an LLM config, the GitHub auth
    /// for the live `github_search` tool, and the storage holding the ingested
    /// knowledge.
    #[must_use]
    pub fn new(
        config: &DevSupportConfig,
        llm: LlmConfig,
        github_auth: ToolGithubAuth,
        storage: Arc<dyn StorageAdapter>,
    ) -> Self {
        Self {
            storage,
            llm,
            owner: config.github.owner.clone(),
            repo: config.github.repo.clone(),
            system_prompt: config.agent.system_prompt.clone(),
            enabled_tools: config.agent.tools.clone(),
            github_auth,
            max_iterations: DEFAULT_MAX_ITERATIONS,
            llm_provider: None,
            github_backend: None,
        }
    }

    /// Inject a scripted [`LlmProvider`] (e.g. a `MockLlmClient`) so the agent
    /// loop runs deterministically with no network/key (test seam).
    #[must_use]
    pub fn with_llm_provider(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.llm_provider = Some(provider);
        self
    }

    /// Inject a stub [`GithubSearchBackend`] so `github_search` runs offline
    /// (test seam).
    #[must_use]
    pub fn with_github_backend(mut self, backend: Arc<dyn GithubSearchBackend>) -> Self {
        self.github_backend = Some(backend);
        self
    }

    /// Override the agent-loop iteration cap (default 8).
    #[must_use]
    pub fn with_max_iterations(mut self, max: u32) -> Self {
        self.max_iterations = max;
        self
    }

    /// Build the `github_search` tool: either over the injected stub backend, or
    /// the live [`OctocrabGithubSearch`] backend.
    fn github_tool(&self) -> GithubSearchTool {
        match &self.github_backend {
            Some(backend) => {
                GithubSearchTool::with_backend(Arc::clone(backend), &self.owner, &self.repo)
            }
            None => GithubSearchTool::new(
                self.github_auth.clone(),
                self.owner.clone(),
                self.repo.clone(),
            ),
        }
    }

    /// Assemble the agent for a turn: knowledge auto-injection + the enabled
    /// tools (`knowledge_search` and/or `github_search`) over the ingested
    /// knowledge, with the mock provider attached when one was injected.
    fn build_agent(&self, events: Arc<Mutex<Vec<AgentEvent>>>) -> Agent {
        let knowledge = self.storage.knowledge();

        // (1) Knowledge auto-injection: the engine queries the KB with the
        //     user's message and prepends top matches before the first LLM call.
        let config = AgentConfig::new("dev-support", &self.system_prompt, self.llm.clone())
            .with_max_iterations(self.max_iterations)
            .with_knowledge(Arc::clone(&knowledge));

        // (2) The two dev-team tools, gated by config.
        let mut tools = ToolRegistry::new();
        if self.enabled_tools.contains(&ToolName::KnowledgeSearch) {
            tools.register(KnowledgeSearchTool::new(Arc::clone(&knowledge)));
        }
        if self.enabled_tools.contains(&ToolName::GithubSearch) {
            tools.register(self.github_tool());
        }

        let agent = Agent::new(config, tools)
            .with_checkpoint_store(self.storage.checkpoints())
            .with_event_handler(move |event| {
                events.lock().expect("event sink poisoned").push(event);
            });

        match &self.llm_provider {
            Some(provider) => agent.with_llm_provider(Arc::clone(provider)),
            None => agent,
        }
    }

    /// Run one grounded turn. Drives the real smooth-operator agent loop to
    /// completion and returns the final reply + every event emitted.
    ///
    /// # Errors
    /// Returns an error if the agent loop fails fatally.
    pub async fn run_turn(&self, user_message: &str) -> Result<TurnOutcome> {
        let events = Arc::new(Mutex::new(Vec::<AgentEvent>::new()));
        let agent = self.build_agent(Arc::clone(&events));

        let conversation = agent.run(user_message).await?;
        let reply = conversation
            .last_assistant_content()
            .unwrap_or_default()
            .to_string();

        // Drop the agent so its event-handler closure releases its `events`
        // clone, then move the collected events out.
        drop(agent);
        let events = match Arc::try_unwrap(events) {
            Ok(mutex) => mutex
                .into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Err(arc) => arc.lock().expect("event sink poisoned").clone(),
        };

        Ok(TurnOutcome { reply, events })
    }
}

/// Build the live-gateway [`LlmConfig`] from the config's model + a gateway key,
/// pointed at `gateway_url` (default [`crate::config::DEFAULT_GATEWAY_URL`]).
///
/// `max_tokens` is kept modest because the gateway is paid-per-token.
#[must_use]
pub fn gateway_llm_config(
    model: impl Into<String>,
    api_key: impl Into<String>,
    gateway_url: impl Into<String>,
    max_tokens: u32,
) -> LlmConfig {
    use smooth_operator_core::llm::{ApiFormat, RetryPolicy};
    LlmConfig {
        api_url: gateway_url.into(),
        api_key: api_key.into(),
        model: model.into(),
        max_tokens,
        temperature: 0.0,
        retry_policy: RetryPolicy::default(),
        api_format: ApiFormat::OpenAiCompat,
    }
}

/// Build the `github_search` tool auth from the same config the connector uses.
/// (The two auth enums are deliberately independent types — the tool crate
/// doesn't depend on the ingestion crate — so we translate here.)
///
/// # Errors
/// Propagates [`DevSupportConfig::resolve_github_auth`] failures.
pub fn tool_github_auth(config: &DevSupportConfig) -> Result<ToolGithubAuth> {
    use smooth_operator_ingestion::GithubAuth as IngestAuth;
    Ok(match config.resolve_github_auth()? {
        IngestAuth::Token(t) => ToolGithubAuth::Token(t),
        IngestAuth::AppInstallation {
            app_id,
            private_key,
            installation_id,
        } => ToolGithubAuth::AppInstallation {
            app_id,
            private_key,
            installation_id,
        },
        IngestAuth::Unauthenticated => ToolGithubAuth::Unauthenticated,
    })
}
