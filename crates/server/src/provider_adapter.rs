use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::model::ChatAttachment;
use crate::provider_runtime_ingestion::RuntimeEvent;

#[derive(Debug)]
pub(crate) enum AdapterEvent {
    SessionId(String),
    Runtime(RuntimeEvent),
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub(crate) struct ProviderSessionState {
    pub provider_name: String,
    pub runtime_mode: String,
    pub provider_session_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct StartSessionInput {
    pub thread_id: String,
    pub cwd: String,
    pub model: Option<String>,
    pub runtime_mode: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct SendTurnInput {
    pub thread_id: String,
    pub turn_id: String,
    pub assistant_message_id: String,
    pub cwd: String,
    pub state_dir: String,
    pub prompt: String,
    pub model: Option<String>,
    pub model_options: Option<Value>,
    pub provider_options: Option<Value>,
    pub runtime_mode: String,
    pub interaction_mode: String,
    pub assistant_delivery_mode: Option<String>,
    pub attachments: Vec<ChatAttachment>,
    pub created_at: String,
}

#[derive(Default)]
pub(crate) struct ProviderAdapterRegistry {
    adapters: HashMap<String, Arc<dyn ProviderAdapter>>,
}

impl ProviderAdapterRegistry {
    pub(crate) fn new(adapters: Vec<Arc<dyn ProviderAdapter>>) -> Self {
        let adapters = adapters
            .into_iter()
            .map(|adapter| (adapter.provider_name().to_string(), adapter))
            .collect();
        Self { adapters }
    }

    pub(crate) fn get(&self, provider_name: &str) -> Option<Arc<dyn ProviderAdapter>> {
        self.adapters.get(provider_name).cloned()
    }
}

#[async_trait]
pub(crate) trait ProviderAdapter: Send + Sync {
    fn provider_name(&self) -> &'static str;

    async fn start_session(
        &self,
        session: &mut ProviderSessionState,
        input: &StartSessionInput,
    ) -> anyhow::Result<()>;

    async fn send_turn(
        &self,
        session: ProviderSessionState,
        input: SendTurnInput,
        events: mpsc::UnboundedSender<AdapterEvent>,
        kill: tokio::sync::oneshot::Receiver<()>,
    ) -> anyhow::Result<ProviderSessionState>;

    async fn interrupt_turn(
        &self,
        _session: &ProviderSessionState,
        _thread_id: &str,
        _turn_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn respond_to_approval(
        &self,
        _session: &ProviderSessionState,
        _thread_id: &str,
        _request_id: &str,
        _decision: &str,
    ) -> anyhow::Result<()> {
        anyhow::bail!("interactive approvals are not supported by this adapter")
    }

    async fn respond_to_user_input(
        &self,
        _session: &ProviderSessionState,
        _thread_id: &str,
        _request_id: &str,
        _answers: &Value,
    ) -> anyhow::Result<()> {
        anyhow::bail!("interactive user input is not supported by this adapter")
    }

    async fn stop_session(
        &self,
        _session: &ProviderSessionState,
        _thread_id: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
