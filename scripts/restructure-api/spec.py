"""Mapping tables for the dovetail-api restructure (throwaway tooling, not part of the repo)."""

# --- where each top-level item of model.rs / protocol.rs goes: name -> file (relative to src/) ---
ITEM_DEST = {}


def put(dest, *names):
    for n in names:
        assert n not in ITEM_DEST, n
        ITEM_DEST[n] = dest


put("project/mod.rs", "ProjectId", "RepoId", "ProjectSummary", "RepoSummary", "ProjectCounts", "ProjectKind")
put("task/mod.rs", "TaskId", "Task", "TaskStatus", "PLevel", "Bucket")
put("task/command.rs", "NewTask", "TaskChange", "describe_task_patch")
put("agent/mod.rs", "AgentId", "Agent", "RunningTool", "AgentKind", "WorktreeRef", "AgentStatus",
    "PermissionMode", "SubagentProgress", "SessionLock", "SlashCommand", "TodoItem", "TodoStatus",
    "TodoListId")
put("inbox/mod.rs", "InboxId", "InboxItem", "InboxKind", "START_SLICE_PERMISSION",
    "REMOVE_WORKTREE_PERMISSION", "RELEASE_TAKEOVER_PERMISSION", "UPGRADE_READY_PERMISSION",
    "RuleSuggestion")
put("inbox/command.rs", "PermissionAnswer")
put("conversation/mod.rs", "EntryId", "ConversationTail", "ConversationEntry", "EntryKind",
    "agent_message_text", "AgentMessageParts")
put("conversation/attachment.rs", "MAX_ATTACHMENTS", "MB", "MAX_ATTACHMENT_BYTES", "size_text",
    "attachment_limit_text", "Attachment", "ImageType")
put("clipboard/mod.rs", "Paste")
put("activity/mod.rs", "ActivityItem", "ActivityKind", "AgentMessage")
put("status/mod.rs", "Totals", "Warning", "WarningId", "UNREGISTERED_WARNING")
put("usage/mod.rs", "UsageLimits", "UsageWindow", "UsageStatus")
put("decision/mod.rs", "DecisionId", "Decision", "Alternative", "DecisionAuthor")
put("decision/command.rs", "NewDecision")
put("worktree/mod.rs", "WorktreeInfo", "SyncState", "Changes", "PrLookup", "PullRequest", "PrState",
    "WorktreeSetup")
put("model/mod.rs", "ReasoningEffort", "BUILTIN_MODELS", "ModelOption", "model_option_index")
put("tangent/mod.rs", "TangentSummary", "tangent_agent", "tangent_task", "TANGENTS_PROJECT")
put("journal/mod.rs", "JournalState")
put("snapshot.rs", "Snapshot", "ACTIVITY_TAIL", "MESSAGE_TAIL", "CONVERSATION_TAIL",
    "CONVERSATION_KEEP")
put("wire.rs", "PROTOCOL_VERSION", "ServerMsg", "CoreLink")
# Event and Command are generated; Paste handled above.

# --- Command variants: old name -> (feature, new name) ---
CMD = {}
EVT = {}


def cmds(feature, *pairs):
    for p in pairs:
        old, new = p if isinstance(p, tuple) else (p, p)
        assert old not in CMD, old
        CMD[old] = (feature, new)


def evts(feature, *pairs):
    for p in pairs:
        old, new = p if isinstance(p, tuple) else (p, p)
        assert old not in EVT, old
        EVT[old] = (feature, new)


cmds("task", ("CreateTask", "Create"), ("UpdateTask", "Update"), ("MoveTask", "Move"),
     ("ArchiveTask", "Archive"), ("ReorderTask", "Reorder"))
cmds("decision", ("LogDecision", "Log"))
cmds("agent", "Interrupt", "CyclePermissionMode", "NewSlice", "CloseSlice", "NewSideChat")
cmds("conversation", "Prompt", ("LoadConversation", "Load"), "ListPaths")
cmds("inbox", "AnswerPermission", "AnswerQuestion", ("DismissInbox", "Dismiss"))
cmds("model", ("SetModel", "Set"))
cmds("mcp", ("Mcp", "Request"))
cmds("journal", "EndOfDay", "Handoff", "FreshSession")
cmds("tangent", ("NewTangent", "New"), ("AttachTangent", "Attach"), ("FileTangent", "File"),
     ("StartFromTangent", "StartFrom"), ("DropTangent", "Drop"), ("LoadTangent", "Load"))
