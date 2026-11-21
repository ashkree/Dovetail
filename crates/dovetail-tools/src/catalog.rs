//! Roles, suites, and the MCP tool catalogue ([`TOOLS`]).

use serde::{Deserialize, Serialize};

/// Which suites a session gets by default, and which role prompt it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The coordinator: every suite.
    Coordinator,
    /// A slice: Core, Coordination, Maps and Design.
    Slice,
    /// A side chat: Core only (ADR 0011, ADR 0016).
    SideChat,
    /// Reading mode's Ask (ADR 0018): no suites; it reads the research files.
    Ask,
}

impl Role {
    /// The value of [`crate::env::ROLE`].
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Coordinator => "coordinator",
            Role::Slice => "slice",
            Role::SideChat => "side-chat",
            Role::Ask => "ask",
        }
    }

    /// Parses [`crate::env::ROLE`]; anything unknown is a slice, the safer default.
    pub fn from_env_value(value: &str) -> Role {
        match value {
            "coordinator" => Role::Coordinator,
            "side-chat" => Role::SideChat,
            "ask" => Role::Ask,
            _ => Role::Slice,
        }
    }

    /// The suites the role gets unless `config.toml` says otherwise (ADR 0016).
    pub fn default_suites(self) -> &'static [Suite] {
        match self {
            Role::Coordinator => Suite::ALL,
            Role::Slice => &[Suite::Core, Suite::Coordination, Suite::Maps, Suite::Design],
            Role::SideChat => &[Suite::Core],
            Role::Ask => &[],
        }
    }
}

/// A named group of tools; a role's config lists the suites it gets (ADR 0016). Each tool's
/// name starts with its suite's [`Suite::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Suite {
    /// Questions, status and the decision log: every role.
    Core,
    /// The user's task list.
    Tasks,
    /// Slice lifecycle and project registration.
    Project,
    /// What sessions tell each other through Dovetail: a slice's reports (ADR 0022). Claims
    /// and ordering come in v1.1.
    Coordination,
    /// Research rounds, reports and proposals (ADR 0015).
    Research,
    /// Maps (decision 25).
    Maps,
    /// The effort canvas: add a block, add a card (into a block or incoming), link two cards
    /// (ADR 0025, ADR 0027). Coordinator only for now.
    Canvas,
    /// What a slice's previewed page can switch: `design_declare_variants` (ADR 0026).
    Design,
}

impl Suite {
    /// Every suite, in the order the ADR lists them.
    pub const ALL: &'static [Suite] = &[
        Suite::Core,
        Suite::Tasks,
        Suite::Project,
        Suite::Coordination,
        Suite::Research,
        Suite::Maps,
        Suite::Canvas,
        Suite::Design,
    ];

    /// The name in `config.toml` and [`crate::env::SUITES`], also the tools' prefix.
    pub fn as_str(self) -> &'static str {
        match self {
            Suite::Core => "core",
            Suite::Tasks => "tasks",
            Suite::Project => "project",
            Suite::Coordination => "coordination",
            Suite::Research => "research",
            Suite::Maps => "maps",
            Suite::Canvas => "canvas",
            Suite::Design => "design",
        }
    }

    /// Parses one suite name.
    pub fn from_name(name: &str) -> Option<Suite> {
        Suite::ALL.iter().copied().find(|s| s.as_str() == name)
    }

    /// The value of [`crate::env::SUITES`]: names joined by commas.
    pub fn join(suites: &[Suite]) -> String {
        suites
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Parses [`crate::env::SUITES`]; unknown names are skipped, so a typo grants nothing.
    pub fn parse_list(value: &str) -> Vec<Suite> {
        value
            .split(',')
            .filter_map(|n| Suite::from_name(n.trim()))
            .collect()
    }
}

