import DefaultTheme from "vitepress/theme";
import type { Theme } from "vitepress";
import { theme, useOpenapi } from "vitepress-openapi/client";
import "vitepress-openapi/dist/style.css";
import "./custom.css";

import spec from "../../api.yaml?raw";

export default {
  extends: DefaultTheme,
  async enhanceApp({ app }) {
    useOpenapi({
      spec,
      config: {
        schemaDefaultView: "schema",
        showTryButton: false,
      },
    });
    theme.enhanceApp({ app });
  },
} satisfies Theme;
