import { usePaths } from "vitepress-openapi";
import { parsedSpec } from "../.vitepress/spec";

export default {
  paths() {
    return usePaths({ spec: parsedSpec })
      .getPathsByVerbs()
      .filter(({ operationId }) => operationId)
      .map(({ operationId, summary }) => ({
        params: {
          operationId,
          pageTitle: `${summary} - RustMail API`,
        },
      }));
  },
};
