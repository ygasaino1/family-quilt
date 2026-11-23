-- Feedback on proposals: why one was rejected (shown to the agent), what the reviewer corrected before approving (a JSON
-- array), and a fingerprint of what the proposal asked for, so that an agent that sends the same thing again after a
-- rejection is told so (a JSON array of short strings).
ALTER TABLE proposal ADD COLUMN decision_note TEXT;
ALTER TABLE proposal ADD COLUMN fingerprints TEXT;
ALTER TABLE proposal ADD COLUMN reviewer_edits TEXT;
