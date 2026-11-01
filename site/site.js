// InstaFind — بحث سريع · small progressive enhancements. The page works fully without this file.
(function () {
  "use strict";
  var root = document.documentElement;
  var reduce = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  // Theme toggle (system by default; a choice is remembered on this device only).
  var btn = document.querySelector(".theme-btn");
  if (btn) {
    btn.addEventListener("click", function () {
      var dark = root.getAttribute("data-theme")
        ? root.getAttribute("data-theme") === "dark"
        : window.matchMedia("(prefers-color-scheme: dark)").matches;
      var next = dark ? "light" : "dark";
      root.setAttribute("data-theme", next);
      try { localStorage.setItem("theme", next); } catch (e) {}
    });
  }

  // Fade sections in and grow the speed bars when they scroll into view.
  var targets = document.querySelectorAll(".fade, .reveal");
  if ("IntersectionObserver" in window && !reduce) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); }
      });
    }, { rootMargin: "0px 0px -8% 0px", threshold: 0.08 });
    targets.forEach(function (t) { io.observe(t); });
  } else {
    targets.forEach(function (t) { t.classList.add("in"); });
  }

  // Typing demo in the Arabic-search tile: types each query, then highlights its row.
  var demo = document.querySelector("[data-typing]");
  if (demo) {
    var out = demo.querySelector(".typed");
    var rows = Array.prototype.slice.call(demo.querySelectorAll("li[data-q]"));
    if (out && rows.length && !reduce) {
      var i = 0;
      var step = function () {
        var row = rows[i % rows.length];
        var q = row.getAttribute("data-q");
        rows.forEach(function (r) { r.classList.remove("on"); });
        out.setAttribute("dir", /[A-Za-z0-9]/.test(q.charAt(0)) ? "ltr" : "rtl");
        var n = 0;
        out.textContent = "";
        var t = setInterval(function () {
          n++;
          out.textContent = q.slice(0, n);
          if (n >= q.length) {
            clearInterval(t);
            row.classList.add("on");
            i++;
            setTimeout(step, 1900);
          }
        }, 110);
      };
      step();
    }
  }

  // Copy buttons on install commands.
  document.querySelectorAll("pre").forEach(function (pre) {
    var code = pre.querySelector("code");
    var b = pre.querySelector(".copy");
    if (!code || !b || !navigator.clipboard) { if (b) b.remove(); return; }
    var label = b.textContent;
    b.addEventListener("click", function () {
      navigator.clipboard.writeText(code.textContent).then(function () {
        b.textContent = b.getAttribute("data-done") || "✓";
        setTimeout(function () { b.textContent = label; }, 1600);
      });
    });
  });
})();
