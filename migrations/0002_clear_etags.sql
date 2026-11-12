-- One-time reset: synthetic (HTML/sitemap) feeds used to store the page's
-- ETag, which made their next refresh a 304 and skipped discovery. Force one
-- full GET so those rows self-heal; real feeds re-fetch unconditionally once.
UPDATE feeds SET etag = NULL, last_modified = NULL;
