export interface Alias {
	name: string;
	desc?: string;
	type?: string; // simple type, e.g. "number | { percent: number }"
	variants?: string[]; // union literal values, e.g. ['"linear"', '"ease_in"', ...]
}

export interface Field {
	name: string;
	optional: boolean;
	type: string;
	desc?: string;
}

export interface Param {
	name: string;
	optional: boolean;
	type: string;
	desc?: string;
}

export interface Func {
	name: string;
	fullName: string; // e.g. "lewdware.spawn_image_popup" or "Window:close"
	className?: string; // set for class methods
	sep?: '.' | ':';
	params: Param[];
	returnType?: string;
	desc?: string;
}

export interface Class {
	name: string;
	parent?: string;
	desc?: string;
	fields: Field[];
	methods: Func[];
}

export interface Namespace {
	name: string;
	functions: Func[];
}

export interface ApiDoc {
	aliases: Alias[];
	classes: Class[];
	namespaces: Namespace[];
}

// ---------------------------------------------------------------------------
// Block extraction
// ---------------------------------------------------------------------------

interface Block {
	comments: string[]; // each line stripped of leading `---` and one optional space
	codeLine?: string;
}

function getBlocks(source: string): Block[] {
	const blocks: Block[] = [];
	let comments: string[] = [];

	for (const rawLine of source.split('\n')) {
		const line = rawLine.trim();

		if (line.startsWith('---')) {
			// Strip `---` and one optional leading space
			comments.push(line.slice(3).replace(/^ /, ''));
		} else if (line === '') {
			if (comments.length > 0) {
				blocks.push({ comments: [...comments] });
				comments = [];
			}
		} else {
			if (comments.length > 0) {
				blocks.push({ comments: [...comments], codeLine: line });
				comments = [];
			}
		}
	}

	if (comments.length > 0) blocks.push({ comments });
	return blocks;
}

// Merge continuation lines into the preceding line.
// A continuation line is either:
//   - indented (starts with a space, meaning the `---` was followed by spaces), OR
//   - part of an open bracket group from a previous line (e.g. the closing `}` in
//     a multiline `---@param opts? {\n---   key: type,\n---}` block)
function joinContinuations(lines: string[]): string[] {
	const result: string[] = [];
	let openDepth = 0;

	for (const line of lines) {
		if ((openDepth > 0 || line.startsWith(' ')) && result.length > 0) {
			result[result.length - 1] += ' ' + line.trim();
		} else {
			result.push(line);
		}

		for (const ch of line) {
			if (ch === '{' || ch === '[' || ch === '(') openDepth++;
			else if (ch === '}' || ch === ']' || ch === ')') openDepth = Math.max(0, openDepth - 1);
		}
	}

	return result;
}

// ---------------------------------------------------------------------------
// Type reader — reads a LuaCATS type expression from the start of a string.
// Stops at a space that is NOT followed by `|` (union) or `&` (intersection),
// respecting nesting depth of `{`, `[`, `(`.
// ---------------------------------------------------------------------------

function readType(s: string): { type: string; rest: string } {
	let i = 0;
	let depth = 0;

	while (i < s.length) {
		const ch = s[i];
		if (ch === '{' || ch === '[' || ch === '(') {
			depth++;
			i++;
			continue;
		}
		if (ch === '}' || ch === ']' || ch === ')') {
			depth--;
			i++;
			continue;
		}
		if (depth === 0 && ch === ' ') {
			const ahead = s.slice(i + 1).trimStart();
			if (ahead.startsWith('|') || ahead.startsWith('&')) {
				i++;
				continue;
			}
			break;
		}
		i++;
	}

	return { type: s.slice(0, i).trim(), rest: s.slice(i).trim() };
}

// ---------------------------------------------------------------------------
// Individual annotation parsers
// ---------------------------------------------------------------------------

function parseFieldLine(line: string): Field | null {
	// @field name? type [desc]  OR  @field name type [desc]
	const m = line.match(/^@field\s+(\w+)(\?)?\s*(.*)?$/);
	if (!m) return null;
	const name = m[1];
	const optional = Boolean(m[2]);
	const rest = (m[3] ?? '').trim();
	if (!rest) return { name, optional, type: 'any' };
	const { type, rest: desc } = readType(rest);
	return { name, optional, type, desc: desc || undefined };
}

function parseParamLine(line: string): Param | null {
	// @param name? type [desc]
	const m = line.match(/^@param\s+(\w+)(\?)?\s+(.*)?$/);
	if (!m) return null;
	const name = m[1];
	const optional = Boolean(m[2]);
	const rest = (m[3] ?? '').trim();
	const { type, rest: desc } = readType(rest);
	return { name, optional, type, desc: desc || undefined };
}

// ---------------------------------------------------------------------------
// Block parsers
// ---------------------------------------------------------------------------

function parseClassBlock(block: Block): Class | null {
	const lines = joinContinuations(block.comments);

	let name = '';
	let parent: string | undefined;
	const fields: Field[] = [];
	const descLines: string[] = [];
	let gatheringDesc = false;
	let found = false;

	for (const line of lines) {
		if (line.startsWith('@class ')) {
			const m = line.match(/^@class\s+(\w+)(?:\s*:\s*(\w+))?/);
			if (!m) continue;
			name = m[1];
			parent = m[2];
			found = true;
			gatheringDesc = true;
			continue;
		}

		if (!found) continue;

		if (line.startsWith('@field ')) {
			gatheringDesc = false;
			const f = parseFieldLine(line);
			if (f) fields.push(f);
			continue;
		}

		if (line.startsWith('@')) {
			gatheringDesc = false;
			continue;
		}

		if (gatheringDesc && line !== '') {
			descLines.push(line);
		}
	}

	if (!found) return null;
	return {
		name,
		parent,
		desc: descLines.join(' ').trim() || undefined,
		fields,
		methods: [],
	};
}

