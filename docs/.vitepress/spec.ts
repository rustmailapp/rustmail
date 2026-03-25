import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { parseSpec } from "vitepress-openapi";

const raw = readFileSync(resolve(__dirname, "../api.yaml"), "utf-8");

export const parsedSpec = parseSpec(raw);
