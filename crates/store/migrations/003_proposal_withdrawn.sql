-- A proposal its own agent took back (not the same as the user rejecting it: the resend guard ignores these).
ALTER TABLE proposal ADD COLUMN withdrawn INTEGER NOT NULL DEFAULT 0;
