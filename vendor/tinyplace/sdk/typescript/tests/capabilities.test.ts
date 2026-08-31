import { describe, expect, it, vi } from "vitest";

import {
  CAPABILITY_PROMPT,
  parseCapabilityReply,
  probeCapabilities,
  repoNameFromRemote,
} from "../src/cli/daemon/capabilities.js";
import { decodeTaskFrame, TINYPLACE_PROTO } from "../src/cli/daemon/protocol.js";
import { DaemonRuntime, type RunTaskFn } from "../src/cli/daemon/runtime.js";
import type { AgentCapabilities } from "../src/cli/daemon/capabilities.js";
import type { RunTaskOptions } from "../src/cli/daemon/providers.js";
import type { TaskFrame } from "../src/cli/daemon/protocol.js";

const CAPABILITY_JSON = JSON.stringify({
  tools: ["Bash", "Read", "Edit"],
  mcpServers: ["tinyplace", "github"],
  accessibleDirs: ["/repo", "/tmp/scratch"],
  summary: "I can read and edit this repo and run shell commands.",
});

function replyingRunTask(reply: string): {
  runTask: RunTaskFn;
  calls: Array<RunTaskOptions>;
} {
  const calls: Array<RunTaskOptions> = [];
  return {
    calls,
    runTask: async (options) => {
      calls.push(options);
      return { provider: options.provider, reply, events: 1 };
    },
  };
}

// ── probeCapabilities ────────────────────────────────────────────────────────

describe("probeCapabilities", () => {
  it("parses the agent's JSON report and fills the daemon-known facts", async () => {
    const { runTask, calls } = replyingRunTask(CAPABILITY_JSON);
    const caps = await probeCapabilities({
      provider: "claude",
      runTask,
      workspace: "/repo",
      env: {},
      providers: ["claude", "codex"],
    });

    expect(caps.tools).toEqual(["Bash", "Read", "Edit"]);
    expect(caps.mcpServers).toEqual(["tinyplace", "github"]);
    expect(caps.summary).toBe(
      "I can read and edit this repo and run shell commands.",
    );
    expect(caps.providers).toEqual(["claude", "codex"]);
    expect(caps.cwd).toBe("/repo");
    // The daemon's own cwd always leads, with the agent's extra dirs merged in.
    expect(caps.accessibleDirs).toEqual(["/repo", "/tmp/scratch"]);

    expect(calls).toHaveLength(1);
    expect(calls[0]?.prompt).toBe(CAPABILITY_PROMPT);
    expect(calls[0]?.provider).toBe("claude");
    expect(calls[0]?.timeoutMs).toBe(60_000);
  });

  it("honors an explicit probe timeout", async () => {
    const { runTask, calls } = replyingRunTask(CAPABILITY_JSON);
    await probeCapabilities({
      provider: "codex",
      runTask,
      workspace: "/repo",
      env: {},
      providers: ["codex"],
      timeoutMs: 5_000,
    });
    expect(calls[0]?.timeoutMs).toBe(5_000);
  });

  it("digs the JSON object out of prose + a markdown fence", async () => {
    const { runTask } = replyingRunTask(
      `Sure! Here you go:\n\n\`\`\`json\n${CAPABILITY_JSON}\n\`\`\`\nHope that helps.`,
    );
    const caps = await probeCapabilities({
      provider: "claude",
      runTask,
      workspace: "/repo",
      env: {},
      providers: ["claude"],
    });
    expect(caps.tools).toEqual(["Bash", "Read", "Edit"]);
    expect(caps.mcpServers).toEqual(["tinyplace", "github"]);
  });

  it("falls back to summary=raw reply when the agent answers with prose", async () => {
    const { runTask } = replyingRunTask("  I can edit files and run tests.  ");
    const caps = await probeCapabilities({
      provider: "claude",
      runTask,
      workspace: "/repo",
      env: {},
      providers: ["claude"],
    });
    expect(caps.summary).toBe("I can edit files and run tests.");
    expect(caps.tools).toEqual([]);
    expect(caps.mcpServers).toEqual([]);
    expect(caps.accessibleDirs).toEqual(["/repo"]);
  });

  it("never throws — a failed provider run degrades to the cheap facts", async () => {
    const caps = await probeCapabilities({
      provider: "claude",
      runTask: async () => {
        throw new Error("claude not found on PATH");
      },
      workspace: "/repo",
      env: {},
      providers: ["claude"],
    });
    expect(caps).toMatchObject({
      cwd: "/repo",
      accessibleDirs: ["/repo"],
      providers: ["claude"],
      tools: [],
      mcpServers: [],
    });
    expect(caps.summary).toBeUndefined();
  });

  it("reports the real repo's project + branch from git", async () => {
    const { runTask } = replyingRunTask(CAPABILITY_JSON);
    const caps = await probeCapabilities({
      provider: "claude",
      runTask,
      workspace: process.cwd(),
      env: {},
      providers: ["claude"],
    });
    expect(caps.project).toBe("tiny.place");
    expect(caps.branch).toBeTruthy();
  });
});

