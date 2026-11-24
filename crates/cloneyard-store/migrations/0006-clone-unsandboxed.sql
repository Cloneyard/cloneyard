-- The clones before this migration ran without the sandbox.
ALTER TABLE clone ADD COLUMN unsandboxed INTEGER NOT NULL DEFAULT 1;
