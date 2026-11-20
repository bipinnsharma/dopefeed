(function () {
  "use strict";
  var SUBS = "dopefeed.subs";
  var READ = "dopefeed.read";
  var CAP = 5000;
  var MAX_SUBS = 50;
  var reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

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

  /* Toasts follow Sonner's defaults: 4s dismiss, bottom-right stack, hover
     expands and pauses, swipe right/down to dismiss. */
  var TOAST_MS = 4000;
  var TOAST_GAP = 14;
  var TOAST_MAX = 3;
  var hoverCapable = window.matchMedia("(hover: hover)").matches;

  var toaster = document.createElement("div");
  toaster.className = "toaster";
  toaster.setAttribute("aria-live", "polite");
  document.body.appendChild(toaster);

  var toasts = [];

  function toastExpanded() {
    return hoverCapable && toaster.matches(":hover");
  }

  // toasts[0] is the newest; every later toast sits above it.
  function layoutToasts(expanded) {
    toaster.classList.toggle("is-expanded", expanded);
    var frontHeight = toasts.length ? toasts[0].height : 0;
    var offset = 0;
    toasts.forEach(function (t, index) {
      var el = t.el;
      if (index === 0) {
        el.style.height = t.height + "px";
        el.style.removeProperty("--toast-y");
        el.style.removeProperty("--toast-scale");
        el.classList.remove("is-collapsed");
        offset = t.height + TOAST_GAP;
        return;
      }
      if (expanded) {
        el.style.height = t.height + "px";
        el.style.setProperty("--toast-y", -offset + "px");
        el.style.setProperty("--toast-scale", "1");
        el.classList.remove("is-collapsed");
        offset += t.height + TOAST_GAP;
      } else {
        el.style.height = frontHeight + "px";
        el.style.setProperty("--toast-y", -(index * TOAST_GAP) + "px");
        el.style.setProperty("--toast-scale", String(1 + index * 0.05));
        el.classList.add("is-collapsed");
      }
    });
  }

  function dismiss(t, swiped) {
    if (t.removed) return;
    t.removed = true;
    clearTimeout(t.timer);
    var index = toasts.indexOf(t);
    if (index !== -1) toasts.splice(index, 1);
    t.el.style.pointerEvents = "none";
    if (!swiped) {
      t.el.style.setProperty("--toast-y", index === 0 ? "100%" : "40%");
      t.el.style.opacity = "0";
    }
    layoutToasts(toastExpanded());
    setTimeout(function () {
      t.el.remove();
    }, 400);
  }

  function pauseToast(t) {
    if (!t.timer) return;
    clearTimeout(t.timer);
    t.timer = null;
    t.remaining = Math.max(0, t.remaining - (Date.now() - t.startedAt));
  }

  function resumeToast(t) {
    if (t.removed || t.timer) return;
    t.startedAt = Date.now();
    t.timer = setTimeout(function () {
      dismiss(t);
    }, t.remaining);
  }

  if (hoverCapable) {
    toaster.addEventListener("mouseenter", function () {
      toasts.forEach(pauseToast);
      layoutToasts(true);
    });
    toaster.addEventListener("mouseleave", function () {
      layoutToasts(false);
      toasts.forEach(resumeToast);
    });
  }

  function attachSwipe(t) {
    var el = t.el;
    var start = null;
    var axis = null;
    function dampen(value) {
      return value / (1.5 + Math.abs(value) / 20);
    }
    el.addEventListener("pointerdown", function (event) {
      if (event.button !== 0 || t.removed) return;
      start = { x: event.clientX, y: event.clientY, at: Date.now() };
      axis = null;
      el.setPointerCapture(event.pointerId);
      el.classList.add("is-swiping");
    });
    el.addEventListener("pointermove", function (event) {
      if (!start) return;
      var dx = event.clientX - start.x;
      var dy = event.clientY - start.y;
      if (!axis && (Math.abs(dx) > 1 || Math.abs(dy) > 1)) {
        axis = Math.abs(dx) > Math.abs(dy) ? "x" : "y";
      }
      if (!axis) return;
      el.style.setProperty("--swipe-x", (axis === "x" ? Math.max(0, dx) : dampen(dx)) + "px");
      el.style.setProperty("--swipe-y", (axis === "y" ? Math.max(0, dy) : dampen(dy)) + "px");
    });
    el.addEventListener("pointerup", function (event) {
      if (!start) return;
      var dx = event.clientX - start.x;
      var dy = event.clientY - start.y;
      var amount = axis === "x" ? dx : dy;
      var velocity = Math.abs(amount) / Math.max(1, Date.now() - start.at);
      var swiped = axis && amount > 0 && (Math.abs(amount) >= 45 || velocity > 0.11);
      start = null;
      el.classList.remove("is-swiping");
      if (swiped) {
        el.style.setProperty("--swipe-x", axis === "x" ? "calc(100% + 48px)" : "0px");
        el.style.setProperty("--swipe-y", axis === "y" ? "calc(100% + 48px)" : "0px");
        dismiss(t, true);
      } else {
        el.style.setProperty("--swipe-x", "0px");
        el.style.setProperty("--swipe-y", "0px");
      }
    });
    el.addEventListener("pointercancel", function () {
      if (!start) return;
      start = null;
      el.classList.remove("is-swiping");
      el.style.setProperty("--swipe-x", "0px");
      el.style.setProperty("--swipe-y", "0px");
    });
  }

  function showToast(message) {
    if (toasts.length >= TOAST_MAX) {
      var oldest = toasts.pop();
      clearTimeout(oldest.timer);
      oldest.el.remove();
    }
    var el = document.createElement("div");
    el.className = "toast";
    el.setAttribute("role", "status");
    var text = document.createElement("span");
    text.textContent = message;
    el.appendChild(text);
    toaster.appendChild(el);
    var t = {
      el: el,
      // shortcut: measured once; a resize during the 4s lifetime keeps the old height.
      height: el.offsetHeight,
      remaining: TOAST_MS,
      startedAt: 0,
      timer: null,
      removed: false,
    };
    toasts.unshift(t);
    el.style.height = t.height + "px";
    attachSwipe(t);
    layoutToasts(toastExpanded());
    el.classList.add("is-mounted");
    t.startedAt = Date.now();
    t.timer = setTimeout(function () {
      dismiss(t);
    }, TOAST_MS);
  }

  var subs = load(SUBS).map(Number).filter(Number.isInteger);
  var read = load(READ).map(Number).filter(Number.isInteger);

  function paintState() {
    subs = load(SUBS).map(Number).filter(Number.isInteger);
    read = load(READ).map(Number).filter(Number.isInteger);
    document.querySelectorAll("button[data-feed-id]").forEach(function (button) {
      button.setAttribute("aria-pressed", subs.indexOf(Number(button.dataset.feedId)) !== -1 ? "true" : "false");
    });
    var readSet = new Set(read);
    document.querySelectorAll(".post-row[data-post-id]").forEach(function (row) {
      row.classList.toggle("is-read", readSet.has(Number(row.dataset.postId)));
    });
  }

  paintState();
  document.addEventListener("click", function (event) {
    var button = event.target.closest("button[data-feed-id]");
    if (!button) return;
    var id = Number(button.dataset.feedId);
    subs = load(SUBS).map(Number).filter(Number.isInteger);
    var wasOn = subs.indexOf(id) !== -1;
    if (!wasOn && subs.length >= MAX_SUBS) {
      showToast("You can subscribe to up to 50 feeds. Unsubscribe from one first.");
      return;
    }
    subs = wasOn
      ? subs.filter(function (value) { return value !== id; })
      : subs.concat(id);
    save(SUBS, subs);
    paintState();
    if (!wasOn) showToast((button.dataset.feedTitle || "Feed") + " is added to your feed.");
  });
  document.addEventListener("htmx:afterSwap", paintState);
  document.addEventListener("htmx:afterSwap", function (event) {
    if (reduceMotion) return;
    var trigger = event.detail && event.detail.elt;
    var categoryChange = trigger && trigger.closest(".chips");
    var loadMore = trigger && trigger.closest(".load-more");
    if (!categoryChange && !loadMore) return;

    var grids = document.querySelectorAll("#feed-grid .feed-grid");
    var grid = grids[loadMore ? grids.length - 1 : 0];
    if (!grid) return;
    grid.querySelectorAll(".feed-card").forEach(function (card, index) {
      card.style.setProperty("--feed-enter-delay", Math.min(index * 30, 300) + "ms");
    });
    grid.classList.add("is-entering");
  });
  window.addEventListener("pageshow", paintState);
  window.addEventListener("storage", function (event) {
    if (event.key === SUBS || event.key === READ) paintState();
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
    var filters = document.getElementById("filters");
    var sortForm = document.querySelector(".sort-form");
    var query = filters && filters.querySelector('[name="q"]');
    if (query && sortForm) {
      query.addEventListener("input", function () {
        sortForm.querySelector('[name="q"]').value = query.value;
      });
    }
    group.querySelectorAll(".chip").forEach(function (chip) {
      chip.addEventListener("click", function () {
        var category = new URL(chip.href, location.href).searchParams.get("category") || "";
        if (filters) filters.querySelector('[name="category"]').value = category;
        if (sortForm) sortForm.querySelector('[name="category"]').value = category;
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

  var FONT_KEY = "dopefeed.readerFont";
  var fontSelect = document.querySelector("select[data-reader-font]");
  if (fontSelect) {
    fontSelect.value = root.dataset.readerFont || "book";
    fontSelect.addEventListener("change", function () {
      root.dataset.readerFont = fontSelect.value;
      try {
        localStorage.setItem(FONT_KEY, fontSelect.value);
      } catch (e) {
        /* storage unavailable: font applies for this page only */
      }
    });
  }

  var barSearchToggle = document.querySelector(".bar-search-toggle");
  var barSearchForm = document.querySelector(".topbar-search");
  var barSearchActions = document.querySelector(".mobile-bar-actions");
  if (barSearchToggle && barSearchForm && barSearchActions) {
    var barSearchHome = barSearchForm.parentElement;
    var barSearchOpen = false;
    var barSearchTimer;

    function setBarSearch(open) {
      clearTimeout(barSearchTimer);
      barSearchOpen = open;
      barSearchToggle.setAttribute("aria-expanded", open ? "true" : "false");
      barSearchToggle.setAttribute("aria-label", open ? "Close search" : "Search");
      if (open) {
        barSearchForm.classList.add("is-collapsed");
        barSearchActions.insertBefore(barSearchForm, barSearchToggle);
        barSearchForm.offsetWidth; // flush the collapsed width so the expand runs
        barSearchForm.classList.remove("is-collapsed");
        barSearchForm.querySelector("input").focus();
      } else {
        barSearchForm.classList.add("is-collapsed");
        barSearchTimer = setTimeout(function () {
          barSearchForm.classList.remove("is-collapsed");
          barSearchHome.appendChild(barSearchForm);
        }, 200);
      }
    }

    barSearchToggle.addEventListener("click", function () {
      setBarSearch(!barSearchOpen);
    });
    barSearchForm.addEventListener("keydown", function (event) {
      if (event.key === "Escape" && barSearchOpen) {
        setBarSearch(false);
        barSearchToggle.focus();
      }
    });
    window.addEventListener("resize", function () {
      if (!barSearchOpen || window.innerWidth <= 1023) return;
      clearTimeout(barSearchTimer);
      barSearchOpen = false;
      barSearchToggle.setAttribute("aria-expanded", "false");
      barSearchToggle.setAttribute("aria-label", "Search");
      barSearchForm.classList.remove("is-collapsed");
      barSearchHome.appendChild(barSearchForm);
    });
  }

  var menuToggle = document.getElementById("menu-toggle");
  var mobileMenu = document.getElementById("mobile-menu");
  if (menuToggle && mobileMenu) {
    var menuAnimation;
    function setMenu(open) {
      menuToggle.setAttribute("aria-expanded", open ? "true" : "false");
      menuToggle.setAttribute("aria-label", open ? "Close menu" : "Open menu");
      if (menuAnimation) menuAnimation.cancel();
      if (reduceMotion || !mobileMenu.animate) {
        mobileMenu.hidden = !open;
        return;
      }
      if (open) {
        mobileMenu.hidden = false;
        menuAnimation = mobileMenu.animate(
          [
            { opacity: 0, transform: "translateY(-8px)" },
            { opacity: 1, transform: "translateY(0)" },
          ],
          { duration: 220, easing: "cubic-bezier(0.2, 0, 0, 1)", fill: "both" }
        );
      } else {
        menuAnimation = mobileMenu.animate(
          [
            { opacity: 1, transform: "translateY(0)" },
            { opacity: 0, transform: "translateY(-8px)" },
          ],
          { duration: 180, easing: "cubic-bezier(0.4, 0, 1, 1)", fill: "both" }
        );
        menuAnimation.finished
          .then(function () {
            if (menuToggle.getAttribute("aria-expanded") === "false") mobileMenu.hidden = true;
          })
          .catch(function () {});
      }
    }
    menuToggle.addEventListener("click", function () {
      setMenu(mobileMenu.hidden);
    });
    mobileMenu.addEventListener("click", function (event) {
      if (event.target.closest("a")) setMenu(false);
    });
    document.addEventListener("click", function (event) {
      if (!mobileMenu.hidden && !mobileMenu.contains(event.target) && !menuToggle.contains(event.target)) {
        setMenu(false);
      }
    });
    document.addEventListener("keydown", function (event) {
      if (event.key === "Escape" && !mobileMenu.hidden) {
        setMenu(false);
        menuToggle.focus();
      }
    });
  }

  var chips = document.querySelector(".chips");
  if (chips && chips.parentElement) {
    var filterRow = chips.parentElement;
    var updateFade = function () {
      var max = chips.scrollWidth - chips.clientWidth;
      filterRow.toggleAttribute("data-fade", max > 1 && chips.scrollLeft < max - 1);
    };
    chips.addEventListener("scroll", updateFade, { passive: true });
    window.addEventListener("resize", updateFade);
    updateFade();
  }

  var timeline = document.getElementById("timeline");
  if (timeline) {
    if (subs.length > MAX_SUBS) {
      var warning = document.createElement("div");
      warning.className = "empty";
      warning.appendChild(document.createTextNode("You have " + subs.length + " saved subscriptions; the timeline supports up to 50. None were removed. "));
      var browse = document.createElement("a");
      browse.className = "text-link";
      browse.href = "/";
      browse.textContent = "Browse the directory";
      warning.appendChild(browse);
      warning.appendChild(document.createTextNode(" and unsubscribe from feeds, then reload."));
      timeline.replaceChildren(warning);
    } else if (subs.length === 0) {
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
