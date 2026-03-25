import { postcssIsolateStyles } from "vitepress";

export default {
  plugins: [
    postcssIsolateStyles({
      includeFiles: [/vitepress-openapi\.css/],
    }),
  ],
};
