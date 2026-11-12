use crate::{config::Config, routes::submit, state::AppState, worker::discover};
use sqlx::SqlitePool;
use tokio::task::JoinSet;
use url::Url;

const SEED: &[(&str, &str, &str)] = &[
    (
        "Daring Fireball",
        "tech",
        "https://daringfireball.net/feeds/main",
    ),
    ("Poche Explore", "design", "https://poche.app/explore/rss"),
    (
        "Simon Willison",
        "coding",
        "https://simonwillison.net/atom/everything/",
    ),
    (
        "The Verge",
        "tech",
        "https://www.theverge.com/rss/index.xml",
    ),
    (
        "Peter Diamandis",
        "youtube",
        "discover:https://www.youtube.com/@peterdiamandis",
    ),
    ("少数派", "tech", "https://sspai.com/feed"),
    (
        "Lenny's Newsletter",
        "product",
        "https://www.lennysnewsletter.com/feed",
    ),
    (
        "Christopher Butler",
        "design",
        "https://www.chrbutler.com/feed.rss",
    ),
    ("Derek Sivers", "writing", "https://sive.rs/feed"),
    (
        "Every Inc",
        "youtube",
        "discover:https://www.youtube.com/@EveryInc",
    ),
    (
        "Anthropic Research",
        "ai",
        "discover:https://www.anthropic.com/research",
    ),
    (
        "David Senra",
        "youtube",
        "discover:https://www.youtube.com/@DavidSenra",
    ),
    (
        "Source Code",
        "startup",
        "https://every.to/source-code/feed",
    ),
    ("CSS-Tricks", "coding", "https://css-tricks.com/feed/"),
    ("exe.dev", "coding", "https://blog.exe.dev/atom.xml"),
    ("How to AI", "ai", "https://ruben.substack.com/feed"),
    ("Amp", "ai", "https://ampcode.com/news.rss"),
    (
        "Dwarkesh Patel",
        "youtube",
        "https://www.youtube.com/feeds/videos.xml?channel_id=UCXl4i9dYBrFOabk0xGmbkRA",
    ),
    ("GitHub Blog", "product", "https://github.blog/feed/"),
    ("Sources", "ai", "https://sources.news/feed"),
    (
        "First of Kind",
        "youtube",
        "discover:https://www.youtube.com/@firstofkind",
    ),
    ("Ben's Bites", "ai", "https://www.bensbites.com/feed"),
    (
        "The Pragmatic Engineer",
        "startup",
        "https://newsletter.pragmaticengineer.com/feed",
    ),
    ("Dingyi", "writing", "https://quaily.com/dingyi/feed/atom"),
    ("Linear", "product", "https://linear.app/rss/now.xml"),
    ("Karl Koch", "design", "https://karlkoch.me/rss.xml"),
    (
        "Dive Club",
        "youtube",
        "discover:https://www.youtube.com/@joindiveclub",
    ),
    (
        "The AI-Augmented Engineer",
        "ai",
        "https://www.augmentedswe.com/feed",
    ),
    ("Linus Lee", "coding", "https://thesephist.com/index.xml"),
    ("Vercel Blog", "product", "https://vercel.com/atom"),
    ("Greg Wilson", "coding", "https://third-bit.com/atom.xml"),
    (
        "Jim Nielsen",
        "coding",
        "https://blog.jim-nielsen.com/feed.xml",
    ),
    (
        "Proof of Concept",
        "design",
        "https://www.proofofconcept.pub/feed",
    ),
    (
        "Paul Stamatiou",
        "design",
        "https://paulstamatiou.com/posts.xml",
    ),
    (
        "Y Combinator (YouTube)",
        "startup",
        "https://www.youtube.com/feeds/videos.xml?channel_id=UCcefcZRL2oaA_uBNeo5UOWg",
    ),
    (
        "Sean Goedecke",
        "coding",
        "https://www.seangoedecke.com/rss.xml",
    ),
    (
        "美团技术团队",
        "coding",
        "https://tech.meituan.com/atom.xml",
    ),
    ("Lee Robinson", "coding", "discover:https://leerob.com"),
    (
        "Paul Graham",
        "ceo",
        "http://www.aaronsw.com/2002/feeds/pgessays.rss",
    ),
    (
        "Balint's Newsletter",
        "design",
        "https://balintbogdan.substack.com/feed",
    ),
    (
        "The Thinkers Club",
        "writing",
        "https://www.thethinkers.club/feed",
    ),
    ("Tim Ferriss", "writing", "https://tim.blog/feed/"),
    ("Platformer", "startup", "https://www.platformer.news/rss/"),
    (
        "Mitchell Hashimoto",
        "coding",
        "https://mitchellh.com/feed.xml",
    ),
    (
        "Erik Johannes Husom",
        "coding",
        "https://erikjohannes.no/index.xml",
    ),
    (
        "阮一峰",
        "coding",
        "https://feeds.feedburner.com/ruanyifeng",
    ),
    ("Bozhidar Batsov", "coding", "https://batsov.com/feed"),
    ("Arun Venkatesan", "design", "https://arun.is/rss.xml"),
    ("Ahmad Shadeed", "design", "https://ishadeed.com/feed.xml"),
    ("Daily Up", "writing", "https://www.dailyup.blog/rss.xml"),
    (
        "Eleanor Konik",
        "design",
        "https://www.eleanorkonik.com/feed",
    ),
    ("Takuya Matsuyama", "coding", "https://www.devas.life/rss/"),
    (
        "Maggie Appleton",
        "ai",
        "https://maggieappleton.com/rss.xml",
    ),
    ("Dan Koe", "writing", "https://letters.thedankoe.com/feed"),
    (
        "Luke Wroblewski",
        "design",
        "http://feeds.feedburner.com/FunctioningForm",
    ),
    ("The Pudding", "design", "https://pudding.cool/rss.xml"),
    ("Zach Holman", "startup", "https://zachholman.com/atom.xml"),
    (
        "Tobias van Schneider",
        "design",
        "https://vanschneider.com/blog/rss/",
    ),
    (
        "Karri Saarinen",
        "design",
        "https://karrisaarinen.com/rss.xml",
    ),
    ("Dan Luu", "coding", "https://danluu.com/atom.xml"),
    (
        "Minimalism",
        "lifestyle",
        "https://minimalism.substack.com/feed",
    ),
    (
        "Andrej Karpathy",
        "ai",
        "https://karpathy.bearblog.dev/feed/",
    ),
    (
        "George Hotz",
        "coding",
        "https://geohot.github.io/blog/feed.xml",
    ),
    ("Benji Taylor", "design", "discover:https://benji.org"),
    ("Will Larson", "coding", "https://lethain.com/feeds.xml"),
    ("Joel Hawksley", "coding", "https://hawksley.org/feed.xml"),
    ("DOC", "design", "https://doc.cc/feed/"),
    ("passo.uno", "writing", "https://passo.uno/posts/index.xml"),
    (
        "Baseline Design",
        "design",
        "https://newsletter.baselinedesign.com/rss/",
    ),
    ("danicat.dev", "coding", "https://danicat.dev/index.xml"),
    ("Anton Sten", "design", "https://www.antonsten.com/rss.xml"),
    (
        "Lex Fridman",
        "youtube",
        "discover:https://www.youtube.com/lexfridman",
    ),
    ("Figmalion", "design", "https://figmalion.com/feed.atom"),
    ("DHH", "ceo", "https://world.hey.com/dhh/feed.atom"),
    ("OneV's Den", "coding", "https://onevcat.com/feed.xml"),
    ("Shu Ding", "coding", "discover:https://shud.in/thoughts"),
    ("ADPList", "design", "https://adplist.substack.com/feed"),
    (
        "Josh Comeau",
        "coding",
        "https://www.joshwcomeau.com/rss.xml",
    ),
    ("Jakub Krčmář", "design", "https://jakub.kr/api/rss"),
    ("duxi.ao", "coding", "https://duxi.ao/feed.xml"),
    (
        "Maxime Heckel",
        "coding",
        "https://blog.maximeheckel.com/rss.xml",
    ),
    (
        "Google Design",
        "design",
        "discover:https://design.google/library",
    ),
    ("Nervegna", "design", "https://nervegna.substack.com/feed"),
    ("alexop.dev", "coding", "https://alexop.dev/rss.xml"),
    ("Stripe Dev Blog", "coding", "https://stripe.dev/blog/feed"),
    ("Dries Buytaert", "coding", "https://dri.es/rss.xml"),
    ("Interface Cafe", "design", "https://interfacecafe.com/rss/"),
    ("Steve Yegge", "dev", "https://medium.com/feed/@steve-yegge"),
    (
        "Mei Time",
        "youtube",
        "discover:https://www.youtube.com/@Meitimeyt",
    ),
    ("Sophie Koonin", "coding", "https://localghost.dev/feed.xml"),
    (
        "Founders Podcast",
        "youtube",
        "discover:https://www.youtube.com/@founderspodcast1",
    ),
    ("Matt Shumer", "ai", "discover:https://shumer.dev/blog"),
    (
        "os.rocorgi.wang",
        "coding",
        "discover:https://os.rocorgi.wang",
    ),
    (
        "Making Software",
        "design",
        "discover:https://www.makingsoftware.com",
    ),
    ("Nan Yu", "coding", "discover:https://www.nan.fyi"),
    ("Boris Tane", "coding", "https://boristane.com/rss.xml"),
    ("shaoruu.io", "coding", "https://shaoruu.io/atom.xml"),
    (
        "Carl Barenbrug",
        "design",
        "https://carlbarenbrug.com/feed/rss",
    ),
    (
        "Adventures in Claude",
        "ai",
        "https://adventuresinclaude.ai/index.xml",
    ),
    ("Oliur", "design", "discover:https://www.oliur.com"),
    (
        "Joan León - English",
        "coding",
        "https://joanleon.dev/rss.xml",
    ),
    (
        "Hardcore Software",
        "ceo",
        "https://hardcoresoftware.learningbyshipping.com/feed",
    ),
    (
        "Pablo Stanley",
        "design",
        "https://pablostanley.substack.com/feed",
    ),
    (
        "Amelia Wattenberger",
        "design",
        "https://wattenberger.com/rss",
    ),
    ("Marcin Wichary", "design", "https://aresluna.org/main.rss"),
    ("Zed Blog", "product", "https://zed.dev/blog.rss"),
    ("Addy Osmani", "coding", "https://addyosmani.com/rss.xml"),
    ("Dario Amodei", "ceo", "discover:https://darioamodei.com"),
    (
        "AI Product Playbook",
        "ai",
        "https://amankhan1.substack.com/feed",
    ),
    ("Matt Godbolt", "coding", "https://xania.org/feed.atom"),
    ("Jonny Burch", "design", "https://jonnyburch.com/feed.xml"),
    (
        "Weekend Engineering",
        "coding",
        "https://blog.alaindichiappari.dev/feed",
    ),
    (
        "Brian Lovin",
        "design",
        "https://brianlovin.com/writing/rss.xml",
    ),
    (
        "David Hoang",
        "design",
        "https://www.davidhoang.com/rss.xml",
    ),
    ("Hardik Pandya", "coding", "https://hvpandya.com/atom.xml"),
    ("Balaji Srinivasan", "design", "https://balajis.com/feed"),
    ("antirez", "coding", "https://antirez.com/rss"),
    ("Chanh Dai", "coding", "https://chanhdai.com/rss"),
    (
        "Jake Quist",
        "design",
        "https://www.jakequist.com/index.xml",
    ),
    ("Naval Ravikant", "startup", "https://nav.al/feed"),
    ("Terry Godier", "design", "discover:https://terrygodier.com"),
    (
        "Margaret-Anne Storey",
        "coding",
        "https://margaretstorey.com/feed",
    ),
    ("PJ Onori's blog", "design", "https://pjonori.blog/rss.xml"),
    ("Design Forces", "design", "https://df.fenx.work/rss/design"),
    ("@kentgigger", "coding", "https://kentgigger.com/rss.xml"),
    (
        "panzhixiang.cn",
        "coding",
        "https://panzhixiang.cn/atom.xml",
    ),
    ("Mario Zechner", "coding", "https://mariozechner.at/rss.xml"),
    ("/dev/michael", "coding", "https://michaellivs.com/rss.xml"),
    ("David Crawshaw", "coding", "https://crawshaw.io/atom.xml"),
    (
        "Andreessen Horowitz",
        "startup",
        "discover:https://a16z.com",
    ),
    (
        "OpenAI Developers",
        "ai",
        "discover:https://developers.openai.com/",
    ),
    ("Elad Blog", "startup", "https://blog.eladgil.com/feed"),
    ("Sam Altman", "ceo", "https://blog.samaltman.com/posts.atom"),
    ("Preet Suthar", "design", "discover:https://preetsuthar.me"),
    ("Fuma Nama", "coding", "https://www.fuma-nama.dev/rss.xml"),
    (
        "Nichecraft",
        "startup",
        "https://nichecraft.substack.com/feed",
    ),
    (
        "User Interface Wiki",
        "design",
        "https://www.userinterface.wiki/feed.xml",
    ),
    (
        "Jack Vaughan",
        "youtube",
        "discover:https://www.youtube.com/@JackVaughan",
    ),
    ("Ruslan Osipov", "coding", "https://rosipov.com/atom.xml"),
    ("Erik Nilsson", "design", "discover:https://eriks.design"),
    ("Rupert's World", "design", "https://ruperts.world/feed.xml"),
    (
        "Mark Anthony Cianfrani",
        "design",
        "https://cianfrani.dev/rss.xml",
    ),
    ("Not Boring", "design", "https://notbor.ing/words/rss.xml"),
    (
        "Helen Min",
        "design",
        "https://www.helenmin.com/blog?format=rss",
    ),
    ("Manuel Schipper", "coding", "https://schipper.ai/feed.xml"),
    (
        "Learn UI Design",
        "design",
        "https://www.learnui.design/feed.xml",
    ),
    ("nazha", "coding", "https://www.nazha.co/rss.xml"),
    ("Peter Steinberger", "coding", "https://steipete.me/rss.xml"),
    ("Asekachov", "design", "https://asekachov.substack.com/feed"),
    (
        "The Quiet Engineer",
        "coding",
        "https://www.quiet.engineer/feed.xml",
    ),
    ("Kailash Nadh", "coding", "https://nadh.in/index.xml"),
    ("Ralph Ammer", "design", "https://ralphammer.com/feed/"),
    ("Yann-Edern Gillet", "design", "https://yannglt.com/rss.xml"),
    (
        "Karl Lorey",
        "coding",
        "discover:https://karllorey.com/blog",
    ),
    ("Jacky Zhao", "coding", "https://jzhao.xyz/index.xml"),
    ("Alex Harri", "coding", "https://alexharri.com/rss.xml"),
    ("Dan Eden", "design", "https://daneden.me/feed.xml"),
    (
        "Chris Gregori",
        "coding",
        "https://www.chrisgregori.dev/rss.xml",
    ),
    ("z1han", "coding", "https://z1han.com/blog/rss.xml"),
    ("Dan Wang", "startup", "discover:https://danwang.co"),
    ("Artifact", "design", "https://www.readartifact.com/feed"),
    ("Steph Ango", "ceo", "https://stephango.com/feed.xml"),
    (
        "Charlie Deets",
        "design",
        "discover:https://charliedeets.com",
    ),
    ("Benja.dev", "coding", "discover:https://benja.dev/writing"),
    ("Reid Hoffman", "ceo", "https://www.reidhoffman.org/feed/"),
    (
        "Superorganizers",
        "lifestyle",
        "https://every.to/superorganizers/feed",
    ),
    ("Huozhi", "coding", "discover:https://huozhi.im"),
    ("Guillermo Rauch", "ceo", "https://rauchg.com/atom"),
    ("Cursor", "ai", "https://cursor.com/atom.xml"),
    ("Nadia Asparouhova", "writing", "https://nadia.xyz/feed.xml"),
    ("Andrew Chen", "startup", "https://andrewchen.com/feed/"),
    ("Pedro Duarte", "coding", "discover:https://ped.ro"),
    (
        "Alex Cornell",
        "design",
        "https://www.alexcornell.com/feed/",
    ),
    ("Rauno Freiberg", "design", "discover:https://rauno.me"),
    ("Marc Köhlbrugge", "startup", "https://marc.io/feed.xml"),
    (
        "Running in Production",
        "coding",
        "https://runninginproduction.com/podcast/feed.xml",
    ),
    ("Jeff Huang", "ai", "https://jeffhuang.com/feed.xml"),
    ("Emil Kowalski", "design", "discover:https://emilkowal.ski/"),
];

