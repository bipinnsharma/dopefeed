-- Keep Charlie Deets' directory feed focused on writing. The current archive's
-- canonical article URLs use /posts/, while its legacy URLs used /writings/.
DELETE FROM posts
WHERE feed_id = (
  SELECT id FROM feeds
  WHERE title = 'Charlie Deets' AND url = 'https://charliedeets.com/'
)
AND url NOT LIKE 'https://charliedeets.com/posts/%';

UPDATE feeds
SET url = 'https://charliedeets.com/writing/',
    etag = NULL,
    last_modified = NULL,
    next_fetch_at = datetime('now')
WHERE title = 'Charlie Deets'
  AND url = 'https://charliedeets.com/';

UPDATE posts
SET url = substr(url, 1, length(url) - 1)
WHERE feed_id = (SELECT id FROM feeds WHERE title = 'Charlie Deets')
  AND url LIKE 'https://charliedeets.com/posts/%/';