describe("parseCapabilityReply", () => {
  it("ignores non-string and blank entries in the arrays", () => {
    const parsed = parseCapabilityReply(
      JSON.stringify({
        tools: ["Bash", 7, "", "  ", "Bash", "Read"],
        mcpServers: "not-an-array",
        accessibleDirs: null,
      }),
    );
    expect(parsed.tools).toEqual(["Bash", "Read"]);
    expect(parsed.mcpServers).toEqual([]);
    expect(parsed.accessibleDirs).toEqual([]);
    expect(parsed.summary).toBeUndefined();
  });

  it("treats brace-balanced non-JSON as prose rather than throwing", () => {
    const parsed = parseCapabilityReply("I can run {bash, git} here.");
    expect(parsed.summary).toBe("I can run {bash, git} here.");
    expect(parsed.tools).toEqual([]);
  });
});

describe("repoNameFromRemote", () => {
  it("extracts the repo name from ssh, https, and path remotes", () => {
    expect(repoNameFromRemote("git@github.com:tinyhumansai/tiny.place.git")).toBe(
      "tiny.place",
    );
    expect(repoNameFromRemote("https://github.com/org/repo.git\n")).toBe("repo");
    expect(repoNameFromRemote("/srv/git/repo/")).toBe("repo");
  });

  it("drops a query string / fragment so tokens never leak into the name", () => {
    expect(repoNameFromRemote("https://github.com/org/repo.git?token=secret")).toBe("repo");
    expect(repoNameFromRemote("https://github.com/org/repo.git#frag")).toBe("repo");
    expect(repoNameFromRemote("https://user:pass@github.com/org/repo.git?ref=x")).toBe("repo");
  });
});

// ── the capabilities frame end-to-end ────────────────────────────────────────

interface Sent {
  to: string;
  frame: TaskFrame | undefined;
}

function collector(): {
  sent: Array<Sent>;
  send: (to: string, body: string) => Promise<void>;
} {
  const sent: Array<Sent> = [];
  return {
    sent,
    send: async (to, body) => {
      sent.push({ to, frame: decodeTaskFrame(body) });
    },
  };
}

const baseDeps = {
  providers: ["claude"] as const,
  defaultProvider: "claude" as const,
  workspace: "/repo",
  env: {},
  taskTimeoutMs: 1_000,
  concurrency: 2,
  now: (): number => 0,
};

function capabilitiesFrame(taskId: string, correlationId?: string): TaskFrame {
  return {
    proto: TINYPLACE_PROTO,
    kind: "capabilities",
    taskId,
    text: "",
    ts: "now",
    ...(correlationId ? { correlationId } : {}),
  };
}

