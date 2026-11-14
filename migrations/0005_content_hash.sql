-- Hash of the entry inputs that produced a row; an unchanged hash skips the
-- sanitize/extract/update work on the next refresh.
ALTER TABLE posts ADD COLUMN content_hash TEXT;
