/* Dev-only design annotator. Loaded only in debug builds (see base.html).
   Click "Annotate", click any element, type what should change, then Copy
   report and paste it into the chat. Nothing here runs in production. */
(function () {
  "use strict";

  var KEY = "dopefeed.annotations";
  var on = false;
  var target = null;
  var editing = false;

  var SVG =
    "<svg viewBox='0 0 24 24' fill='none' stroke='currentColor' stroke-width='1.5' " +
    "stroke-linecap='round' stroke-linejoin='round' aria-hidden='true'>";

  function icon(name) {
    var paths = {
      annotate: "<path d='M5 3l6.5 17 2.4-6.6 6.6-2.4z'/>",
      list: "<path d='M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01'/>",
      close: "<path d='M6 6l12 12M18 6L6 18'/>",
      copy: "<rect x='8.5' y='8.5' width='11' height='11' rx='2'/>" +
        "<path d='M15.5 8.5v-3a2 2 0 0 0-2-2h-8a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h3'/>",
      check: "<path d='M5 12.5l4.5 4.5L19 7.5'/>",
      trash: "<path d='M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2'/>" +
        "<path d='M6.5 7l.8 12a1 1 0 0 0 1 1h7.4a1 1 0 0 0 1-1l.8-12M10 11v6M14 11v6'/>"
    };
    return SVG + paths[name] + "</svg>";
  }

  var css =
    ".anno-dock,.anno-panel,.anno-composer{position:fixed;z-index:2147483000;" +
    "font:500 13px/1.4 var(--font-sans,system-ui);color:var(--color-ink,#2c2821)}" +
    ".anno-dock{right:16px;bottom:calc(16px + env(safe-area-inset-bottom,0px));display:flex;gap:2px;" +
    "padding:3px;border:1px solid var(--color-border,#e4d9c6);border-radius:12px;" +
    "background:var(--color-surface-solid,#fcfcfc);box-shadow:var(--shadow-md,0 1px 3px #0002)}" +
    ".anno-dock button{position:relative;display:inline-flex;align-items:center;justify-content:center;" +
    "width:34px;height:34px;padding:0;border:0;border-radius:9px;background:transparent;" +
    "color:var(--color-ink-secondary,#6a6053);cursor:pointer;transition:background 120ms,color 120ms}" +
    ".anno-dock button:hover{background:var(--color-fill-hover,#2c282114);color:var(--color-ink,#2c2821)}" +
    ".anno-dock svg{display:block;width:18px;height:18px}" +
    ".anno-dock .anno-annotate[aria-pressed='true']{background:var(--color-accent,#ff4000);color:#fff}" +
    ".anno-dock .anno-annotate[aria-pressed='true']:hover{background:var(--color-accent-strong,#f62b00)}" +
    ".anno-badge{position:absolute;top:1px;right:1px;min-width:15px;height:15px;padding:0 4px;" +
    "border-radius:9999px;background:var(--color-accent,#ff4000);color:#fff;font-size:9px;" +
    "font-weight:600;line-height:15px;text-align:center}" +
    ".anno-badge[hidden]{display:none}" +
    ".anno-box{position:fixed;z-index:2147483001;pointer-events:none;display:none;" +
    "border:1.5px solid var(--color-accent,#ff5c22);background:#ff5c2214;border-radius:2px}" +
    ".anno-panel,.anno-composer{right:16px;bottom:calc(64px + env(safe-area-inset-bottom,0px));" +
    "display:none;padding:10px 12px 12px;background:var(--color-surface-solid,#fcfcfc);" +
    "border:1px solid var(--color-border,#e4d9c6);border-radius:12px;" +
    "box-shadow:var(--shadow-lg,0 8px 24px #0003)}" +
    ".anno-panel.is-open,.anno-composer.is-open{display:block;animation:anno-rise 160ms cubic-bezier(.2,0,0,1)}" +
    ".anno-panel{width:min(360px,calc(100vw - 24px));max-height:60vh;overflow:auto}" +
    ".anno-composer{width:min(420px,calc(100vw - 24px))}" +
    "@keyframes anno-rise{from{opacity:0;transform:translateY(4px)}to{opacity:1;transform:none}}" +
    "@media (prefers-reduced-motion:reduce){.anno-panel.is-open,.anno-composer.is-open{animation:none}}" +
    ".anno-head{display:flex;align-items:center;gap:4px;margin-bottom:6px}" +
    ".anno-title{flex:1;font-size:13px;font-weight:600}" +
    ".anno-count{min-width:18px;padding:1px 6px;border-radius:9999px;background:var(--color-fill-hover,#2c282114);" +
    "font-size:11px;font-weight:500;text-align:center}" +
    ".anno-icon{display:inline-flex;align-items:center;justify-content:center;width:26px;height:26px;" +
    "padding:0;border:0;border-radius:7px;background:transparent;color:var(--color-ink-secondary,#6a6053);" +
    "cursor:pointer;transition:background 120ms,color 120ms}" +
    ".anno-icon:hover{background:var(--color-fill-hover,#2c282114);color:var(--color-ink,#2c2821)}" +
    ".anno-icon.is-danger:hover{background:var(--color-danger-surface,#ffe2dd);color:var(--color-danger,#ff0005)}" +
    ".anno-icon.is-done{color:var(--color-success,#00c500)}" +
    ".anno-icon svg{display:block;width:16px;height:16px}" +
    ".anno-dock button:focus-visible,.anno-icon:focus-visible,.anno-save:focus-visible," +
    ".anno-note:focus-visible{outline:2px solid var(--color-focus,#2287ff);outline-offset:2px}" +
    ".anno-target{margin:0;flex:1;min-width:0;font:400 11px/1.4 ui-monospace,monospace;" +
    "color:var(--color-ink-secondary,#6a6053);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}" +
    ".anno-meta{margin:-2px 0 8px;font-size:11px;color:var(--color-ink-tertiary,#9a8f7e)}" +
    ".anno-note{width:100%;min-height:72px;resize:vertical;box-sizing:border-box;padding:8px 10px;" +
    "font:400 13px/1.5 var(--font-sans,system-ui);color:inherit;background:transparent;" +
    "border:1px solid var(--color-border,#e4d9c6);border-radius:8px}" +
    ".anno-note::placeholder{color:var(--color-ink-tertiary,#9a8f7e)}" +
    ".anno-actions{display:flex;gap:6px;justify-content:flex-end;margin-top:10px}" +
    ".anno-save{display:inline-flex;align-items:center;justify-content:center;width:30px;height:30px;" +
    "padding:0;border:0;border-radius:8px;background:var(--color-accent,#ff4000);color:#fff;cursor:pointer}" +
    ".anno-save:hover{background:var(--color-accent-strong,#f62b00)}" +
    ".anno-save svg{display:block;width:16px;height:16px}" +
    ".anno-item{position:relative;padding:8px 26px 8px 0;border-top:1px solid var(--color-border,#e4d9c6)}" +
    ".anno-item p{margin:0;font-size:13px;overflow-wrap:anywhere}" +
    ".anno-item code{display:block;margin-top:2px;font:400 11px/1.4 ui-monospace,monospace;" +
    "color:var(--color-ink-tertiary,#9a8f7e);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}" +
    ".anno-item .anno-del{position:absolute;top:5px;right:-4px;opacity:0}" +
    ".anno-item:hover .anno-del,.anno-del:focus-visible{opacity:1}" +
    ".anno-empty{display:flex;gap:8px;align-items:flex-start;margin:6px 0 2px;" +
    "color:var(--color-ink-secondary,#6a6053)}" +
    ".anno-empty svg{flex:none;width:16px;height:16px;margin-top:1px;opacity:.7}" +
    "@media (hover:none){.anno-del{opacity:1}}" +
    "@media (pointer:coarse){.anno-dock button{width:44px;height:44px}.anno-icon{width:32px;height:32px}}" +
    "body.anno-on,body.anno-on *{cursor:crosshair !important}" +
    "body.anno-on .anno-dock,body.anno-on .anno-dock *," +
    "body.anno-on .anno-panel *,body.anno-on .anno-composer *{cursor:auto !important}" +
    "body.anno-on .anno-dock button{cursor:pointer !important}";
  var style = document.createElement("style");
  style.textContent = css;
  document.head.appendChild(style);

  var box = el("div", "anno-box");
  var dock = el("div", "anno-dock");
  var annotateBtn = el("button", "anno-annotate");
  annotateBtn.type = "button";
  annotateBtn.setAttribute("aria-pressed", "false");
  annotateBtn.setAttribute("aria-label", "Annotate elements");
  annotateBtn.title = "Annotate";
  annotateBtn.innerHTML = icon("annotate");
  var listBtn = el("button", "anno-list-toggle");
  listBtn.type = "button";
  listBtn.setAttribute("aria-expanded", "false");
  listBtn.setAttribute("aria-label", "Annotations");
  listBtn.title = "Annotations";
  listBtn.innerHTML = icon("list") + "<span class='anno-badge' hidden>0</span>";
  dock.appendChild(annotateBtn);
  dock.appendChild(listBtn);

  var composer = el("form", "anno-composer");
  composer.innerHTML =
    "<div class='anno-head'><code class='anno-target'></code>" +
    "<button type='button' class='anno-icon anno-cancel' title='Cancel' aria-label='Cancel'>" +
    icon("close") + "</button></div>" +
    "<p class='anno-meta'></p>" +
    "<textarea class='anno-note' aria-label='Annotation' " +
    "placeholder='What should change here?'></textarea>" +
    "<div class='anno-actions'><button type='submit' class='anno-save' " +
    "title='Save annotation' aria-label='Save annotation'>" + icon("check") + "</button></div>";

  var panel = el("div", "anno-panel");
  panel.innerHTML =
    "<div class='anno-head'><strong class='anno-title'>Annotations</strong>" +
    "<span class='anno-count'>0</span>" +
    "<button type='button' class='anno-icon anno-copy' title='Copy report' aria-label='Copy report'>" +
    icon("copy") + "</button>" +
    "<button type='button' class='anno-icon is-danger anno-clear' title='Clear annotations' " +
    "aria-label='Clear annotations'>" + icon("trash") + "</button>" +
    "<button type='button' class='anno-icon anno-close' title='Close' aria-label='Close annotations'>" +
    icon("close") + "</button></div><div class='anno-list'></div>";

  document.body.appendChild(box);
  document.body.appendChild(dock);
  document.body.appendChild(composer);
  document.body.appendChild(panel);

  var targetLabel = composer.querySelector(".anno-target");
  var meta = composer.querySelector(".anno-meta");
  var note = composer.querySelector(".anno-note");
  var list = panel.querySelector(".anno-list");
  var count = panel.querySelector(".anno-count");
  var badge = listBtn.querySelector(".anno-badge");

  function el(tag, cls) {
    var node = document.createElement(tag);
    node.className = cls;
    return node;
  }

  function all() {
    try {
      var value = JSON.parse(localStorage.getItem(KEY) || "[]");
      return Array.isArray(value) ? value : [];
    } catch (e) {
      return [];
    }
  }

  function save(list) {
    try {
      localStorage.setItem(KEY, JSON.stringify(list));
    } catch (e) {}
  }

  function page() {
    return location.pathname;
  }

  function selectorFor(node) {
    var parts = [];
    while (node && node.nodeType === 1 && parts.length < 6) {
      if (node.id) {
        parts.unshift("#" + node.id);
        break;
      }
      var part = node.tagName.toLowerCase();
      var cls = (node.getAttribute("class") || "").trim().split(/\s+/).filter(Boolean).slice(0, 2);
      if (cls.length) part += "." + cls.join(".");
      var parent = node.parentElement;
      if (parent) {
        var same = Array.prototype.filter.call(parent.children, function (child) {
          return child.tagName === node.tagName;
        });
        if (same.length > 1) part += ":nth-of-type(" + (same.indexOf(node) + 1) + ")";
      }
      parts.unshift(part);
      node = parent;
    }
    return parts.join(" > ");
  }

  function stylesFor(node) {
    var s = getComputedStyle(node);
    var keys = ["display", "padding", "margin", "gap", "width", "max-width", "min-height",
      "background-color", "color", "border", "border-radius", "font-size", "font-weight",
      "line-height", "box-shadow"];
    return keys
      .map(function (key) {
        return key + ": " + s.getPropertyValue(key);
      })
      .join("; ");
  }

  function describe(node) {
    var rect = node.getBoundingClientRect();
    return {
      page: page(),
      selector: selectorFor(node),
      tag: node.tagName.toLowerCase() + (node.id ? "#" + node.id : ""),
      text: (node.textContent || "").replace(/\s+/g, " ").trim().slice(0, 80),
      rect: [Math.round(rect.left), Math.round(rect.top), Math.round(rect.width), Math.round(rect.height)],
      styles: stylesFor(node),
      viewport: [window.innerWidth, window.innerHeight],
      note: ""
    };
  }

  function report(items) {
    var groups = {};
    items.forEach(function (item) {
      (groups[item.page] = groups[item.page] || []).push(item);
    });
    return Object.keys(groups)
      .map(function (pagePath) {
        var lines = ["### " + pagePath + " · " + groups[pagePath][0].viewport.join("×")];
        groups[pagePath].forEach(function (item, i) {
          lines.push(
            (i + 1) + ". `" + item.selector + "` — " + item.note.replace(/\s+/g, " ") + "\n" +
            "   - " + item.tag + (item.text ? ' "' + item.text + '"' : "") +
            " @ " + item.rect.join(",") + "\n" +
            "   - " + item.styles
          );
        });
        return lines.join("\n");
      })
      .join("\n\n");
  }

  function render() {
    var items = all();
    var mine = items.filter(function (item) {
      return item.page === page();
    });
    badge.textContent = items.length;
    badge.hidden = !items.length;
    count.textContent = mine.length;
    list.innerHTML = mine.length
      ? mine
          .map(function (item) {
            return (
              "<div class='anno-item' data-id='" + item.id + "'>" +
              "<p>" + escapeHtml(item.note) + "</p>" +
              "<code title='" + escapeHtml(item.selector) + "'>" + escapeHtml(item.selector) + "</code>" +
              "<button type='button' class='anno-icon anno-del' title='Delete annotation' " +
              "aria-label='Delete annotation'>" + icon("trash") + "</button></div>"
            );
          })
          .join("")
      : "<p class='anno-empty'>" + icon("annotate") +
        "<span>Click Annotate, then pick any element.</span></p>";
  }

  function escapeHtml(value) {
    return String(value).replace(/[&<>"]/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c];
    });
  }

  function outline(node) {
    if (!node || node.closest(".anno-dock,.anno-panel,.anno-composer")) {
      box.style.display = "none";
      return;
    }
    var rect = node.getBoundingClientRect();
    box.style.display = "block";
    box.style.left = rect.left + "px";
    box.style.top = rect.top + "px";
    box.style.width = rect.width + "px";
    box.style.height = rect.height + "px";
  }

  function setOn(value) {
    on = value;
    document.body.classList.toggle("anno-on", on);
    annotateBtn.setAttribute("aria-pressed", on ? "true" : "false");
    if (!on) {
      box.style.display = "none";
      composer.classList.remove("is-open");
      editing = false;
    }
  }

  annotateBtn.addEventListener("click", function (event) {
    event.preventDefault();
    event.stopPropagation();
    setOn(!on);
    if (on) panel.classList.remove("is-open");
    listBtn.setAttribute("aria-expanded", "false");
    render();
  });

  listBtn.addEventListener("click", function (event) {
    event.preventDefault();
    event.stopPropagation();
    if (on) setOn(false);
    var open = panel.classList.toggle("is-open");
    listBtn.setAttribute("aria-expanded", open ? "true" : "false");
    render();
  });

  document.addEventListener(
    "mousemove",
    function (event) {
      if (on) outline(event.target);
    },
    true
  );

  document.addEventListener(
    "click",
    function (event) {
      if (!on || event.target.closest(".anno-dock,.anno-panel,.anno-composer")) return;
      event.preventDefault();
      event.stopPropagation();
      target = event.target;
      editing = true;
      targetLabel.textContent = selectorFor(target);
      targetLabel.title = selectorFor(target);
      var rect = target.getBoundingClientRect();
      meta.textContent = target.tagName.toLowerCase() + (target.id ? "#" + target.id : "") +
        " @ " + [rect.left, rect.top, rect.width, rect.height].map(Math.round).join(",");
      note.value = "";
      composer.classList.add("is-open");
      note.focus();
    },
    true
  );

  composer.querySelector(".anno-cancel").addEventListener("click", function () {
    composer.classList.remove("is-open");
    editing = false;
  });

  composer.addEventListener("submit", function (event) {
    event.preventDefault();
    var text = note.value.trim();
    if (!target || !text) {
      composer.classList.remove("is-open");
      editing = false;
      return;
    }
    var item = describe(target);
    item.note = text;
    item.id = String(Date.now());
    var items = all();
    items.push(item);
    save(items);
    composer.classList.remove("is-open");
    editing = false;
    render();
  });

  panel.addEventListener("click", function (event) {
    var del = event.target.closest(".anno-del");
    if (del) {
      var id = del.closest(".anno-item").getAttribute("data-id");
      save(
        all().filter(function (item) {
          return item.id !== id;
        })
      );
      render();
      return;
    }
    if (event.target.closest(".anno-clear")) {
      save([]);
      render();
      return;
    }
    if (event.target.closest(".anno-close")) {
      panel.classList.remove("is-open");
      listBtn.setAttribute("aria-expanded", "false");
      return;
    }
    var copy = event.target.closest(".anno-copy");
    if (copy) {
      var items = all();
      if (!items.length) return;
      var text = report(items);
      var done = function () {
        copy.innerHTML = icon("check");
        copy.classList.add("is-done");
        copy.title = "Copied";
        setTimeout(function () {
          copy.innerHTML = icon("copy");
          copy.classList.remove("is-done");
          copy.title = "Copy report";
        }, 1200);
      };
      if (navigator.clipboard) {
        navigator.clipboard.writeText(text).then(done, function () {});
      } else {
        console.log(text);
      }
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.key !== "Escape") return;
    if (editing) {
      composer.classList.remove("is-open");
      editing = false;
    } else if (on) {
      setOn(false);
    } else if (panel.classList.contains("is-open")) {
      panel.classList.remove("is-open");
      listBtn.setAttribute("aria-expanded", "false");
    }
  });

  render();
})();
