import { describe, expect, it } from "vitest";

import { toLabel } from "./labels";

/**
 * Regression guard for the runtime crash "Objects are not valid as a React
 * child (found: object with keys {id, name})".
 *
 * The Open Directory backend returns agent-card `skills`/`capabilities` as
 * `{ id, name, ... }` objects even though the SDK types them as `Array<string>`.
 * Components render these list items as React children, so every item must
 * coerce to a string — never leak an object into JSX.
 */
describe("toLabel — directory list item normalization", () => {
	it("passes through plain strings", () => {
		expect(toLabel("csv-analysis")).toBe("csv-analysis");
	});

	it("renders the backend's { id, name } skill object as its name", () => {
		expect(toLabel({ id: "csv-analysis", name: "CSV Analysis" })).toBe(
			"CSV Analysis"
		);
	});

	it("falls back to id when name is absent", () => {
		expect(toLabel({ id: "echo" })).toBe("echo");
	});

	it("never returns a non-string for any supported shape", () => {
		const inputs: Array<unknown> = [
			"plain",
			{ id: "a", name: "Alpha" },
			{ id: "b" },
			{ name: "Gamma" },
			null,
			undefined,
		];
		for (const value of inputs) {
			expect(typeof toLabel(value)).toBe("string");
		}
	});

	it("maps a backend agent card's skills array to safe strings (no objects leak to JSX)", () => {
		// Shape returned by GET /directory/agents/{id} (see backend AgentSkill).
		const agentCard = {
			agentId: "AbC123",
			name: "csv-bot",
			skills: [
				{ id: "csv-analysis", name: "CSV Analysis" },
				{ id: "echo", name: "Echo" },
			],
			tags: ["e2e", "data"],
		};

		const skillLabels = (agentCard.skills ?? []).map(toLabel);
		expect(skillLabels).toEqual(["CSV Analysis", "Echo"]);
		expect(skillLabels.every((label) => typeof label === "string")).toBe(true);

		const tagLabels = (agentCard.tags ?? []).map(toLabel);
		expect(tagLabels).toEqual(["e2e", "data"]);
	});
});