pub async fn run(pool: SqlitePool, config: Config) -> anyhow::Result<()> {
    let state = AppState::new(pool, config)?;
    let mut tasks = JoinSet::new();

    for &(title, category, raw_url) in SEED {
        let state = state.clone();
        tasks.spawn(async move {
            let _permit = state.fetch_slots.clone().acquire_owned().await;
            // `discover:` marks a site URL; resolve() finds the feed behind it.
            let raw_url = raw_url.strip_prefix("discover:").unwrap_or(raw_url);
            let Ok(url) = Url::parse(raw_url) else {
                tracing::warn!(url = raw_url, "invalid seed url");
                return;
            };
            match discover::resolve(&state.http, &url).await {
                Ok(resolved) => {
                    match submit::insert_resolved(&state, resolved, category, Some(title)).await {
                        Ok(Some(id)) => tracing::info!(%url, id, "seeded"),
                        Ok(None) => tracing::info!(%url, "already present"),
                        Err(error) => tracing::warn!(%url, %error, "seed insert failed"),
                    }
                }
                Err(error) => {
                    tracing::warn!(%url, %error, "seed fetch failed, adding placeholder");
                    if let Err(insert_error) =
                        insert_placeholder(&state, raw_url, title, category, &error).await
                    {
                        tracing::warn!(%url, %insert_error, "placeholder insert failed");
                    }
                }
            }
        });
    }
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            tracing::warn!(%error, "seed task failed");
        }
    }

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feeds")
        .fetch_one(&state.pool)
        .await?;
    tracing::info!(feeds = total, "seed complete");
    Ok(())
}

/// Offline fallback so a fresh clone has a populated directory without network.
/// Stores the real site URL (never `discover:`) so the worker can retry discovery.
pub(crate) async fn insert_placeholder(
    state: &AppState,
    raw_url: &str,
    title: &str,
    category: &str,
    error: &str,
) -> anyhow::Result<()> {
    let url = Url::parse(raw_url)?;
    let domain = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_string();
    let site_url = url.origin().ascii_serialization();
    sqlx::query(
        "INSERT INTO feeds (url, site_url, domain, title, category, status,
         last_error, next_fetch_at)
         VALUES (?, ?, ?, ?, ?, 'active', ?, datetime('now', '+60 seconds'))
         ON CONFLICT(url) DO NOTHING",
    )
    .bind(url.to_string())
    .bind(site_url)
    .bind(domain)
    .bind(title)
    .bind(category)
    .bind(error)
    .execute(&state.pool)
    .await?;
    Ok(())
}
