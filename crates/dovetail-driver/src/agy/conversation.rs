//! One agy conversation as a state machine with no I/O: lines in, events and lines out.
//!
//! The real session ([`super::AgySession`]) and the fake's replay both drive it, so a fixture
//! goes through the same mapping a live session does. Start-up is a chain of requests, each sent
//! when the previous one is answered: `initialize`, then `session/new` (or `session/resume`),
//! then the spec's model and mode. Prompts sent before that, or while a turn runs, wait in a
//! queue and go one at a time: each `session/prompt` is answered when its turn ends.

use super::args::mode_id;
use super::models::ModelTable;
use super::permission::{self, Pending};
use super::rpc::{self, Call, Rpc};
use super::turn::Turn;
use crate::event::*;
use crate::{ControlAnswer, DriverError, PermissionMode, SessionSpec, UserInput};
use dovetail_api::model::ReasoningEffort;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};

/// What one step produced: events for the core, lines to write to agy.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Step {
    pub(crate) events: Vec<DriverEvent>,
    pub(crate) lines: Vec<Value>,
    /// Close stdin: the conversation cannot go on (agy refused to open it).
    pub(crate) close: bool,
}

impl Step {
    pub(crate) fn line(line: Value) -> Self {
        Self {
            lines: vec![line],
            ..Self::default()
        }
    }

    fn event(event: DriverEvent) -> Self {
        Self {
            events: vec![event],
            ..Self::default()
        }
    }

    fn and(mut self, other: Step) -> Self {
        self.events.extend(other.events);
        self.lines.extend(other.lines);
        self.close |= other.close;
        self
    }
}

/// What the conversation is started with, from the spec.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Plan {
    /// `session/new` or `session/resume` params.
    pub(crate) open: Value,
    pub(crate) resume: bool,
    pub(crate) cwd: String,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<ReasoningEffort>,
    pub(crate) mode: PermissionMode,
    /// Tools allowed without asking.
    pub(crate) allowed_tools: Vec<String>,
}

impl Plan {
    /// The plan for `spec`; `role_prompt` is the role prompt file's text.
    pub(crate) fn of(spec: &SessionSpec, role_prompt: Option<&str>) -> Self {
        Self {
            open: spec.agy_session_params(role_prompt),
            resume: spec.resume,
            cwd: spec.cwd.to_string_lossy().into_owned(),
            model: spec.model.clone(),
            effort: spec.effort,
            mode: spec.permission_mode,
            allowed_tools: spec.agy.allowed_tools.clone(),
        }
    }
}

/// One conversation's state.
#[derive(Debug)]
pub(crate) struct Conversation {
    plan: Plan,
    rpc: Rpc,
    turn: Turn,
    models: ModelTable,
    /// The family and effort in use.
    current: Option<(String, Option<ReasoningEffort>)>,
    mode: PermissionMode,
    /// agy's conversation id, once opened.
    session_id: Option<String>,
    /// `agentInfo.version` from `initialize`.
    version: Option<String>,
    /// Start-up requests not yet answered; prompts wait until there are none.
    starting: usize,
    ready: bool,
    /// A `session/prompt` is unanswered.
    prompting: bool,
    queued: VecDeque<Value>,
    permissions: HashMap<ControlId, Pending>,
    switches: u64,
    /// Replaying a fixture: the recording holds its own requests, so none are made here.
    replay: bool,
}

impl Conversation {
    pub(crate) fn new(plan: Plan) -> Self {
        let mode = plan.mode;
        Self {
            plan,
            rpc: Rpc::default(),
            turn: Turn::default(),
            models: ModelTable::default(),
            current: None,
            mode,
            session_id: None,
            version: None,
            starting: 0,
            ready: false,
            prompting: false,
            queued: VecDeque::new(),
            permissions: HashMap::new(),
            switches: 0,
            replay: false,
        }
    }