describe("DaemonRuntime capabilities frame", () => {
  it("answers a capabilities query with the probed JSON, echoing correlationId", async () => {
    const { sent, send } = collector();
    const { runTask } = replyingRunTask(CAPABILITY_JSON);
    const runtime = new DaemonRuntime({ ...baseDeps, send, runTask });

    runtime.handleMessage(
      "peerA",
      { text: "" },
      capabilitiesFrame("q1", "cyc-1/q1/n"),
    );
    await runtime.idle();

    expect(sent).toHaveLength(1);
    const frame = sent[0]?.frame;
    expect(frame?.kind).toBe("capabilities_result");
    expect(frame?.taskId).toBe("q1");
    expect(frame?.correlationId).toBe("cyc-1/q1/n");
    expect(frame?.harness).toBe("claude");

    const caps = JSON.parse(frame?.text ?? "{}") as AgentCapabilities;
    expect(caps.tools).toEqual(["Bash", "Read", "Edit"]);
    expect(caps.mcpServers).toEqual(["tinyplace", "github"]);
    expect(caps.providers).toEqual(["claude"]);
    expect(caps.cwd).toBe("/repo");
  });

  it("probes the LLM once and serves later queries from cache", async () => {
    const { sent, send } = collector();
    const runTask = vi.fn<RunTaskFn>(async (options) => ({
      provider: options.provider,
      reply: CAPABILITY_JSON,
      events: 1,
    }));
    const runtime = new DaemonRuntime({ ...baseDeps, send, runTask });

    runtime.handleMessage("peerA", { text: "" }, capabilitiesFrame("q1"));
    runtime.handleMessage("peerB", { text: "" }, capabilitiesFrame("q2"));
    await runtime.idle();
    runtime.handleMessage("peerA", { text: "" }, capabilitiesFrame("q3"));
    await runtime.idle();

    expect(runTask).toHaveBeenCalledTimes(1);
    expect(sent).toHaveLength(3);
    for (const entry of sent) {
      expect(entry.frame?.kind).toBe("capabilities_result");
      const caps = JSON.parse(entry.frame?.text ?? "{}") as AgentCapabilities;
      expect(caps.tools).toEqual(["Bash", "Read", "Edit"]);
    }
  });

  it("still answers when the probe run fails", async () => {
    const { sent, send } = collector();
    const runTask: RunTaskFn = async () => {
      throw new Error("provider exploded");
    };
    const runtime = new DaemonRuntime({ ...baseDeps, send, runTask });

    runtime.handleMessage("peerA", { text: "" }, capabilitiesFrame("q1"));
    await runtime.idle();

    const caps = JSON.parse(sent[0]?.frame?.text ?? "{}") as AgentCapabilities;
    expect(sent[0]?.frame?.kind).toBe("capabilities_result");
    expect(caps.tools).toEqual([]);
    expect(caps.providers).toEqual(["claude"]);
  });

  it("aborts the in-flight probe on shutdown (bounded lifecycle)", async () => {
    const { sent, send } = collector();
    let sawAbort = false;
    let probeStarted = false;
    // A probe run that only settles when its signal aborts — so the daemon must
    // pass a signal through and cancel it on shutdown, or this hangs forever.
    const runTask: RunTaskFn = (options) =>
      new Promise((resolve) => {
        probeStarted = true;
        const onAbort = (): void => {
          sawAbort = true;
          resolve({ provider: options.provider, reply: "", events: 0 });
        };
        if (options.signal?.aborted) onAbort();
        else options.signal?.addEventListener("abort", onAbort);
      });
    const runtime = new DaemonRuntime({ ...baseDeps, send, runTask });

    runtime.handleMessage("peerA", { text: "" }, capabilitiesFrame("q1"));
    // Wait until the probe has actually reached runTask (past the git-facts step),
    // so the shutdown abort lands on a listening signal rather than racing it.
    await vi.waitFor(() => expect(probeStarted).toBe(true));
    runtime.shutdown();
    await runtime.idle();

    expect(sawAbort).toBe(true);
    // An aborted probe degrades to the cheap facts, so the query is still answered.
    const caps = JSON.parse(sent[0]?.frame?.text ?? "{}") as AgentCapabilities;
    expect(sent[0]?.frame?.kind).toBe("capabilities_result");
    expect(caps.tools).toEqual([]);
  });

  it("ignores a capabilities_result reply (no re-answer, no probe)", async () => {
    const { sent, send } = collector();
    const runTask = vi.fn<RunTaskFn>(async (options) => ({
      provider: options.provider,
      reply: CAPABILITY_JSON,
      events: 1,
    }));
    const runtime = new DaemonRuntime({ ...baseDeps, send, runTask });

    // A response frame (what another daemon emits) must not re-trigger a probe or
    // a reply — else two daemons ping-pong capability JSON forever.
    runtime.handleMessage("peerA", { text: CAPABILITY_JSON }, {
      ...capabilitiesFrame("q1"),
      kind: "capabilities_result",
    });
    await runtime.idle();

    expect(sent).toHaveLength(0);
    expect(runTask).not.toHaveBeenCalled();
  });

  it("probes on the daemon's configured runner options", async () => {
    const { sent, send } = collector();
    let seen: RunTaskOptions | undefined;
    const runTask = vi.fn<RunTaskFn>(async (options) => {
      seen = options;
      return { provider: options.provider, reply: CAPABILITY_JSON, events: 1 };
    });
    const runtime = new DaemonRuntime({
      ...baseDeps,
      send,
      runTask,
      model: "claude-sonnet-5",
      skipPermissions: true,
    });

    runtime.handleMessage("peerA", { text: "" }, capabilitiesFrame("q1"));
    await runtime.idle();

    expect(seen?.model).toBe("claude-sonnet-5");
    expect(seen?.skipPermissions).toBe(true);
    expect(sent).toHaveLength(1);
  });
});
