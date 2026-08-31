/**
 * Capability discovery: what can this daemon's agent actually *do* here?
 *
 * An orchestrator picking a lane needs more than the provider name — it needs the
 * repo, the branch, the directories the agent can touch, and the tools/MCP servers
 * it can reach. Config-file heuristics get that wrong (a `.mcp.json` on disk says
 * nothing about which servers actually connected, and tool availability depends on
 * the harness's runtime permissions), so we ask the agent itself: a short, strict
 * prompt run through the ordinary provider path, answered as JSON.
 *
 * The daemon still owns the facts it can establish cheaply and authoritatively —
 * cwd, git project/branch, the detected providers — and those win on merge. The
 * model only supplies what it alone knows: tools, MCP servers, extra accessible
 * dirs, and a prose summary. The probe never throws: a provider that is missing,
 * wedged, or babbling degrades to the cheap facts plus empty arrays.
 */
import { spawn } from "node:child_process";
import { resolve } from "node:path";

import type { RunTaskOptions, RunTaskResult } from "./providers.js";
import type { HarnessProvider } from "../../types/harness.js";

/** What the agent reports it can do, merged with what the daemon knows. */
export interface AgentCapabilities {
  /** The workspace the daemon runs tasks in (absolute). */
  cwd: string;
  /** Directories the agent can read/write — always includes `cwd`. */
  accessibleDirs: Array<string>;
  /** Repo name derived from the git origin remote, when there is one. */
  project?: string;
  /** Current git branch, when the workspace is a repo. */
  branch?: string;
  /** Coding-agent CLIs this daemon can run tasks with. */
  providers: Array<HarnessProvider>;
  /** Tool/command names the agent says it can invoke. */
  tools: Array<string>;
  /** MCP servers/connectors the agent says are available to it. */
  mcpServers: Array<string>;
  /** 1-2 sentences, in the agent's own words, on what it can do here. */
  summary?: string;
}

/**
 * The probe prompt. Deliberately strict about the shape: the reply is parsed by a
 * machine, and a provider that wraps JSON in prose or a markdown fence costs us the
 * structured fields (we fall back to summary-only).
 */
export const CAPABILITY_PROMPT =
  'Report your own capabilities for an orchestrator. Respond with ONLY a JSON object, no prose or markdown, matching {"tools":string[],"mcpServers":string[],"accessibleDirs":string[],"summary":string}: tools=tool/command names you can invoke; mcpServers=MCP servers/connectors available to you; accessibleDirs=absolute dirs you can read/write; summary=1-2 sentences on what you can do here.';

/** A capability probe should answer in seconds; a slow one must not stall a query. */
const DEFAULT_PROBE_TIMEOUT_MS = 60_000;

export interface ProbeCapabilitiesOptions {
  provider: HarnessProvider;
  runTask: (options: RunTaskOptions) => Promise<RunTaskResult>;
  workspace: string;
  env: Record<string, string | undefined>;
  providers: ReadonlyArray<HarnessProvider>;
  timeoutMs?: number;
  // Same runner settings the daemon forwards to real tasks — so the probe runs on
  // the SAME agent that will handle delegated work (not the default model/agent or
  // a permission-blocked run), or the reported capabilities are wrong.
  model?: string;
  agent?: string;
  skipPermissions?: boolean;
  /** Cancels the probe run (the daemon aborts it on shutdown). An aborted probe
   *  throws inside runTask and degrades to the cheap facts like any other fault. */
  signal?: AbortSignal;
}

/** The git-derived facts the daemon establishes without asking the model. */
export interface GitFacts {
  project?: string;
  branch?: string;
}

/**
 * Ask the agent what it can do, and merge its answer over the facts we already
 * know. Never throws — a failed probe yields the cheap facts and empty arrays.
 */
export async function probeCapabilities(
  options: ProbeCapabilitiesOptions,
): Promise<AgentCapabilities> {
  const cwd = resolve(options.workspace);
  const git = await readGitFacts(cwd);
  const base: AgentCapabilities = {
    cwd,
    accessibleDirs: [cwd],
    providers: [...options.providers],
    tools: [],
    mcpServers: [],
    ...(git.project ? { project: git.project } : {}),
    ...(git.branch ? { branch: git.branch } : {}),
  };

  let reply: string;
  try {
    const result = await options.runTask({
      provider: options.provider,
      prompt: CAPABILITY_PROMPT,
      cwd,
      env: options.env,
      timeoutMs: options.timeoutMs ?? DEFAULT_PROBE_TIMEOUT_MS,
      ...(options.model ? { model: options.model } : {}),
      ...(options.agent ? { agent: options.agent } : {}),
      ...(options.skipPermissions ? { skipPermissions: true } : {}),
      ...(options.signal ? { signal: options.signal } : {}),
    });
    reply = result.reply;
  } catch {
    // A missing/wedged provider must not deny the caller the facts we do have.
    return base;
  }

  const reported = parseCapabilityReply(reply);
  return {
    ...base,
    // The daemon's git/cwd/provider facts win: the model can be wrong about where
    // it is, and an orchestrator routing on a hallucinated repo name is worse than
    // one routing on none.
    accessibleDirs: unique([cwd, ...reported.accessibleDirs]),
    tools: reported.tools,
    mcpServers: reported.mcpServers,
    ...(reported.summary ? { summary: reported.summary } : {}),
  };
}