    /// The first line: `initialize`, with no file system or terminal offered.
    pub(crate) fn start(&mut self) -> Value {
        self.rpc.request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                "clientInfo": {"name": "dovetail", "version": env!("CARGO_PKG_VERSION")},
            }),
            Call::Initialize,
        )
    }

    /// A fixture's request line (replay): remembered so its recorded answer maps.
    pub(crate) fn note_sent(&mut self, msg: &Value) {
        self.replay = true;
        self.rpc.note_sent(msg);
        if msg.get("method").and_then(Value::as_str) == Some("session/prompt") {
            self.prompting = true;
        }
    }

    /// The agy id in use.
    fn model_id(&self) -> Option<String> {
        let (family, effort) = self.current.as_ref()?;
        self.models.id_for(family, *effort).map(str::to_owned)
    }

    fn session_params(&self, extra: Value) -> Value {
        let mut params = json!({"sessionId": self.session_id});
        if let (Value::Object(p), Value::Object(e)) = (&mut params, extra) {
            p.extend(e);
        }
        params
    }

    /// One line from agy's stdout.
    pub(crate) fn received(&mut self, msg: &Value) -> Step {
        match (msg.get("id"), msg.get("method")) {
            (Some(_), Some(_)) => self.on_request(msg),
            (None, Some(_)) => self.on_notification(msg),
            (Some(id), None) => match self.rpc.take(id) {
                Some(call) => self.on_answer(call, msg),
                None => Step::event(DriverEvent::Unknown(msg.clone())),
            },
            (None, None) => Step::event(DriverEvent::Unknown(msg.clone())),
        }
    }

    fn on_notification(&mut self, msg: &Value) -> Step {
        let Some(update) = msg.pointer("/params/update") else {
            return Step::event(DriverEvent::Unknown(msg.clone()));
        };
        if update.get("sessionUpdate").and_then(Value::as_str) == Some("config_option_update") {
            if let Some(choice) = update
                .get("configOptions")
                .and_then(|o| self.models.choice_from_config(o))
            {
                self.current = Some(choice);
            }
            return Step::default();
        }
        let model = self.model_id();
        Step {
            events: self.turn.on_update(update, model.as_deref()),
            ..Step::default()
        }
    }

    fn on_request(&mut self, msg: &Value) -> Step {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        if msg.get("method").and_then(Value::as_str) != Some("session/request_permission") {
            return Step::line(rpc::method_not_found(&id));
        }
        let (request, pending) = permission::request_of(msg);
        if self.plan.allowed_tools.contains(&request.tool_name) {
            return Step::line(pending.answer(&ControlAnswer::allow()).0);
        }
        if self.mode == PermissionMode::DontAsk {
            let deny = ControlAnswer::deny("not allowed in this session");
            return Step::line(pending.answer(&deny).0);
        }
        self.permissions.insert(request.id.clone(), pending);
        Step::event(DriverEvent::PermissionRequest(request))
    }

    fn on_answer(&mut self, call: Call, msg: &Value) -> Step {
        let failed = msg.get("error").is_some();
        let result = msg.get("result").cloned().unwrap_or_default();
        match call {
            Call::Initialize if failed => self.cannot_open(msg),
            Call::Initialize => {
                self.version = result
                    .pointer("/agentInfo/version")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if self.replay {
                    return Step::default();
                }
                let method = if self.plan.resume {
                    "session/resume"
                } else {
                    "session/new"
                };
                Step::line(self.rpc.request(method, self.plan.open.clone(), Call::Open))
            }
            Call::Open if failed => self.cannot_open(msg),
            Call::Open => self.opened(&result),
            Call::StartModel { choice } => {
                if failed {
                    tracing::warn!("agy refused the start model: {}", rpc::error_text(msg));
                } else if !choice.0.is_empty() {
                    self.current = Some(choice);
                }
                self.start_step_done()
            }
            Call::StartMode => self.start_step_done(),
            Call::SetMode => Step::default(),
            Call::SetModel { switch, choice } => {
                if failed {
                    return Step::event(DriverEvent::ModelSwitchFailed {
                        switch,
                        error: rpc::error_text(msg),
                    });
                }
                self.current = Some(choice.clone());
                Step::event(DriverEvent::ModelSettings {
                    switch: Some(switch),
                    model: Some(choice.0),
                    effort: choice.1,
                })
            }
            Call::Prompt => {
                self.prompting = false;
                let model = self.model_id();
                let events = self.turn.end(msg, model.as_deref(), self.queued.len());
                Step {
                    events,
                    ..Step::default()
                }
                .and(self.next_prompt())
            }
        }
    }

    /// agy would not start or open the conversation: say why as a failed turn, and close.
    fn cannot_open(&mut self, msg: &Value) -> Step {
        let model = self.model_id();
        let mut step = Step {
            events: self.turn.end(msg, model.as_deref(), self.queued.len()),
            ..Step::default()
        };
        step.close = true;
        step
    }

    /// `session/new` or `session/resume` answered: report the session, then apply the spec's
    /// model and mode.
    fn opened(&mut self, result: &Value) -> Step {
        self.session_id = result
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                self.plan
                    .open
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        if let Some(list) = result.pointer("/models/availableModels") {
            self.models = ModelTable::from_available(list);
        }
        let current_id = result
            .pointer("/models/currentModelId")
            .and_then(Value::as_str);
        self.current = result
            .get("configOptions")
            .and_then(|o| self.models.choice_from_config(o))
            .or_else(|| current_id.and_then(|id| self.models.choice_of(id)));
        let agy_mode = result
            .pointer("/modes/currentModeId")
            .and_then(Value::as_str);
        let mut step = Step::event(DriverEvent::Init(InitInfo {
            session_id: self.session_id.clone().unwrap_or_default(),
            model: self.model_id().or(current_id.map(str::to_owned)),
            permission_mode: agy_mode
                .and_then(super::args::mode_as_cli)
                .map(str::to_owned),
            cwd: Some(self.plan.cwd.clone()),
            tools: Vec::new(),
            version: self.version.clone(),
            raw: result.clone(),
        }));
        step.events.push(DriverEvent::Models(self.models.infos()));
        if !self.replay {
            step.lines = self.start_requests(agy_mode.map(str::to_owned));
        }
        if self.starting == 0 {
            step = step.and(self.become_ready());
        }
        step
    }

    /// The requests that apply the spec's model and mode, where agy's differ (`agy_mode` is
    /// the mode it opened in). Each counts as a start-up step.
    fn start_requests(&mut self, agy_mode: Option<String>) -> Vec<Value> {
        let mut lines = Vec::new();
        if let Some(family) = self.plan.model.clone() {
            match self.models.id_for(&family, self.plan.effort) {
                Some(id) if Some(id) != self.model_id().as_deref() => {
                    let params = self.session_params(json!({"modelId": id}));
                    let call = Call::StartModel {
                        choice: (family, self.plan.effort),
                    };
                    lines.push(self.rpc.request("session/set_model", params, call));
                }
                Some(_) => {}
                None => tracing::warn!(
                    "agy does not offer {family} at {:?}; keeping its default",
                    self.plan.effort
                ),
            }
        }
        let wanted = mode_id(self.plan.mode);
        if agy_mode.as_deref() != Some(wanted) {
            let params = self.session_params(json!({"modeId": wanted}));
            lines.push(
                self.rpc
                    .request("session/set_mode", params, Call::StartMode),
            );
        }
        self.starting = lines.len();
        lines
    }

    fn start_step_done(&mut self) -> Step {
        self.starting = self.starting.saturating_sub(1);
        if self.starting == 0 && !self.ready {
            self.become_ready()
        } else {
            Step::default()
        }
    }

    /// Start-up is over: report the model in use, and send the first waiting prompt.
    fn become_ready(&mut self) -> Step {
        self.ready = true;
        let (model, effort) = self.current.clone().unzip();
        Step::event(DriverEvent::ModelSettings {
            switch: None,
            model,
            effort: effort.flatten(),
        })
        .and(self.next_prompt())
    }

    fn next_prompt(&mut self) -> Step {
        if !self.ready || self.prompting {
            return Step::default();
        }
        let Some(prompt) = self.queued.pop_front() else {
            return Step::default();
        };
        self.prompting = true;
        let params = self.session_params(json!({"prompt": prompt}));
        Step::line(self.rpc.request("session/prompt", params, Call::Prompt))
    }

    /// A user message: sent now, or queued behind start-up and the running turn.
    pub(crate) fn send(&mut self, input: &UserInput) -> Step {
        self.queued.push_back(prompt_blocks(input));
        self.next_prompt()
    }

    /// Cancels the running turn (`session/cancel`); its prompt then answers `cancelled`.
    pub(crate) fn interrupt(&mut self) -> Step {
        if !self.prompting {
            return Step::default();
        }
        Step::line(rpc::notification(
            "session/cancel",
            json!({"sessionId": self.session_id}),
        ))
    }

    pub(crate) fn set_mode(&mut self, mode: PermissionMode) -> Step {
        self.mode = mode;
        self.plan.mode = mode;
        if self.session_id.is_none() {
            return Step::default();
        }
        let params = self.session_params(json!({"modeId": mode_id(mode)}));
        Step::line(self.rpc.request("session/set_mode", params, Call::SetMode))
    }

    /// Switches model (`None`: the family in use) and effort. A choice agy does not list fails
    /// at once, with nothing sent.
    pub(crate) fn set_model(
        &mut self,
        model: Option<&str>,
        effort: Option<ReasoningEffort>,
    ) -> (SwitchId, Step) {
        self.switches += 1;
        let switch = SwitchId(self.switches);
        let family = model
            .map(str::to_owned)
            .or_else(|| self.current.as_ref().map(|(f, _)| f.clone()))
            .unwrap_or_default();
        let Some(id) = self.models.id_for(&family, effort).map(str::to_owned) else {
            let error = format!("agy does not offer {family} at that effort");
            return (
                switch,
                Step::event(DriverEvent::ModelSwitchFailed { switch, error }),
            );
        };
        let params = self.session_params(json!({"modelId": id}));
        let call = Call::SetModel {
            switch,
            choice: (family, effort),
        };
        (
            switch,
            Step::line(self.rpc.request("session/set_model", params, call)),
        )
    }

    /// Answers a held permission request, in any order.
    pub(crate) fn answer(
        &mut self,
        id: &ControlId,
        answer: &ControlAnswer,
    ) -> Result<Step, DriverError> {
        let pending = self
            .permissions
            .remove(id)
            .ok_or_else(|| DriverError::UnknownControl(id.clone()))?;
        let (line, interrupt) = pending.answer(answer);
        let step = Step::line(line);
        Ok(if interrupt {
            step.and(self.interrupt())
        } else {
            step
        })
    }
}

/// A user message as ACP prompt blocks: images first, then the text.
fn prompt_blocks(input: &UserInput) -> Value {
    let mut blocks: Vec<Value> = input
        .images
        .iter()
        .map(|image| {
            json!({
                "type": "image",
                "mimeType": image.media_type,
                "data": crate::base64::encode(&image.data),
            })
        })
        .collect();
    if !input.text.is_empty() || blocks.is_empty() {
        blocks.push(json!({"type": "text", "text": input.text}));
    }
    Value::Array(blocks)
}

#[cfg(test)]
mod tests;
