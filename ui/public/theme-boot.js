(() => {
  const root = document.documentElement;
  const theme = localStorage.getItem("rustmail-theme");
  const systemDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const dark = theme === "dark" || (theme !== "light" && systemDark);
  root.classList.toggle("dark", dark);
  root.dataset.palette = localStorage.getItem("rustmail-palette") ?? "rustmail";
  root.classList.toggle(
    "rusted",
    localStorage.getItem("rustmail-rusted") === "true",
  );
})();
