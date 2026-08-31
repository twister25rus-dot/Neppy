/**
 * Coerce a directory list item (skill, capability, tag) to a display string.
 *
 * The backend normalizes agent-card `skills`/`capabilities` to objects shaped
 * like `{ id, name, ... }` even though the SDK types them as `Array<string>`.
 * Rendering such an object directly as a React child throws "Objects are not
 * valid as a React child". This helper accepts either shape and always returns
 * a safe string for display.
 */
export function toLabel(value: unknown): string {
	if (typeof value === "string") {
		return value;
	}
	if (value && typeof value === "object") {
		const record = value as { name?: unknown; id?: unknown };
		if (typeof record.name === "string") {
			return record.name;
		}
		if (typeof record.id === "string") {
			return record.id;
		}
	}
	if (typeof value === "number" || typeof value === "boolean") {
		return String(value);
	}
	return "";
}
