(function () {
  "use strict";
  var SUBS = "dopefeed.subs";
  var READ = "dopefeed.read";
  var CAP = 5000;

  function load(key) {
    try {
      var value = JSON.parse(localStorage.getItem(key) || "[]");
      return Array.isArray(value) ? value : [];
    } catch (e) {
      return [];
    }
  }

  function save(key, value) {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch (e) {
      /* storage unavailable: subscribing simply does not persist */
    }
  }

  var subs = load(SUBS).map(Number).filter(Number.isInteger);
  var read = load(READ).map(Number).filter(Number.isInteger);

  document.querySelectorAll("button[data-feed-id]").forEach(function (button) {
    var id = Number(button.dataset.feedId);
    function paint() {
      var on = subs.indexOf(id) !== -1;
      button.setAttribute("aria-pressed", on ? "true" : "false");
      button.textContent = on ? "\u2713" : "+";
    }
    paint();
    button.addEventListener("click", function () {
      subs = subs.indexOf(id) !== -1
        ? subs.filter(function (value) { return value !== id; })
        : subs.concat(id);
      save(SUBS, subs);
      paint();
    });
  });

  var readSet = new Set(read);
  document.querySelectorAll(".post-row[data-post-id]").forEach(function (row) {
    if (readSet.has(Number(row.dataset.postId))) {
      row.classList.add("is-read");
    }
  });

  var article = document.querySelector("article.reader[data-post-id]");
  if (article) {
    var postId = Number(article.dataset.postId);
    read = read.filter(function (value) { return value !== postId; }).concat(postId);
    save(READ, read.slice(-CAP));
  }

  var timeline = document.getElementById("timeline");
  if (timeline) {
    if (subs.length === 0) {
      timeline.innerHTML =
        '<div class="empty">Subscribe to feeds to build your timeline. ' +
        '<a class="text-link" href="/">Browse the directory \u2192</a></div>';
    } else if (window.htmx) {
      window.htmx.ajax("GET", "/api/timeline?feeds=" + encodeURIComponent(subs.join(",")), {
        target: "#timeline",
        swap: "innerHTML",
      });
    }
  }
})();
