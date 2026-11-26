(() => {
  const retiredPalettes = {
    copper: "tide",
    classic: "graphite",
    ember: "graphite",
    dawn: "rustmail",
  };
  const root = document.documentElement;
  const theme = localStorage.getItem("rustmail-theme");
  const systemDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const dark = theme === "dark" || (theme !== "light" && systemDark);
  root.classList.toggle("dark", dark);
  const palette = localStorage.getItem("rustmail-palette") ?? "rustmail";
  root.dataset.palette = Object.hasOwn(retiredPalettes, palette)
    ? retiredPalettes[palette]
    : palette;
  root.classList.toggle(
    "rusted",
    localStorage.getItem("rustmail-rusted") === "true",
  );
})();
