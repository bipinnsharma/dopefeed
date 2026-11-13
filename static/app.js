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

  document.querySelectorAll("form[data-confirm]").forEach(function (form) {
    form.addEventListener("submit", function (event) {
      if (!window.confirm(form.dataset.confirm)) {
        event.preventDefault();
      }
    });
  });

  document.querySelectorAll("select[data-autosubmit]").forEach(function (select) {
    select.addEventListener("change", function () {
      if (select.form) select.form.submit();
    });
  });

  var path = location.pathname.replace(/\/+$/, "") || "/";
  document.querySelectorAll(".side-link, .mobile-link").forEach(function (link) {
    if (link.getAttribute("href") === path) {
      link.classList.add("is-active");
      link.setAttribute("aria-current", "page");
    }
  });

  document.querySelectorAll(".chips").forEach(function (group) {
    group.querySelectorAll(".chip").forEach(function (chip) {
      chip.addEventListener("click", function () {
        group.querySelectorAll(".chip").forEach(function (other) {
          other.classList.remove("is-active");
          other.removeAttribute("aria-current");
        });
        chip.classList.add("is-active");
        chip.setAttribute("aria-current", "page");
      });
    });
  });

  var THEME_KEY = "dopefeed.theme";
  var root = document.documentElement;
  var darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

  function themeMode() {
    return root.dataset.themeMode || "system";
  }

  function paintTheme() {
    document.querySelectorAll("[data-theme-mode]").forEach(function (button) {
      button.setAttribute("aria-pressed", button.dataset.themeMode === themeMode() ? "true" : "false");
    });
  }

  function applyTheme() {
    var mode = themeMode();
    var dark = mode === "dark" || (mode === "system" && darkQuery.matches);
    root.dataset.theme = dark ? "dark" : "light";
    paintTheme();
  }

  function suppressTransitions(fn) {
    var style = document.createElement("style");
    style.textContent = "*,*::before,*::after{transition:none!important}";
    document.head.appendChild(style);
    fn();
    void document.body.offsetWidth;
    requestAnimationFrame(function () {
      style.remove();
    });
  }

  document.querySelectorAll("[data-theme-mode]").forEach(function (button) {
    button.addEventListener("click", function () {
      root.dataset.themeMode = button.dataset.themeMode;
      try {
        localStorage.setItem(THEME_KEY, button.dataset.themeMode);
      } catch (e) {
        /* storage unavailable: theme applies for this page only */
      }
      suppressTransitions(applyTheme);
    });
  });

  darkQuery.addEventListener("change", function () {
    if (themeMode() === "system") suppressTransitions(applyTheme);
  });

  paintTheme();

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