function parseAliasBlock(block: Block): Alias | null {
	const lines = joinContinuations(block.comments);

	let name = '';
	let type = '';
	let desc = '';
	const variants: string[] = [];

	for (const line of lines) {
		if (line.startsWith('@alias ')) {
			const m = line.match(/^@alias\s+(\w+)(?:\s+(.+))?$/);
			if (!m) continue;
			name = m[1];
			const rest = (m[2] ?? '').trim();
			if (rest) {
				const parsed = readType(rest);
				type = parsed.type;
				desc = parsed.rest;
			}
			continue;
		}

		if (line.startsWith('| ')) {
			variants.push(line.slice(2).trim());
			continue;
		}

		if (!line.startsWith('@') && name && line !== '') {
			if (!desc) desc = line;
		}
	}

	if (!name) return null;
	return {
		name,
		desc: desc || undefined,
		type: type || undefined,
		variants: variants.length > 0 ? variants : undefined,
	};
}

function parseFuncBlock(block: Block & { codeLine: string }): Func | null {
	const codeLine = block.codeLine;
	const lines = joinContinuations(block.comments);

	// Parse the function declaration
	let className: string | undefined;
	let funcName: string;
	let sep: '.' | ':' | undefined;
	let fullName: string;

	// Pattern 1: function ClassName:method(...)
	const methodMatch = codeLine.match(/^function\s+(\w+)(:)(\w+)\s*\(/);
	if (methodMatch) {
		className = methodMatch[1];
		sep = ':';
		funcName = methodMatch[3];
		fullName = `${className}:${funcName}`;
	} else {
		// Pattern 2: function ns1.ns2.funcName(...) — greedily matches the rightmost dot
		const nsMatch = codeLine.match(/^function\s+((?:\w+\.)*\w+)\.(\w+)\s*\(/);
		if (nsMatch) {
			sep = '.';
			funcName = nsMatch[2];
			fullName = `${nsMatch[1]}.${funcName}`;
		} else {
			return null;
		}
	}

	const params: Param[] = [];
	let returnType: string | undefined;
	const descLines: string[] = [];

	for (const line of lines) {
		if (line.startsWith('@param ')) {
			const p = parseParamLine(line);
			if (p) params.push(p);
			continue;
		}
		if (line.startsWith('@return ')) {
			returnType = line.slice('@return '.length).trim();
			continue;
		}
		if (!line.startsWith('@') && line !== '') {
			descLines.push(line);
		}
	}

	return {
		name: funcName,
		fullName,
		className,
		sep,
		params,
		returnType,
		desc: descLines.join('\n').trim() || undefined,
	};
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

export function parseLuaCATS(source: string): ApiDoc {
	const blocks = getBlocks(source);

	const aliases: Alias[] = [];
	// Use insertion-order map to preserve declaration order
	const classMap = new Map<string, Class>();
	const nsFuncsMap = new Map<string, Func[]>();

	// First pass: register all class names so method blocks can look them up
	for (const block of blocks) {
		for (const line of block.comments) {
			const m = line.match(/^@class\s+(\w+)/);
			if (m && !classMap.has(m[1])) {
				classMap.set(m[1], { name: m[1], fields: [], methods: [] });
			}
		}
	}

	// Second pass: full parse
	for (const block of blocks) {
		const hasClass = block.comments.some((l) => l.startsWith('@class'));
		const hasAlias = block.comments.some((l) => l.startsWith('@alias'));

		if (hasClass) {
			const cls = parseClassBlock(block);
			if (cls) {
				const existing = classMap.get(cls.name);
				if (existing) {
					if (cls.parent) existing.parent = cls.parent;
					if (cls.desc) existing.desc = cls.desc;
					existing.fields.push(...cls.fields);
				}
			}
		} else if (hasAlias) {
			const alias = parseAliasBlock(block);
			if (alias) aliases.push(alias);
		} else if (block.codeLine) {
			const func = parseFuncBlock(block as Block & { codeLine: string });
			if (!func) continue;

			if (func.className && classMap.has(func.className)) {
				classMap.get(func.className)!.methods.push(func);
			} else {
				let nsKey = 'lewdware';
				if (func.fullName.startsWith('lewdware.media.')) nsKey = 'lewdware.media';
				else if (func.fullName.startsWith('lewdware.monitors.')) nsKey = 'lewdware.monitors';

				if (!nsFuncsMap.has(nsKey)) nsFuncsMap.set(nsKey, []);
				nsFuncsMap.get(nsKey)!.push(func);
			}
		}
	}

	const namespaces = (['lewdware', 'lewdware.media', 'lewdware.monitors'] as const)
		.filter((ns) => nsFuncsMap.has(ns))
		.map((ns) => ({ name: ns, functions: nsFuncsMap.get(ns)! }));

	return {
		aliases,
		classes: [...classMap.values()],
		namespaces,
	};
}
