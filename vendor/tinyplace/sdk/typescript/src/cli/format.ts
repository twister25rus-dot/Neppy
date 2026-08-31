import { boolFlag, stringFlag } from "./args.js";
import type { Flags, JsonObject, OutputFormat } from "./types.js";

export function resolveFormat(flags: Flags): OutputFormat {
  if (boolFlag(flags, "md")) {
    return "md";
  }
  if (boolFlag(flags, "json")) {
    return "json";
  }
  const format = stringFlag(flags, "format");
  return format === "md" || format === "markdown" ? "md" : "json";
}

export function formatResult(value: unknown, format: OutputFormat, raw: boolean): string {
  const redacted = redactSecrets(value);
  const prepared = raw ? redacted : slim(redacted);
  if (format === "md") {
    return `${renderMarkdown(prepared)}\n`;
  }
  return `${JSON.stringify(prepared, null, 2)}\n`;
}

const NOISE_KEYS = new Set(["signature", "signerPublicKey"]);

export function slim(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(slim);
  }
  if (value && typeof value === "object") {
    const out: JsonObject = {};
    for (const [key, child] of Object.entries(value)) {
      if (NOISE_KEYS.has(key)) {
        continue;
      }
      const slimmed = slim(child);
      if (isEmptyValue(slimmed)) {
        continue;
      }
      out[key] = slimmed;
    }
    return out;
  }
  return value;
}

function isEmptyValue(value: unknown): boolean {
  if (value === null || value === undefined || value === "") {
    return true;
  }
  if (Array.isArray(value)) {
    return value.length === 0;
  }
  if (typeof value === "object") {
    return Object.keys(value).length === 0;
  }
  return false;
}

export function renderMarkdown(value: unknown, indent = ""): string {
  if (value === null || value === undefined) {
    return `${indent}_null_`;
  }
  if (Array.isArray(value)) {
    if (value.length === 0) {
      return `${indent}_(empty)_`;
    }
    return value
      .map((item) =>
        item && typeof item === "object"
          ? `${indent}-\n${renderMarkdown(item, `${indent}  `)}`
          : `${indent}- ${renderScalar(item)}`,
      )
      .join("\n");
  }
  if (typeof value === "object") {
    const entries = Object.entries(value);
    if (entries.length === 0) {
      return `${indent}_(empty)_`;
    }
    return entries
      .map(([key, child]) =>
        child && typeof child === "object"
          ? `${indent}- **${key}**:\n${renderMarkdown(child, `${indent}  `)}`
          : `${indent}- **${key}**: ${renderScalar(child)}`,
      )
      .join("\n");
  }
  return `${indent}${renderScalar(value)}`;
}

function renderScalar(value: unknown): string {
  return typeof value === "string" ? value : String(value);
}

export function redactSecrets(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => redactSecrets(item));
  }
  if (!value || typeof value !== "object") {
    return value;
  }
  const out: JsonObject = {};
  for (const [key, child] of Object.entries(value)) {
    out[key] = isSecretKeyName(key) ? "[redacted]" : redactSecrets(child);
  }
  return out;
}

function isSecretKeyName(key: string): boolean {
  const normalized = key.toLowerCase().replace(/[^a-z0-9]/g, "");
  return normalized.includes("secret") || normalized.includes("privatekey");
}
