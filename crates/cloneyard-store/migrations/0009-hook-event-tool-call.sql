-- Hook events recorded before this migration have no tool call.
ALTER TABLE hook_event ADD COLUMN tool_use_id TEXT;
ALTER TABLE hook_event ADD COLUMN command TEXT;
ALTER TABLE hook_event ADD COLUMN path TEXT;

CREATE INDEX hook_event_tool_use ON hook_event (clone_id, tool_use_id);
