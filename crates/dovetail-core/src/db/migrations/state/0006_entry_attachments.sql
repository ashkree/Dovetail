-- Task #1 (2026-11-09): images pasted into the chat bar. A user entry records the saved image
-- files as a JSON array of `dovetail_api::Attachment` (path, media type, size); NULL when none.
ALTER TABLE conversation_entries ADD COLUMN attachments TEXT;
