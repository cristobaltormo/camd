(function () {
  var ACCENTS = {
    blue: { dark: "#4a86f7", light: "#2563eb", ink: "#ffffff" },
    green: { dark: "#2fb67c", light: "#16a065", ink: "#ffffff" },
    orange: { dark: "#ec8a3c", light: "#d9701f", ink: "#ffffff" },
    graphite: { dark: "#e6e6e9", light: "#232327", ink: null },
  };
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
    root.style.setProperty("--accent", accent[resolved]);
    root.style.setProperty("--on-accent", accent.ink || (resolved === "dark" ? "#111113" : "#ffffff"));
    var meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.setAttribute("content", resolved === "dark" ? "#0c0c0d" : "#f4f4f5");
  }

  function set(key, value) {
    try { localStorage.setItem(key, value); } catch (_) {}
    apply();
  }

  dark.addEventListener("change", apply);
  window.camTheme = { apply: apply, set: set, accents: ACCENTS };
  apply();
})();