interface ReportedCapabilities {
  accessibleDirs: Array<string>;
  tools: Array<string>;
  mcpServers: Array<string>;
  summary?: string;
}

/**
 * Pull the capability object out of a provider reply. Providers wander off-format
 * (a markdown fence, a sentence of preamble), so we scan for the first balanced
 * `{...}` rather than parsing the whole reply. A reply with no usable JSON is not a
 * failure — it is still the agent describing itself, so it becomes the summary.
 */
export function parseCapabilityReply(reply: string): ReportedCapabilities {
  const json = firstJsonObject(reply);
  if (json) {
    try {
      const parsed = JSON.parse(json) as Record<string, unknown>;
      return {
        tools: stringArray(parsed.tools),
        mcpServers: stringArray(parsed.mcpServers),
        accessibleDirs: stringArray(parsed.accessibleDirs),
        ...(typeof parsed.summary === "string" && parsed.summary.trim()
          ? { summary: parsed.summary.trim() }
          : {}),
      };
    } catch {
      // Brace-balanced but not valid JSON — fall through to the prose fallback.
    }
  }
  const raw = reply.trim();
  return {
    tools: [],
    mcpServers: [],
    accessibleDirs: [],
    ...(raw ? { summary: raw } : {}),
  };
}

/** Scan out the first brace-balanced object, ignoring braces inside strings. */
function firstJsonObject(text: string): string | undefined {
  const start = text.indexOf("{");
  if (start === -1) return undefined;
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let index = start; index < text.length; index += 1) {
    const char = text[index];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (char === "\\") {
        escaped = true;
      } else if (char === '"') {
        inString = false;
      }
      continue;
    }
    if (char === '"') {
      inString = true;
    } else if (char === "{") {
      depth += 1;
    } else if (char === "}") {
      depth -= 1;
      if (depth === 0) {
        return text.slice(start, index + 1);
      }
    }
  }
  return undefined;
}

function stringArray(value: unknown): Array<string> {
  if (!Array.isArray(value)) return [];
  return unique(
    value
      .filter((entry): entry is string => typeof entry === "string")
      .map((entry) => entry.trim())
      .filter(Boolean),
  );
}

function unique(values: ReadonlyArray<string>): Array<string> {
  return [...new Set(values)];
}

/**
 * Project + branch from git, best-effort. Runs `git -C <cwd>` rather than spawning
 * *in* `cwd`, so a workspace that does not exist fails as a non-zero exit instead
 * of a spawn error, and a non-repo simply reports nothing.
 */
export async function readGitFacts(cwd: string): Promise<GitFacts> {
  const [origin, branch] = await Promise.all([
    runGit(["-C", cwd, "remote", "get-url", "origin"]),
    runGit(["-C", cwd, "rev-parse", "--abbrev-ref", "HEAD"]),
  ]);
  const project = origin ? repoNameFromRemote(origin) : undefined;
  return {
    ...(project ? { project } : {}),
    ...(branch ? { branch } : {}),
  };
}

/** `git@host:org/repo.git`, `https://host/org/repo.git`, `/path/to/repo` → `repo`.
 *  Any `?query`/`#fragment` (and thus embedded tokens) is dropped before the
 *  `.git` suffix and final segment are taken, so a name is never polluted. */
export function repoNameFromRemote(remote: string): string | undefined {
  const trimmed = remote
    .trim()
    .replace(/[?#].*$/, "")
    .replace(/\/+$/, "")
    .replace(/\.git$/i, "");
  const last = trimmed.split(/[/:]/).pop();
  return last && last.trim() ? last.trim() : undefined;
}

/** Run a git command, resolving to its trimmed stdout or `undefined` on any fault. */
function runGit(args: Array<string>): Promise<string | undefined> {
  return new Promise<string | undefined>((resolveGit) => {
    let child;
    try {
      child = spawn("git", args, { stdio: ["ignore", "pipe", "ignore"] });
    } catch {
      resolveGit(undefined);
      return;
    }
    let out = "";
    child.stdout?.on("data", (chunk: Buffer) => {
      out += chunk.toString();
    });
    child.on("error", () => resolveGit(undefined));
    child.on("close", (code: number | null) => {
      const value = out.trim();
      resolveGit(code === 0 && value ? value : undefined);
    });
  });
}