cmds("terminal", "OpenEditor", ("TerminalReleased", "Released"), "TakeOver")
cmds("clipboard", ("PasteClipboard", "Paste"), "PasteText", ("CopyToClipboard", "Copy"))
cmds("upgrade", ("Upgrade", "Start"))
cmds("canvas", "CreateBlock", "RenameBlock", "AddCard", "PlaceCard", "SeeCard", "MoveCard",
     "DeleteCard", "DropIncoming", "LinkCards", "UnlinkCards", "CardFromReply", "LoadCard")
cmds("design", "StartPreview", "RestartPreview", "SelectVariant", "KeepVariants")
cmds("effort", ("CreateEffort", "Create"), ("RenameEffort", "Rename"), ("UpdateEffort", "Update"),
     "SetWhereWasI", "AddCriterion", "ClaimCriterion", "ApproveCriterion", "ReturnCriterion")
cmds("worktree", ("RemoveWorktree", "Remove"), ("RefreshWorktrees", "Refresh"))
cmds("project", ("OpenProject", "Open"), ("NewResearchProject", "NewResearch"),
     ("CreateProject", "Create"), ("RenameProject", "Rename"), ("SetProjectAccount", "SetAccount"),
     "AddRepo", "RemoveRepo", ("RegisterProject", "Register"), "SetRecipeProfile")
cmds("map", ("LoadMap", "Load"))
cmds("research", "StartRound", "CloseRound", "DropRound", ("SetResearchKind", "SetKind"),
     "FileUnfiled", ("LoadResearch", "Load"), "StartAsk")
cmds("library", "ListDocs", "LoadDoc", "BrowseProject", "LoadBrowsed")
cmds("app", "OpenExternal", "Resync", "Quit")

evts("task", ("TaskUpserted", "Upserted"), ("TaskRemoved", "Removed"))
evts("effort", ("EffortUpserted", "Upserted"))
evts("design", "PreviewChanged", "VariantsUpserted")
evts("canvas", "BlockUpserted", "CardUpserted", "CardRemoved", "CardLinked", "CardUnlinked",
     "CardLoaded")
evts("agent", ("AgentUpserted", "Upserted"), ("AgentRemoved", "Removed"), "CommandsChanged",
     ("TodosReplaced", "TodosReplaced"))
evts("inbox", ("InboxUpserted", "Upserted"), ("InboxResolved", "Resolved"))
evts("worktree", ("WorktreeUpserted", "Upserted"))
evts("activity", ("ActivityAdded", "Added"), ("MessageAdded", "MessageAdded"))
evts("conversation", ("ConversationAppended", "Appended"), "AssistantDelta",
     ("ConversationPage", "Page"), "PathsListed")
evts("status", "TotalsChanged", "WarningSet", "WarningCleared")
evts("usage", ("UsageChanged", "Changed"))
evts("decision", ("DecisionAdded", "Added"))
evts("map", ("MapUpserted", "Upserted"), ("MapLoaded", "Loaded"))
evts("research", ("ResearchUpserted", "Upserted"), ("ResearchLoaded", "Loaded"))
evts("tangent", ("TangentUpserted", "Upserted"), ("TangentRemoved", "Removed"),
     ("TangentLoaded", "Loaded"))
evts("library", "DocsListed", "DocLoaded", "ProjectBrowsed", "BrowsedLoaded")
evts("model", ("ModelsChanged", "Changed"))
evts("mcp", ("McpReply", "Reply"))
evts("app", "CommandFailed")
evts("clipboard", ("ClipboardPasted", "Pasted"))
evts("terminal", ("LendTerminal", "Lend"), ("TerminalReturned", "Returned"))
evts("journal", ("JournalChanged", "Changed"))
evts("upgrade", ("UpgradeBuilding", "Building"), ("UpgradeRestarting", "Restarting"))
evts("project", ("OpeningProject", "Opening"), ("ProjectUpserted", "Upserted"),
     ("RecipeStarted", "RecipeStarted"))

# Whole files that keep their content: old file -> new folder
WHOLE = {
    "effort.rs": "effort/mod.rs",
    "canvas.rs": "canvas/mod.rs",
    "design.rs": "design/mod.rs",
    "map.rs": "map/mod.rs",
    "mcp.rs": "mcp/mod.rs",
    "research.rs": "research/mod.rs",
}
# items taken out of research.rs into library/mod.rs
LIBRARY_ITEMS = ["DocEntry", "DocImage", "BrowsedProject", "BrowsedListing", "BrowseWhat", "BrowsedDoc"]
