// Cozy Rally site: title-screen behaviour (biome attract loop, game-style menu selection,
// platform detection, latest release version). Everything works without this file.
(() => {
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

  // ---- attract loop: cross-fade through the biomes, "Now showing" follows
  const scenes = [...document.querySelectorAll(".scene")];
  const nowName = document.querySelector("[data-now]");
  let cur = 0;
  const load = (img) => {
    if (img.dataset.src) { img.src = img.dataset.src; img.removeAttribute("data-src"); }
  };
  const show = (i) => {
    const next = scenes[i];
    load(next);
    load(scenes[(i + 1) % scenes.length]);
    const go = () => {
      scenes[cur].classList.remove("is-on");
      // restart the slow drift on the incoming scene
      next.classList.remove("is-on"); void next.offsetWidth; next.classList.add("is-on");
      cur = i;
      nowName.classList.add("is-swap");
      setTimeout(() => { nowName.textContent = next.dataset.name; nowName.classList.remove("is-swap"); }, 380);
    };
    next.complete && next.naturalWidth ? go() : next.addEventListener("load", go, { once: true });
  };
  if (scenes.length > 1 && !reduced) {
    load(scenes[1]);
    let timer = setInterval(() => show((cur + 1) % scenes.length), 7000);
    document.addEventListener("visibilitychange", () => {
      clearInterval(timer);
      if (!document.hidden) timer = setInterval(() => show((cur + 1) % scenes.length), 7000);
    });
  }

  // ---- menu: one selected item like the game; arrows move, Enter follows the link
  const items = [...document.querySelectorAll(".menu .mbtn")];
  const select = (el) => items.forEach((b) => b.classList.toggle("is-selected", b === el));
  const ua = navigator.userAgent;
  const android = /Android/i.test(ua);
  const preferred = items.find((b) => b.dataset.platform === (android ? "android" : "windows")) || items[0];
  if (android) preferred.parentElement.prepend(preferred); // phone visitors see their download first
  const ordered = [...document.querySelectorAll(".menu .mbtn")];
  select(preferred);
  ordered.forEach((b) => {
    b.addEventListener("mouseenter", () => select(b));
    b.addEventListener("focus", () => select(b));
  });
  document.querySelector(".menu").addEventListener("mouseleave", () => {
    if (!ordered.includes(document.activeElement)) select(preferred);
  });
  document.addEventListener("keydown", (e) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const active = document.activeElement;
    const inMenu = ordered.includes(active);
    // only take over the arrows on the title screen, never while reading further down
    if (!inMenu && (active !== document.body || window.scrollY > innerHeight * 0.5)) return;
    e.preventDefault();
    const sel = ordered.findIndex((b) => b.classList.contains("is-selected"));
    const n = (sel + (e.key === "ArrowDown" ? 1 : -1) + ordered.length) % ordered.length;
    ordered[n].focus({ preventScroll: true });
  });

  // ---- latest release: version and download sizes from GitHub (static text stays if offline)
  fetch("https://api.github.com/repos/Pullolo/cozy-rally/releases/latest", { headers: { Accept: "application/vnd.github+json" } })
    .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
    .then((rel) => {
      const ver = document.querySelector("[data-version]");
      if (ver && rel.tag_name) ver.textContent = `Latest: ${rel.tag_name}`;
      const mb = (b) => `${(b / 1048576).toFixed(1)} MB`;
      document.querySelectorAll("[data-asset]").forEach((el) => {
        const a = (rel.assets || []).find((x) => x.name === el.dataset.asset);
        if (a) el.textContent = `${el.textContent} · ${mb(a.size)}`;
      });
    })
    .catch(() => {});
})();