/// A row of [`TOOLS`]: a tool's name, its suite, and the name it had before ADR 0016.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSpec {
    /// The name, without the server prefix.
    pub name: &'static str,
    /// The suite that grants it.
    pub suite: Suite,
    /// The unprefixed v1 name, still answered for one release as a deprecated alias.
    pub old_name: Option<&'static str>,
}

/// Every tool, in the order `tools/list` shows them.
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "core_ask_user",
        suite: Suite::Core,
        old_name: Some("ask_user"),
    },
    ToolSpec {
        name: "core_report_status",
        suite: Suite::Core,
        old_name: Some("report_status"),
    },
    ToolSpec {
        name: "core_log_decision",
        suite: Suite::Core,
        old_name: Some("log_decision"),
    },
    ToolSpec {
        name: "tasks_create",
        suite: Suite::Tasks,
        old_name: Some("create_task"),
    },
    ToolSpec {
        name: "tasks_list",
        suite: Suite::Tasks,
        old_name: Some("list_tasks"),
    },
    ToolSpec {
        name: "tasks_get",
        suite: Suite::Tasks,
        old_name: Some("get_task"),
    },
    ToolSpec {
        name: "tasks_update",
        suite: Suite::Tasks,
        old_name: Some("update_task"),
    },
    ToolSpec {
        name: "tasks_archive",
        suite: Suite::Tasks,
        old_name: Some("archive_task"),
    },
    ToolSpec {
        name: "project_start_slice",
        suite: Suite::Project,
        old_name: Some("start_slice"),
    },
    ToolSpec {
        name: "project_archive_slice",
        suite: Suite::Project,
        old_name: Some("archive_slice"),
    },
    ToolSpec {
        name: "project_add",
        suite: Suite::Project,
        old_name: Some("add_project"),
    },
    ToolSpec {
        name: "research_start_round",
        suite: Suite::Research,
        old_name: None,
    },
    ToolSpec {
        name: "research_write_report",
        suite: Suite::Research,
        old_name: None,
    },
    ToolSpec {
        name: "research_propose",
        suite: Suite::Research,
        old_name: None,
    },
    ToolSpec {
        name: "research_get",
        suite: Suite::Research,
        old_name: None,
    },
    ToolSpec {
        name: "coordination_report",
        suite: Suite::Coordination,
        old_name: None,
    },
    ToolSpec {
        name: "coordination_claim_criterion",
        suite: Suite::Coordination,
        old_name: None,
    },
    ToolSpec {
        name: "maps_draw",
        suite: Suite::Maps,
        old_name: Some("draw_map"),
    },
    ToolSpec {
        name: "canvas_add_block",
        suite: Suite::Canvas,
        old_name: None,
    },
    ToolSpec {
        name: "canvas_add_card",
        suite: Suite::Canvas,
        old_name: None,
    },
    ToolSpec {
        name: "canvas_link_cards",
        suite: Suite::Canvas,
        old_name: None,
    },
    ToolSpec {
        name: "design_declare_variants",
        suite: Suite::Design,
        old_name: None,
    },
];

/// The row of [`TOOLS`] named `name`, by its current or its old name.
pub fn tool_spec(name: &str) -> Option<&'static ToolSpec> {
    TOOLS
        .iter()
        .find(|t| t.name == name || t.old_name == Some(name))
}

/// The row whose old name is `name`: a call by a deprecated alias (ADR 0016).
pub fn deprecated_alias(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|t| t.old_name == Some(name))
}

/// The line added to the result of a call by an old name, so the model learns the new one.
pub fn deprecation_note(spec: &ToolSpec) -> String {
    format!(
        "Note: `{}` is deprecated and will be removed; call `{}` instead.",
        spec.old_name.unwrap_or(spec.name),
        spec.name
    )
}

/// The rows of [`TOOLS`] the suites grant, in table order.
pub fn tools_in(suites: &[Suite]) -> impl Iterator<Item = &'static ToolSpec> + '_ {
    TOOLS.iter().filter(|t| suites.contains(&t.suite))
}
