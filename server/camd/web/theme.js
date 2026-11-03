(function () {
  var ACCENTS = { blue: "#4c82f7", green: "#2fb67c", violet: "#8b5cf6", orange: "#e8873a", pink: "#e0489b" };
  var dark = window.matchMedia("(prefers-color-scheme: dark)");

  function read(key, fallback) {
    try { return localStorage.getItem(key) || fallback; } catch (_) { return fallback; }
  }

  function apply() {
    var pref = read("cam.theme", "auto");
    var resolved = pref === "auto" ? (dark.matches ? "dark" : "light") : pref;
    var root = document.documentElement;
    root.dataset.theme = resolved;
    var accent = ACCENTS[read("cam.accent", "blue")] || ACCENTS.blue;
    root.style.setProperty("--accent", accent);
    var meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.setAttribute("content", resolved === "dark" ? "#0b0c0f" : "#f6f6f7");
  }

  function set(key, value) {
    try { localStorage.setItem(key, value); } catch (_) {}
    apply();
  }

  dark.addEventListener("change", apply);
  window.camTheme = { apply: apply, set: set, accents: ACCENTS };
  apply();
})();
