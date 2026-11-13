-- YouTube channel artwork, fetched once from the channel page.
ALTER TABLE feeds ADD COLUMN avatar_url TEXT;
ALTER TABLE feeds ADD COLUMN banner_url TEXT;
-- Marks a completed artwork lookup so channels without a banner are not refetched.
ALTER TABLE feeds ADD COLUMN images_fetched_at TEXT;
